"""強化学習の系列の各サイクルの実時間を、実行器の記録から工程と工程間の時間に分ける。

使い方: python3 cycle_breakdown.py <NASのalphazeroディレクトリ> <系列名> [最初のサイクル]
系列名は cycles-rl2-az のような系列のディレクトリ名で、<系列名>.jsonl を読む。
出力は標準出力のCSVで、末尾に指定サイクル以降の各列の中央値を表示する。
秒の欄は次の記録から求める。
- total: <系列名>.jsonlのintervalのstarted_atからended_atまで
- gen_stage, train_stage, check_stage: 作業機の<工程>.started.jsonから<工程>.finished.jsonまで
- gen_inner: 生成器が標準エラーへ出した最後の塊のelapsed_seconds（その後の回収と集計の保存は含まない）
- gap_gt: 生成の受信完了から学習の起動まで
- load, train_loop, diag: 学習器のsummaryのelapsed_seconds（起動、コンパイルの一部、書き出しは含まない）
- check_inner: 借りた機械で照合のスクリプト自体にかかった秒数
- gen_rest, train_rest, check_rest: 各工程の時間から、上の計時された区間を引いた残り
- residual: totalからgen_inner、load、train_loop、diag、check_innerを引いた残り
生成の工程の記録がないサイクルと、生成器が塊の進捗を出さなかったサイクル（再開で確定済みの塊だけで目標に届いた場合）は除き、除いた理由を標準エラーに出す。
"""
import json
import re
import statistics
import sys
from datetime import datetime
from pathlib import Path

root = Path(sys.argv[1])
series = sys.argv[2]
first = int(sys.argv[3]) if len(sys.argv) > 3 else 4
cycles = root / series
records = {json.loads(line)["cycle"]: json.loads(line) for line in open(root / f"{series}.jsonl")}


def stamp(path: Path) -> datetime:
    data = json.loads(path.read_text())
    return datetime.fromisoformat(data.get("started_at") or data["finished_at"])


keys = ["total", "gen_stage", "gen_inner", "gap_gt", "train_stage", "load", "train_loop", "diag",
        "check_stage", "check_inner", "gen_rest", "train_rest", "check_rest", "residual"]
rows = []
print("cycle," + ",".join(keys))
for cycle, record in sorted(records.items()):
    directory = cycles / f"cycle-{cycle:04d}"
    try:
        g0, g1 = stamp(directory / "selfplay.started.json"), stamp(directory / "selfplay.finished.json")
        t0, t1 = stamp(directory / "train.started.json"), stamp(directory / "train.finished.json")
        k0, k1 = stamp(directory / "check.started.json"), stamp(directory / "check.finished.json")
    except FileNotFoundError as error:
        print(f"skip cycle {cycle}: missing stage record {error.filename}", file=sys.stderr)
        continue
    chunks = [float(m.group(1)) for m in re.finditer(r"elapsed_seconds: ([\d.]+)", (directory / "selfplay.err").read_text())]
    if not chunks:
        # 確定済みの塊だけで目標に届いた再開では、生成器が進捗を出さないので比べられない。
        print(f"skip cycle {cycle}: the generator printed no chunk progress", file=sys.stderr)
        continue
    interval = record["interval"]
    trainer = record["training_summary"]["elapsed_seconds"]
    row = {
        "total": (datetime.fromisoformat(interval["ended_at"]) - datetime.fromisoformat(interval["started_at"])).total_seconds(),
        "gen_stage": (g1 - g0).total_seconds(),
        "gen_inner": chunks[-1],
        "gap_gt": (t0 - g1).total_seconds(),
        "train_stage": (t1 - t0).total_seconds(),
        "load": trainer["loading"],
        "train_loop": trainer["training"],
        "diag": trainer["diagnostics"],
        "check_stage": (k1 - k0).total_seconds(),
        "check_inner": record["checks"]["elapsed_seconds"],
    }
    row["gen_rest"] = row["gen_stage"] - row["gen_inner"]
    row["train_rest"] = row["train_stage"] - row["load"] - row["train_loop"] - row["diag"]
    row["check_rest"] = row["check_stage"] - row["check_inner"]
    row["residual"] = row["total"] - sum(row[key] for key in ("gen_inner", "load", "train_loop", "diag", "check_inner"))
    print(f"{cycle}," + ",".join(f"{row[key]:.1f}" for key in keys))
    if cycle >= first:
        rows.append(row)
if not rows:
    sys.exit(f"no comparable cycles at or after cycle {first}")
print("median," + ",".join(f"{statistics.median(row[key] for row in rows):.1f}" for key in keys))
