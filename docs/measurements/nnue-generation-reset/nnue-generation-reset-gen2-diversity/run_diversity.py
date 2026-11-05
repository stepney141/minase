"""同じ局面数で対局数だけが違う2つの訓練集合から学び、対局で比べる。

第2世代の教師（全37本）を`--train-subsample`で半分に間引く。
`games:2:0`は対局の半分の全局面、`positions:2:0`は全対局の局面の半分である。
局面数、更新回数、検証集合、教師の勝率尺度、観測マスク、および初期値は2つで揃う。
補正にはどちらも全37本を与える。
候補は`positions:2:0`のネットであり、`H1`は局面数が同じなら多くの対局から採るほうが強いことを示す。
"""
import json
import math
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
DATA = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-generation-reset'
MINASE = str(DATA / 'bin/minase')
TRAINER = str(DATA / 'bin/minase-trainer')
MATCHES = ROOT / 'data/matches'
GEN = DATA / 'gen-2'
OUTPUT = GEN / 'diversity'
BASE = DATA / 'gen-1/fit/nnue.bin'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
BOOKS = [str(path) for path in sorted((GEN / 'teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]
ARMS = {'games-half': 'games:2:0', 'positions-half': 'positions:2:0'}
STC = 'nnue-generation-reset-gen2-diversity-stc'


def low_priority():
    os.nice(19)


def train(name, subsample):
    directory = OUTPUT / name / 'train'
    if (directory / 'summary.json').is_file():
        return
    a = json.loads((GEN / 'train/config.json').read_text())['arguments']
    command = [TRAINER, 'nnue', 'train', '--data', *BOOKS, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', '8',
               '--output-dir', str(directory), '--feature-set', a['feature_set'], '--width', str(a['width']),
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', str(a['epochs']),
               '--patience', str(a['patience']), '--device', a['device'], '--lr-schedule', 'cosine:3:0.1',
               '--train-subsample', subsample]
    if (directory / 'latest.npz').is_file():
        command.extend(['--resume', str(directory)])
    directory.parent.mkdir(parents=True, exist_ok=True)
    with (OUTPUT / name / 'train.log').open('a') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


def fit(name):
    train_directory = OUTPUT / name / 'train'
    status = json.loads((train_directory / 'summary.json').read_text())['status']
    if status != 'candidate':
        raise SystemExit(f'{name}の学習結果がcandidateではありません: {status}')
    directory = OUTPUT / name / 'fit'
    if not (directory / 'summary.json').is_file():
        subprocess.run(['rm', '-rf', str(directory)], check=True)
        with (OUTPUT / name / 'fit.log').open('a') as log:
            subprocess.run([TRAINER, 'nnue', 'fit-linear', '--base', str(train_directory / 'nnue.bin'), '--data', *BOOKS,
                            '--tau', '10', '20', '40', '80', '--seed', '20261105', '--threads', '12',
                            '--output-dir', str(directory)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)
    d = json.loads((directory / 'summary.json').read_text())['decision']['d']
    if not math.isfinite(d) or d >= 0:
        raise SystemExit(f'{name}の補正のDが負の有限値ではありません: {d}')
    return d


def commit(name, subsample, d):
    net = OUTPUT / name / 'fit/nnue.bin'
    reference = f'refs/nnue-generation-reset/gen-2-{name}'
    with tempfile.TemporaryDirectory() as temporary:
        environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}

        def git(*args):
            return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                                  capture_output=True, text=True).stdout.strip()

        git('read-tree', PARENT)
        blob = git('hash-object', '-w', str(net))
        git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
        tree = git('write-tree')
        result = git('commit-tree', tree, '-p', PARENT,
                     '-m', f'feat(eval): embed the generation 2 net trained on {subsample}',
                     '-m', f'第2世代の教師を`--train-subsample {subsample}`で間引いて学習し、全37本で補正した比較用のネット。D={d:.6f}。参照gen-2には使わない。',
                     '-m', TRAILER)
        git('update-ref', reference, result)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True, capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit(f'{name}のコミットのネットが補正後のネットと一致しません')
    return result


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
commits = {}
for name, subsample in ARMS.items():
    print(f'== {name}（{subsample}）の学習', flush=True)
    train(name, subsample)
    print(f'== {name}の補正', flush=True)
    value = fit(name)
    commits[name] = commit(name, subsample, value)
    print(name, commits[name], f'D={value:.6f}', flush=True)
directory = MATCHES / STC
if directory.exists():
    raise SystemExit(f'{directory} はすでにあります')
print('== STC', flush=True)
with (MATCHES / f'{STC}.log').open('ab') as log:
    subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', '20261202900000',
                    '--candidate', 'commit:' + commits['positions-half'], '--baseline', 'commit:' + commits['games-half'],
                    '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                   cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)
print('finished', flush=True)
