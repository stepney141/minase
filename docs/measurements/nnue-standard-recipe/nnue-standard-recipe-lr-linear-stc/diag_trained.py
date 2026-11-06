"""測定1の候補の補正前のネットについて、駒除去差分の符号反転を対照と同じ標本で測る。

本は番号順に並べ、対照の診断（中断した系列の第2世代の学習の記録）と同じ--sample 10000 --seed 1を使う。
"""
import os
import subprocess
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
OLD = ROOT / 'data/nnue-generation-reset'
OUTPUT = ROOT / 'data/nnue-standard-recipe/recipe/lr-linear'
BOOKS = [str(path) for path in sorted((OLD / 'gen-2/teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]

if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
report = OUTPUT / 'diag/removal-candidate-trained.json'
if not report.is_file():
    with (OUTPUT / 'diag/removal-candidate-trained.log').open('w') as log:
        subprocess.run([str(OLD / 'bin/minase-trainer'), 'diag', 'nnue-removal', '--float', str(OUTPUT / 'train/best.npz'),
                        '--net', str(OUTPUT / 'train/nnue.bin'), '--data', *BOOKS, '--sample', '10000', '--seed', '1',
                        '--output', str(report)],
                       check=True, stdout=log, stderr=subprocess.STDOUT, preexec_fn=lambda: os.nice(19))
print('done', report)
