"""試走の生成ごとに、保存した局面の速さ、終局理由、1局あたりの局面数、GPUとCPUの使用率をまとめる。

使い方: python3 summarize.py <生成の出力先>...
各出力先には、gen.shが書く<出力先>.start、.end、.err、.monitorと、生成器のsummary.jsonがあること。
速さは、生成器のsummary.jsonのtraining_positionsを、gen.shが記録した起動から終了までの秒数で割った値とする。
TensorRTのエンジンの作成と、同時対局数が減っていく終盤を含む。各組の対局数を同時対局数の4倍にそろえたので、
終盤の割合は組の間でほぼ等しい。塊は最も遅い対局の終局を待って確定し、確定が終盤にまとまるので、
塊の確定の時刻から途中の速さは求めない。
"""

import json
from pathlib import Path
import re
import statistics
import sys

MONITOR = re.compile(r"^\d+ gpu (\d+) %, (\d+) MiB cpu %Cpu\(s\):\s*([\d.]+) us,\s*([\d.]+) sy,.*?([\d.]+) id")


def summarize(out: Path) -> dict:
    start = float(Path(f"{out}.start").read_text())
    end = float(Path(f"{out}.end").read_text())
    summary = json.loads((out / "summary.json").read_text())
    gpu, cpu_busy = [], []
    for line in Path(f"{out}.monitor").read_text().splitlines():
        match = MONITOR.match(line)
        if match:
            gpu.append(int(match[1]))
            cpu_busy.append(100.0 - float(match[5]))
    if not gpu:
        raise ValueError(f"{out}: no monitor samples")
    reasons = {key: value for key, value in summary.items()
               if (key.startswith("win_reason_") or key.startswith("draw_reason_")) and value}
    return {
        "run": out.name,
        "games": summary["games"],
        "training_positions": summary["training_positions"],
        "positions_per_game": summary["training_positions"] / summary["games"],
        "plies_per_game": summary["plies"] / summary["games"],
        "discarded_max_ply": summary["discarded_max_ply"],
        "reasons": reasons,
        "draws": summary["draws"],
        "wall_seconds": end - start,
        "positions_per_second": summary["training_positions"] / (end - start),
        "gpu_utilization_median": statistics.median(gpu),
        "cpu_busy_median": statistics.median(cpu_busy),
        "monitor_samples": len(gpu),
    }


if __name__ == "__main__":
    for argument in sys.argv[1:]:
        print(json.dumps(summarize(Path(argument)), ensure_ascii=False))
