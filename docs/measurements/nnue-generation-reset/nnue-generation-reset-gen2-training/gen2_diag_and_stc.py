"""第2世代のネットの診断を順に実行し、続けて第1世代とのSTCを実行する。診断の報告が残っていれば計算を省く。"""
import os
import subprocess
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
DATA = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-generation-reset'
MINASE = str(DATA / 'bin/minase')
TRAINER = str(DATA / 'bin/minase-trainer')
GAP = str(ROOT / 'data/nnue-architecture-revision/outcome-gap/new/target/release/outcome-gap')
PST = str(WORKTREE / 'crates/minase/nets/pst.bin')
LISHOGI = str(ROOT / 'data/out-of-core-shuffle/teacher/lishogi.bin')
MATCHES = ROOT / 'data/matches'
GEN = DATA / 'gen-2'
BASE = str(DATA / 'base.bin')
NET1 = str(DATA / 'gen-1/fit/nnue.bin')
FLOAT1 = str(DATA / 'gen-1/train-lambda1/best.npz')
TRAINED = str(GEN / 'train/nnue.bin')
FITTED = str(GEN / 'fit/nnue.bin')
FLOAT2 = str(GEN / 'train/best.npz')
BOOKS = [str(path) for path in sorted((GEN / 'teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]
DIAG = GEN / 'diag'
DIAG.mkdir(exist_ok=True)
GEN1_COMMIT = '7c2cfe4eb868fe54ddcc31187a900efe3a30a7d4'
GEN2_COMMIT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
STC = 'nnue-generation-reset-gen2-stc'


def run(name, *command):
    print('==', name, flush=True)
    with (DIAG / f'{name}.log').open('w') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE,
                       preexec_fn=lambda: os.nice(19))
    print('   done', flush=True)


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
run('distribution', TRAINER, 'diag', 'nnue-distribution', '--pair', 'gen0', PST, BASE, '--pair', 'gen1', PST, NET1,
    '--pair', 'gen2-trained', PST, TRAINED, '--pair', 'gen2', PST, FITTED,
    '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / 'distribution.json'))
# 符号反転は整数のネットとv0の駒価値の比較である。補正後のネットは組の量子化前の重みを持たないので、
# 量子化前後の差の欄は読まない。
run('removal-gen2', TRAINER, 'diag', 'nnue-removal', '--float', FLOAT2, '--net', FITTED,
    '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / 'removal-gen2.json'))
run('removal-gen1', TRAINER, 'diag', 'nnue-removal', '--float', FLOAT1, '--net', NET1,
    '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / 'removal-gen1.json'))
for label, net in (('gen2-trained', TRAINED), ('gen2', FITTED)):
    run(f'moves-{label}', TRAINER, 'diag', 'nnue-moves', '--net', net, '--base', PST, '--positions', LISHOGI,
        '--output', str(DIAG / f'moves-{label}.json'))
run('gap', GAP, str(DIAG / 'gap.csv'), '--net', f'gen0={BASE}', '--net', f'gen2-trained={TRAINED}',
    '--net', f'gen2={FITTED}', '--data', *BOOKS)
print('== stc', flush=True)
directory = MATCHES / STC
if directory.exists():
    raise SystemExit(f'{directory} はすでにあります')
with (MATCHES / f'{STC}.log').open('ab') as log:
    subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', '20261202000000',
                    '--candidate', 'commit:' + GEN2_COMMIT, '--baseline', 'commit:' + GEN1_COMMIT,
                    '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                   cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)
print('finished', flush=True)
