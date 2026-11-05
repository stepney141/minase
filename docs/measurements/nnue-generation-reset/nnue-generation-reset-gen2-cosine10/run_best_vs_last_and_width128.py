"""同じ`cosine:10:0.1`の学習の最良と最後のネットをSTCで比べ、続けて幅128で学習して最良エポックの位置を見る。

エポック2のネットを第2世代と同じ条件で補正して別のコミットにし、エポック10のネットと対局させる。
幅128の学習は構成の手がかりを見るためのもので、対局はしない。
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
BASE = DATA / 'gen-1/fit/nnue.bin'
BEST_NET = GEN / 'train-cosine10/nnue.bin'
FIT = GEN / 'fit-cosine10-best'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
REFERENCE_LAST = 'refs/nnue-generation-reset/gen-2-cosine10'
REFERENCE_BEST = 'refs/nnue-generation-reset/gen-2-cosine10-best'
STC = 'nnue-generation-reset-gen2-cosine10-last-vs-best-stc'
WIDTH128 = GEN / 'train-width128'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
BOOKS = [str(path) for path in sorted((GEN / 'teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]


def low_priority():
    os.nice(19)


def git(*args, environment=None):
    return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                          capture_output=True, text=True).stdout.strip()


def wait_for_previous_stc():
    while subprocess.run(['systemctl', '--user', 'is-active', '--quiet', 'nnue-gen2-cosine10']).returncode == 0:
        time.sleep(60)
    lines = (GEN / 'cosine10.log').read_text().splitlines()
    if not lines or lines[-1] != 'finished':
        raise SystemExit('エポック10のネットのSTCが正常に終わっていません')


def fit_best():
    best = json.loads((GEN / 'train-cosine10/summary.json').read_text())
    if best['best_epoch'] != 2 or best['status'] != 'candidate':
        raise SystemExit(f'最良のネットが想定と違います: {best["best_epoch"]}, {best["status"]}')
    if not (FIT / 'summary.json').is_file():
        subprocess.run(['rm', '-rf', str(FIT)], check=True)
        with (GEN / 'fit-cosine10-best.log').open('a') as log:
            subprocess.run([TRAINER, 'nnue', 'fit-linear', '--base', str(BEST_NET), '--data', *BOOKS,
                            '--tau', '10', '20', '40', '80', '--seed', '20261105', '--threads', '12',
                            '--output-dir', str(FIT)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)
    d = json.loads((FIT / 'summary.json').read_text())['decision']['d']
    if not math.isfinite(d) or d >= 0:
        raise SystemExit(f'補正のDが負の有限値ではありません: {d}')
    return d


def commit_best(d):
    net = FIT / 'nnue.bin'
    with tempfile.TemporaryDirectory() as temporary:
        environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}
        git('read-tree', PARENT, environment=environment)
        blob = git('hash-object', '-w', str(net), environment=environment)
        git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}', environment=environment)
        tree = git('write-tree', environment=environment)
        commit = git('commit-tree', tree, '-p', PARENT, '-m', 'feat(eval): embed the best epoch of the longer schedule',
                     '-m', f'第2世代と同じ教師を`cosine:10:0.1`で学習し、検証損失が最良のエポック2のネットを補正した比較用のネット。D={d:.6f}。参照gen-2には使わない。',
                     '-m', TRAILER, environment=environment)
        git('update-ref', REFERENCE_BEST, commit, environment=environment)
    embedded = subprocess.run(['git', 'show', f'{commit}:{NETWORK}'], cwd=WORKTREE, check=True, capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit('コミットのネットが補正後のネットと一致しません')
    return commit


def stc(baseline):
    candidate = git('rev-parse', '--verify', REFERENCE_LAST + '^{commit}')
    directory = MATCHES / STC
    if directory.exists():
        raise SystemExit(f'{directory} はすでにあります')
    with (MATCHES / f'{STC}.log').open('ab') as log:
        subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', '20261202600000',
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + baseline,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)


def train_width128():
    if (WIDTH128 / 'summary.json').is_file():
        return
    a = json.loads((GEN / 'train/config.json').read_text())['arguments']
    command = [TRAINER, 'nnue', 'train', '--data', *BOOKS, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', '8',
               '--output-dir', str(WIDTH128), '--feature-set', a['feature_set'], '--width', '128',
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', str(a['epochs']),
               '--patience', str(a['patience']), '--device', a['device'],
               '--validation-interval', str(a['validation_interval']), '--lr-schedule', 'cosine:3:0.1']
    if (WIDTH128 / 'latest.npz').is_file():
        command.extend(['--resume', str(WIDTH128)])
    with (GEN / 'train-width128.log').open('a') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
print('== エポック10のSTCを待ちます', flush=True)
wait_for_previous_stc()
print('== エポック2の補正', flush=True)
value = fit_best()
print('== コミット', flush=True)
baseline_commit = commit_best(value)
print(baseline_commit, flush=True)
print('== 最後と最良のSTC', flush=True)
stc(baseline_commit)
print('== 幅128の学習', flush=True)
train_width128()
print('finished', flush=True)
