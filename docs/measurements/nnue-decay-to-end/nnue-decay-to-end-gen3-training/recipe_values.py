#!/usr/bin/env python3
"""第N世代の生成ログの記録局面数から、2つの条件の学習の引数を求める。

使い方: python3 recipe_values.py <世代N> <生成ログのディレクトリ>
生成ログのディレクトリは、book-<i>.logを置いた gen-<N>/logs/ である。
式は設計書 docs/plans/nnue-decay-to-end.md の「測定と判定」に従う。
訓練局面数の見積りMは記録局面数の合計の95%、1エポックの更新回数の見積りUはM÷16,384の切り捨て、
1 superbatchの更新回数BはU÷2の切り上げであり、Bは2つの条件で同じ値を使う。
現行の条件は、減衰の期間T＝減衰の更新回数÷B、上限S＝Tの切り上げ、--validation-patience 2とする。
減衰の更新回数は、第1世代から第4世代が390,656、第5世代が488,320である。
候補の条件は、8 superbatchで学習を終え、同じ8 superbatchで学習率を下げきる。
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
    if len(argv) != 2:
        raise SystemExit(__doc__)
    generation = int(argv[0])
    logs = sorted(Path(argv[1]).glob('book-*.log'), key=lambda path: int(path.stem.split('-')[1]))
    if not logs:
        raise SystemExit('生成ログがありません')
    total = sum(recorded_positions(log) for log in logs)
    training = total * 0.95
    updates_per_epoch = math.floor(training / BATCH)
    interval = math.ceil(updates_per_epoch / 2)
    decay_updates = 488320 if generation >= 5 else 390656
    horizon = decay_updates / interval
    print(json.dumps({
        'generation': generation, 'books': len(logs), 'recorded_positions': total,
        'training_positions_estimate': training, 'updates_per_epoch_estimate': updates_per_epoch,
        'batches_per_superbatch': interval,
        'current': {'superbatches': math.ceil(horizon), 'lr_schedule': f'cosine:{horizon!r}:0.027',
                    'validation_patience': 2},
        'candidate': {'superbatches': 8, 'lr_schedule': 'cosine:8:0.027'}}, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
