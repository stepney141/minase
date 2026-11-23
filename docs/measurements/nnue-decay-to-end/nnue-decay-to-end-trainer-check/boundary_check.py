#!/usr/bin/env python3
"""エポックの境界の前後で検証損失を比べ、設計書のフェーズ1の基準に合うかを判定する。

使い方: python3 boundary_check.py <学習の出力ディレクトリ>...
各ディレクトリの config.json から1エポックの更新回数Uを、training.jsonl から検証損失を読む。
1回目と2回目の境界（U更新目と2U更新目）について、次の2つの平均を比べる。
  (a) 境界の更新より後、12更新後までにある検証損失の平均
  (b) 境界の40更新前より後、境界の更新までにある検証損失の平均
差の絶対値が(b)の標準偏差の2倍以内なら合格とし、1つでも外れれば終了コード1を返す。
"""

import json
from pathlib import Path
import statistics
import sys


def check(directory: Path) -> bool:
    updates = json.loads((directory / 'config.json').read_text())['preprocessing']['updates_per_epoch']
    rows = [json.loads(line) for line in (directory / 'training.jsonl').read_text().splitlines()]
    points = [(row['total_steps'], row['validation_loss']) for row in rows if row['validation_loss'] is not None]
    passed = True
    for boundary in (updates, 2 * updates):
        after = [loss for step, loss in points if boundary < step <= boundary + 12]
        before = [loss for step, loss in points if boundary - 40 < step <= boundary]
        if len(after) < 2 or len(before) < 2:
            raise SystemExit(f'{directory}: 境界{boundary}の前後に検証点が足りません')
        difference = statistics.mean(after) - statistics.mean(before)
        deviation = statistics.pstdev(before)
        ok = abs(difference) <= 2 * deviation
        passed = passed and ok
        print(f'{directory.name} U={updates} 境界={boundary} (a)={statistics.mean(after):.9f} '
              f'(b)={statistics.mean(before):.9f} 差={difference / statistics.mean(before) * 100:+.2f}% '
              f'(b)の標準偏差={deviation / statistics.mean(before) * 100:.2f}% {"合格" if ok else "不合格"}')
    return passed


def main(argv: list[str]) -> int:
    if not argv:
        raise SystemExit(__doc__)
    results = [check(Path(argument)) for argument in argv]
    return 0 if all(results) else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
