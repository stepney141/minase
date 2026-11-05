"""第2世代のSTCの終了を待ち、学習率の予定だけを変えて学び直したネットを第2世代と対局で比べる。

同じ教師、初期値、およびシードで`cosine:10:0.1`により学習し、`fit-linear`で補正して別のコミットにする。
予定の総長の効果を測るので、早期終了で止めず10エポックまで学習する（`--epochs 10 --patience 50`）。
学習と補正の出力が残っていれば、その工程から再開する。参照`gen-2`は動かさない。
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
TRAIN = GEN / 'train-cosine10'
FIT = GEN / 'fit-cosine10'
DIAG = GEN / 'diag'
BASE = DATA / 'gen-1/fit/nnue.bin'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
REFERENCE = 'refs/nnue-generation-reset/gen-2-cosine10'
STC = 'nnue-generation-reset-gen2-cosine10-stc'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
BOOKS = [str(path) for path in sorted((GEN / 'teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]


def low_priority():
    os.nice(19)


def wait_for_generation_stc():
    unit = 'nnue-gen2-diag-stc'
    while subprocess.run(['systemctl', '--user', 'is-active', '--quiet', unit]).returncode == 0:
        time.sleep(60)
    lines = (GEN / 'diag-stc.log').read_text().splitlines()
    if not lines or lines[-1] != 'finished':
        raise SystemExit('第2世代の診断とSTCが正常に終わっていません')


def train():
    if (TRAIN / 'summary.json').is_file():
        return
    a = json.loads((GEN / 'train/config.json').read_text())['arguments']
    command = [TRAINER, 'nnue', 'train', '--data', *BOOKS, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', '8',
               '--output-dir', str(TRAIN), '--feature-set', a['feature_set'], '--width', str(a['width']),
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', '10',
               '--patience', '50', '--device', a['device'],
               '--validation-interval', str(a['validation_interval']), '--lr-schedule', 'cosine:10:0.1']
    if (TRAIN / 'latest.npz').is_file():
        command.extend(['--resume', str(TRAIN)])
    with (GEN / 'train-cosine10.log').open('a') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


def fit():
    status = json.loads((TRAIN / 'summary.json').read_text())['status']
    if status != 'candidate':
        raise SystemExit(f'学習結果がcandidateではありません: {status}')
    if not (FIT / 'summary.json').is_file():
        subprocess.run(['rm', '-rf', str(FIT)], check=True)
        with (GEN / 'fit-cosine10.log').open('a') as log:
            subprocess.run([TRAINER, 'nnue', 'fit-linear', '--base', str(TRAIN / 'nnue.bin'), '--data', *BOOKS,
                            '--tau', '10', '20', '40', '80', '--seed', '20261105', '--threads', '12',
                            '--output-dir', str(FIT)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)
    d = json.loads((FIT / 'summary.json').read_text())['decision']['d']
    if not math.isfinite(d) or d >= 0:
        raise SystemExit(f'補正のDが負の有限値ではありません: {d}')
    return d


def diagnose():
    # 補正後のネットは組の量子化前の重みを持たないので、その報告では符号反転の欄だけを読む。
    for name, net in (('removal-cosine10-trained', TRAIN / 'nnue.bin'), ('removal-cosine10', FIT / 'nnue.bin')):
        with (DIAG / f'{name}.log').open('w') as log:
            subprocess.run([TRAINER, 'diag', 'nnue-removal', '--float', str(TRAIN / 'best.npz'), '--net', str(net),
                            '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / f'{name}.json')],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


def commit(d):
    summary = json.loads((TRAIN / 'summary.json').read_text())
    net = FIT / 'nnue.bin'
    with tempfile.TemporaryDirectory() as temporary:
        environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}

        def git(*args):
            return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                                  capture_output=True, text=True).stdout.strip()

        git('read-tree', PARENT)
        blob = git('hash-object', '-w', str(net))
        git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
        tree = git('write-tree')
        result = git('commit-tree', tree, '-p', PARENT, '-m', 'feat(eval): embed the generation 2 net trained on a longer learning rate schedule',
                     '-m', f'第2世代と同じ教師を`cosine:10:0.1`で学習した比較用のネット。最良エポック{summary["best_epoch"]}、D={d:.6f}。参照gen-2には使わない。',
                     '-m', TRAILER)
        git('update-ref', REFERENCE, result)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True, capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit('コミットのネットが補正後のネットと一致しません')
    return result


def stc(candidate):
    directory = MATCHES / STC
    if directory.exists():
        raise SystemExit(f'{directory} はすでにあります')
    with (MATCHES / f'{STC}.log').open('ab') as log:
        subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', '20261202500000',
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + PARENT,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
print('== 第2世代のSTCを待ちます', flush=True)
wait_for_generation_stc()
print('== 学習', flush=True)
train()
print('== 補正', flush=True)
value = fit()
print('== 診断', flush=True)
diagnose()
print('== コミット', flush=True)
candidate_commit = commit(value)
print(candidate_commit, flush=True)
print('== STC', flush=True)
stc(candidate_commit)
print('finished', flush=True)
