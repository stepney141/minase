"""第2世代の教師の量だけを変えて学習し、全量のネットと半分、4分の1のネットをSTCで比べる。

対局を減らすために、教師ファイルを先頭から数えて選ぶ（全37、19、および9ファイル）。
学習と補正の両方に同じファイルだけを与えるので、教師が少なかった場合を再現する。
GPUの違いが比較に入らないように、全量のネットも作業機で学び直して対照とする。
このネットは50周の候補の基準にも使う。途中検証は省くが、学習の軌跡は変わらない。
候補は全量のネットであり、`H1`は教師を増やすと強くなることを示す。
"""
import json
import math
import os
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
DATA = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-generation-reset'
MINASE = str(DATA / 'bin/minase')
TRAINER = str(DATA / 'bin/minase-trainer')
MATCHES = ROOT / 'data/matches'
GEN = DATA / 'gen-2'
OUTPUT = GEN / 'volume'
BASE = DATA / 'gen-1/fit/nnue.bin'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
BOOKS = [str(path) for path in sorted((GEN / 'teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]
VOLUMES = {'full': 37, 'half': 19, 'quarter': 9}
COMPARISONS = (('half', 'nnue-generation-reset-gen2-volume-half-stc', '20261202700000'),
               ('quarter', 'nnue-generation-reset-gen2-volume-quarter-stc', '20261202800000'))


def low_priority():
    os.nice(19)


def wait_for_previous():
    while subprocess.run(['systemctl', '--user', 'is-active', '--quiet', 'nnue-gen2-best-last']).returncode == 0:
        time.sleep(60)
    lines = (GEN / 'best-last.log').read_text().splitlines()
    if not lines or lines[-1] != 'finished':
        raise SystemExit('最良と最後の比較と幅128の学習が正常に終わっていません')


def train(name, books):
    directory = OUTPUT / name / 'train'
    if (directory / 'summary.json').is_file():
        return
    a = json.loads((GEN / 'train/config.json').read_text())['arguments']
    command = [TRAINER, 'nnue', 'train', '--data', *books, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', '8',
               '--output-dir', str(directory), '--feature-set', a['feature_set'], '--width', str(a['width']),
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', str(a['epochs']),
               '--patience', str(a['patience']), '--device', a['device'], '--lr-schedule', 'cosine:3:0.1']
    if (directory / 'latest.npz').is_file():
        command.extend(['--resume', str(directory)])
    directory.parent.mkdir(parents=True, exist_ok=True)
    with (OUTPUT / name / 'train.log').open('a') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


def fit(name, books):
    train_directory = OUTPUT / name / 'train'
    status = json.loads((train_directory / 'summary.json').read_text())['status']
    if status != 'candidate':
        raise SystemExit(f'{name}の学習結果がcandidateではありません: {status}')
    directory = OUTPUT / name / 'fit'
    if not (directory / 'summary.json').is_file():
        subprocess.run(['rm', '-rf', str(directory)], check=True)
        with (OUTPUT / name / 'fit.log').open('a') as log:
            subprocess.run([TRAINER, 'nnue', 'fit-linear', '--base', str(train_directory / 'nnue.bin'), '--data', *books,
                            '--tau', '10', '20', '40', '80', '--seed', '20261105', '--threads', '12',
                            '--output-dir', str(directory)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)
    d = json.loads((directory / 'summary.json').read_text())['decision']['d']
    if not math.isfinite(d) or d >= 0:
        raise SystemExit(f'{name}の補正のDが負の有限値ではありません: {d}')
    return d


def commit(name, count, d):
    net = OUTPUT / name / 'fit/nnue.bin'
    reference = f'refs/nnue-generation-reset/gen-2-volume-{name}'
    with tempfile.TemporaryDirectory() as temporary:
        environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}

        def git(*args):
            return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                                  capture_output=True, text=True).stdout.strip()

        git('read-tree', PARENT)
        blob = git('hash-object', '-w', str(net))
        git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
        tree = git('write-tree')
        result = git('commit-tree', tree, '-p', PARENT, '-m', f'feat(eval): embed the generation 2 net trained on {count} teacher files',
                     '-m', f'第2世代の教師の先頭{count}ファイルを作業機で学習して補正した比較用のネット。D={d:.6f}。参照gen-2には使わない。',
                     '-m', TRAILER)
        git('update-ref', reference, result)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True, capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit(f'{name}のコミットのネットが補正後のネットと一致しません')
    return result


def stc(name, candidate, baseline, seed):
    directory = MATCHES / name
    if directory.exists():
        raise SystemExit(f'{directory} はすでにあります')
    with (MATCHES / f'{name}.log').open('ab') as log:
        subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', seed,
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + baseline,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
print('== 前の測定を待ちます', flush=True)
wait_for_previous()
commits = {}
for volume, count in VOLUMES.items():
    books = BOOKS[:count]
    print(f'== {volume}（{count}ファイル）の学習', flush=True)
    train(volume, books)
    print(f'== {volume}の補正', flush=True)
    value = fit(volume, books)
    commits[volume] = commit(volume, count, value)
    print(volume, commits[volume], f'D={value:.6f}', flush=True)
for volume, name, seed in COMPARISONS:
    print('== STC', name, flush=True)
    stc(name, commits['full'], commits[volume], seed)
print('finished', flush=True)
