#!/usr/bin/env python3
"""生成ログの記録局面数から、世代反復の本数kと学習の引数B、T、Sを求める。

使い方: python3 recipe_values.py <世代N> <生成ログのディレクトリ>
生成ログのディレクトリは、book-<i>.logを置いた gen-<N>/logs/ である。
式は設計書 docs/plans/nnue-generation-iteration.md の「世代の手順」に従う。
訓練局面数Mは記録局面数の合計の95%、1エポックの更新回数UはM÷16,384の切り上げ、
検証の間隔BはU÷2の切り上げ、減衰の期間Tは減衰の更新回数÷B、上限SはTの切り上げである。
減衰の更新回数は、第1世代から第4世代が390,656、第5世代が488,320である。
kは、1億を1本あたりの記録局面数の平均で割って整数に丸めた値であり、第1世代の最初の4本で求める。
"""

import json
import math
from pathlib import Path
import re
import sys

BATCH = 16384


def recorded_positions(log: Path) -> int:
    """同名の欄は最後の値を使い、生成ログの記録局面数を返す。"""
    values = re.findall(r'^recorded_positions:\s*([0-9]+)\s*$', log.read_text(), re.M)
    if not values:
        raise SystemExit(f'recorded_positionsがありません: {log}')
    return int(values[-1])


def main(argv: list[str]) -> int:
    generation = int(argv[0])
    logs = sorted(Path(argv[1]).glob('book-*.log'), key=lambda path: int(path.stem.split('-')[1]))
    if not logs:
        raise SystemExit('生成ログがありません')
    counts = [recorded_positions(log) for log in logs]
    total = sum(counts)
    mean = total / len(counts)
    training = total * 0.95
    updates_per_epoch = math.ceil(training / BATCH)
    interval = math.ceil(updates_per_epoch / 2)
    decay_updates = 488320 if generation >= 5 else 390656
    horizon = decay_updates / interval
    print(json.dumps({
        'generation': generation, 'books': len(counts), 'recorded_positions': total,
        'recorded_positions_per_book': mean, 'k': round(100_000_000 / mean),
        'training_positions_estimate': training, 'updates_per_epoch': updates_per_epoch,
        'batches_per_superbatch': interval, 'lr_horizon_superbatches': horizon,
        'superbatches': math.ceil(horizon),
        'lr_schedule': f'cosine:{horizon!r}:0.027'}, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
