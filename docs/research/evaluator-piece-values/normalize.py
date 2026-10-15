"""removal.tsv の駒除去差分の平均を、初期配置の片側45枚（王駒を除く）の合計が
手書き評価v0の合計に一致するように評価関数ごとに一律の倍率をかけて表示する。"""
import csv
import sys

rows = {int(r["state"]): r for r in csv.DictReader(open(sys.argv[1]), delimiter="\t")}
# 駒状態番号: 成れる駒は29〜46、奔王10と獅子20は成れない状態。
start = {29: 12, 30: 2, 31: 2, 32: 2, 33: 2, 34: 2, 35: 2, 36: 2, 37: 2, 38: 2,
         39: 1, 40: 2, 41: 2, 42: 2, 43: 2, 44: 2, 45: 1, 46: 1, 10: 1, 20: 1}
assert sum(start.values()) == 45
columns = ["mean_hc", "mean_L", "mean_J", "mean_JFM", "mean_NNUE"]
totals = {c: sum(n * float(rows[s][c]) for s, n in start.items()) for c in columns}
print("army_total\t" + "\t".join(f"{totals[c]:.0f}" for c in columns))
scale = {c: totals["mean_hc"] / totals[c] for c in columns}
print("scale\t" + "\t".join(f"{scale[c]:.4f}" for c in columns))
print("state\tname\t" + "\t".join(columns))
for s, r in sorted(rows.items()):
    print(f"{s}\t{r['name']}\t" + "\t".join(f"{float(r[c]) * scale[c]:.0f}" for c in columns))
