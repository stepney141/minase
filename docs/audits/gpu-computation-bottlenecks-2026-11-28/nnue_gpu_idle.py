"""NNUE学習の実行ディレクトリごとに、学習中にGPUが更新を計算していない時間の割合を求める。

使い方: python3 nnue_gpu_idle.py <実行ディレクトリ>...
各ディレクトリのtraining.jsonlとconfig.jsonを読む。
superbatchごとの実効の1更新はtraining_seconds/steps、GPUの1更新はupdate_secondsであり、
空きの割合は1 − GPUの1更新/実効の1更新とする。値はsuperbatchの中央値である。
"""
import json
import statistics
import sys
from pathlib import Path

for directory in map(Path, sys.argv[1:]):
    rows = [json.loads(line) for line in open(directory / "training.jsonl")]
    config = json.loads((directory / "config.json").read_text())
    rows = [row for row in rows if row.get("training_seconds") and row.get("update_seconds") and row.get("steps")]
    effective = statistics.median(row["training_seconds"] / row["steps"] * 1e3 for row in rows)
    gpu = statistics.median(row["update_seconds"] * 1e3 for row in rows)
    speed = statistics.median(row["training_positions_per_second"] for row in rows)
    print(f"{directory}\t{config['device_name']}\tthreads={config['resolved_threads']}\t"
          f"effective={effective:.2f}ms\tgpu={gpu:.2f}ms\tidle={1 - gpu / effective:.1%}\tpositions/s={speed:,.0f}")
