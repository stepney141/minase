"""measure.sh の出力ディレクトリを1つ読み、測定記録の表に使う値をJSONで標準出力へ書く。"""

import json
import sys
from pathlib import Path


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text().splitlines() if line]


def main() -> None:
    out = Path(sys.argv[1])
    timings = {}
    t0 = None
    for line in (out / "timings.tsv").read_text().splitlines():
        name, stamp = line.split("\t")
        t0 = float(stamp) if t0 is None else t0
        timings[name] = float(stamp) - t0
    phases = {
        phase: timings[f"{phase}-end"] - timings[f"{phase}-start"]
        for phase in ("setup", "build-feasibility", "build-minase", "nets", "trt", "sim", "train")
        if f"{phase}-end" in timings
    }

    inference = [
        {
            "net": r["model"].rsplit("/", 1)[1].removeprefix("zero-").removesuffix(".onnx"),
            "io": r["io"],
            "precision": "fp16" if r["fp16"] else "fp32",
            "batch": r["batch"],
            "evals_per_sec": round(r["evals_per_sec"]),
            "latency_p50_ms": round(r["latency_ms"]["p50"], 2),
            "gpu_mem_mib": r["gpu_mem_mib"],
            "session_create_secs": round(r["session_create_secs"], 1),
        }
        for r in read_jsonl(out / "trt.jsonl")
    ]

    layers = (out / "trt-verbose-layers.txt").read_text().splitlines()
    convolutions = [line for line in layers if line.startswith("Layer(CaskConvolution)")]
    # 層の行は「入力 -> 出力名 (型[形])」で終わるので、最後の矢印より後の型を見る。
    half_convolutions = [line for line in convolutions if "(Half[" in line.rsplit("-> ", 1)[1]]

    breakdown = json.loads((out / "sim-breakdown.json").read_text())
    parallel = {}
    for path in sorted(out.glob("sim-p*-*.json")):
        workers = int(path.name.split("-")[1].removeprefix("p"))
        parallel.setdefault(workers, []).append(json.loads(path.read_text()))
    simulation = {
        "breakdown_us_per_simulation": {
            k: round(v, 3) for k, v in breakdown["breakdown_us_per_simulation"].items()
        },
        "breakdown_mean_us_per_simulation": round(breakdown["mean_us_per_simulation"], 3),
        "parallel": {
            workers: {
                "mean_us_per_simulation": round(
                    sum(r["mean_us_per_simulation"] for r in runs) / len(runs), 3
                ),
                "total_simulations_per_sec": round(sum(r["simulations_per_second"] for r in runs)),
            }
            for workers, runs in sorted(parallel.items())
        },
        "mean_root_legal_moves": round(breakdown["mean_root_legal_moves"], 1),
    }

    training = [
        {
            "net": f"b{r['blocks']}c{r['channels']}",
            "precision": r["precision"],
            "batch": r["batch"],
            "positions_per_sec": round(r["positions_per_sec"]),
            "max_memory_mib": round(r["max_memory_allocated_mib"]),
            "secs_per_4m_positions": round(r["secs_per_4m_positions"]),
        }
        for r in read_jsonl(out / "train.jsonl")
    ]
    failures = out / "train-failures.tsv"
    print(
        json.dumps(
            {
                "phases_secs": {k: round(v) for k, v in phases.items()},
                "inference": inference,
                "fp16_convolutions": f"{len(half_convolutions)}/{len(convolutions)}",
                "simulation": simulation,
                "training": training,
                "training_failures": failures.read_text().splitlines() if failures.exists() else [],
            },
            ensure_ascii=False,
            indent=1,
        )
    )


if __name__ == "__main__":
    main()
