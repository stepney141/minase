#!/usr/bin/env python3
"""合法手の評価差の診断（minase-trainer diag nnue-moves）の報告を、全局面の着手で重みづけて集計する。

使い方: python3 moves_summary.py <moves.json>
報告は局面ごとの統計を持つので、着手数で重みづけた平均と、全局面での最大値を出す。
candidateはNNUE、baselineはPST＋FMであり、値は着手による評価の変化（cp）である。
"""

import json
import sys


def main(argv: list[str]) -> int:
    report = json.load(open(argv[0], encoding='utf-8'))
    rows = report['by_position']
    print(f'局面 {report["position_count"]}  終局の着手 {report["terminal_count"]}  '
          f'NNUEだけの着手 {report["candidate_only_count"]}  PST＋FMだけの着手 {report["baseline_only_count"]}')
    print('項目  着手数  平均  絶対値の平均  絶対値の最大')
    for key in ('candidate', 'baseline', 'candidate_minus_baseline', 'candidate_minus_baseline_pst'):
        count = sum(row[key]['count'] for row in rows)
        mean = sum(row[key]['mean_cp'] * row[key]['count'] for row in rows) / count
        absolute = sum(row[key]['mean_absolute_cp'] * row[key]['count'] for row in rows) / count
        maximum = max(row[key]['max_absolute_cp'] for row in rows)
        print(f'{key} {count} {mean:+.2f} {absolute:.2f} {maximum:.0f}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
