"""第1世代の2つのネットについて、設計書の診断を順に実行する。出力が残っていれば失敗して止まる。"""
import json
import subprocess
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
DATA = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-generation-reset'
TRAINER = str(DATA / 'bin/minase-trainer')
GAP = str(ROOT / 'data/nnue-architecture-revision/outcome-gap/new/target/release/outcome-gap')
PST = str(WORKTREE / 'crates/minase/nets/pst.bin')
LISHOGI = str(ROOT / 'data/out-of-core-shuffle/teacher/lishogi.bin')
GEN = DATA / 'gen-1'
BASE = str(DATA / 'base.bin')
NET75 = str(GEN / 'train/nnue.bin')
NET100 = str(GEN / 'train-lambda1/nnue.bin')
BOOKS = json.loads((GEN / 'train/config.json').read_text())['arguments']['data']
DIAG = GEN / 'diag'
DIAG.mkdir()


def run(name, *command):
    print('==', name, flush=True)
    with (DIAG / f'{name}.log').open('w') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE)
    print('   done', flush=True)


run('distribution', TRAINER, 'diag', 'nnue-distribution',
    '--pair', 'gen0', PST, BASE, '--pair', 'lambda075', PST, NET75, '--pair', 'lambda100', PST, NET100,
    '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / 'distribution.json'))
run('removal-lambda100', TRAINER, 'diag', 'nnue-removal', '--float', str(GEN / 'train-lambda1/best.npz'), '--net', NET100,
    '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / 'removal-lambda100.json'))
for label, net in (('lambda075', NET75), ('lambda100', NET100)):
    run(f'moves-{label}', TRAINER, 'diag', 'nnue-moves', '--net', net, '--base', PST, '--positions', LISHOGI,
        '--output', str(DIAG / f'moves-{label}.json'))
    run(f'fit-{label}', TRAINER, 'nnue', 'fit-linear', '--base', net, '--data', *BOOKS, '--tau', '10', '20', '40', '80',
        '--seed', '20261105', '--threads', '12', '--output-dir', str(DIAG / f'fit-{label}'))
run('gap', GAP, str(DIAG / 'gap.csv'), '--net', f'gen0={BASE}', '--net', f'lambda075={NET75}', '--net', f'lambda100={NET100}',
    '--data', *BOOKS)
print('finished', flush=True)
