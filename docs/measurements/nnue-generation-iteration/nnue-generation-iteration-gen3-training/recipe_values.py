#!/usr/bin/env python3
"""生成ログの記録局面数から、標準レシピv4の1 superbatchの更新回数Bを求める。

使い方: python3 recipe_values.py <生成ログのディレクトリ>
生成ログのディレクトリは、book-<i>.logを置いた gen-<N>/logs/ である。
式は設計書 docs/plans/nnue-generation-iteration.md の「世代の手順」の第3世代以降の手順に従う。
訓練局面数の見積りMは記録局面数の合計の95%、1エポックの更新回数の見積りUはM÷16,384の切り捨て、
BはU÷2の切り上げである。学習の長さと学習率の予定は、世代によらず8 superbatchと cosine:8:0.027 である。
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
    if len(argv) != 1:
        raise SystemExit(__doc__)
    logs = sorted(Path(argv[0]).glob('book-*.log'), key=lambda path: int(path.stem.split('-')[1]))
    if not logs:
        raise SystemExit('生成ログがありません')
    total = sum(recorded_positions(log) for log in logs)
    training = total * 0.95
    updates_per_epoch = math.floor(training / BATCH)
    print(json.dumps({
        'books': len(logs), 'recorded_positions': total, 'training_positions_estimate': training,
        'updates_per_epoch_estimate': updates_per_epoch,
        'batches_per_superbatch': math.ceil(updates_per_epoch / 2),
        'superbatches': 8, 'lr_schedule': 'cosine:8:0.027'}, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
