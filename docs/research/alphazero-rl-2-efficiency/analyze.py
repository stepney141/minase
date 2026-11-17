"""2系列の固定済み記録を比較する。エンジンと学習器は起動しない。

リポジトリの根で、progressのPython環境から実行する。
--records-rootにはcycles.jsonlとcycles-rl2-az.jsonlのあるディレクトリを渡す。
出力はこのファイルと同じディレクトリに置く。
"""

import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt


def read_csv(path):
    with path.open() as source:
        return list(csv.DictReader(source))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records-root", type=Path, required=True)
    args = parser.parse_args()
    out = Path(__file__).resolve().parent
    repo = out.parents[2]
    rows, summary = [], {"inputs": [], "series": {}}
    for number, name, limit in ((1, "cycles", 32), (2, "cycles-rl2-az", 100)):
        series = f"alphazero-rl-{number}"
        raw_path = args.records_root / f"{name}.jsonl"
        raw = raw_path.read_bytes()
        records = [json.loads(line) for line in raw.splitlines()]
        assert [r["cycle"] for r in records] == list(range(1, limit + 1))
        summary["inputs"].append({"path": str(raw_path), "sha256": hashlib.sha256(raw).hexdigest()})
        directory = repo / "docs" / "training" / series
        cycles = read_csv(directory / "cycles.csv")
        validation = {int(v["cycle"]): v for v in read_csv(directory / "validation.csv")}
        assert len(cycles) == limit
        cumulative_seen = cumulative_steps = cumulative_games = cumulative_plies = 0
        cumulative_seconds = 0.0
        for record, cycle in zip(records, cycles):
            c = record["cycle"]
            assert c == int(cycle["cycle"])
            train = record["training_summary"]
            cumulative_seen += train["positions_seen"]
            cumulative_steps += train["completed_steps"]
            cumulative_games += int(cycle["games"])
            cumulative_plies += int(cycle["plies"])
            stage_seconds = sum(record["elapsed_seconds"].values())
            cumulative_seconds += stage_seconds
            row = {
                "series": series, "cycle": c,
                "generated_positions": int(cycle["training_positions"]),
                "cumulative_generated_positions": int(cycle["cumulative_training_positions"]),
                "sample_count": train["sample_count"], "positions_seen": train["positions_seen"],
                "cumulative_positions_seen": cumulative_seen,
                "optimizer_steps": train["completed_steps"], "cumulative_optimizer_steps": cumulative_steps,
                "games": int(cycle["games"]), "cumulative_games": cumulative_games,
                "plies": int(cycle["plies"]), "cumulative_plies": cumulative_plies,
                "generation_seconds": record["elapsed_seconds"]["generation"],
                "training_stage_seconds": record["elapsed_seconds"]["training"],
                "optimizer_loop_seconds": train["elapsed_seconds"]["training"],
                "loading_seconds": train["elapsed_seconds"]["loading"],
                "diagnostic_seconds": train["elapsed_seconds"]["diagnostics"],
                "checking_seconds": record["elapsed_seconds"]["checking"],
                "stage_seconds": stage_seconds, "cumulative_stage_seconds": cumulative_seconds,
                "external_value_loss": "", "external_policy_agreement": "",
            }
            if c in validation:
                row["external_value_loss"] = float(validation[c]["value_result"])
                row["external_policy_agreement"] = float(validation[c]["policy_agreement"])
            rows.append(row)
        selected = [r for r in rows if r["series"] == series]
        summary["series"][series] = {
            "last_cycle": selected[-1],
            "last_20_medians": {key: statistics.median(r[key] for r in selected[-20:])
                                for key in ("generation_seconds", "training_stage_seconds",
                                            "optimizer_loop_seconds", "loading_seconds",
                                            "diagnostic_seconds", "checking_seconds", "stage_seconds")},
            "generation_commits": sorted({r["generation_commit"] for r in records}),
            "trainer_commits": sorted({r["trainer_commit"] for r in records}),
        }
        if number == 2:
            train = records[-1]["training_summary"]
            counts = {}
            for path, count in zip(train["config"]["data"], train["file_training_counts"]):
                c = int(re.search(r"cycle-(\d+)", path).group(1))
                if c not in counts:
                    counts[c] = 0
                counts[c] += count
            total = sum(counts.values())
            assert total == train["replay_buffer_training_count"]
            assert set(counts) == set(range(1, 101))
            # 各サイクルの一様な非復元抽出を独立とみなした期待値。実際の抽出列は再生しない。
            rates = [r["training_summary"]["positions_seen"] /
                     r["training_summary"]["replay_buffer_training_count"] for r in records]
            unused = {c: math.prod(1 - p for p in rates[c - 1:]) for c in counts}
            summary["replay_at_cycle_100"] = {
                "training_positions": total, "games": cumulative_games,
                "latest_cycle_positions": counts[100],
                "latest_cycle_fraction": counts[100] / total,
                "latest_cycle_expected_samples": counts[100] / total * train["positions_seen"],
                "first_50_cycles_fraction": sum(counts[c] for c in range(1, 51)) / total,
                "last_10_cycles_fraction": sum(counts[c] for c in range(91, 101)) / total,
                "mean_age_cycles": sum((100 - c) * n for c, n in counts.items()) / total,
                "expected_unused_fraction": sum(unused[c] * counts[c] for c in counts) / total,
                "last_50_expected_unused_fraction": sum(unused[c] * counts[c] for c in range(51, 101)) /
                                                     sum(counts[c] for c in range(51, 101)),
                "latest_train_diagnostic_positions": train["diagnostics"]["latest_training_sample"]["position_count"],
                "latest_validation_positions": train["diagnostics"]["validation"]["position_count"],
            }

    with (out / "cycles.csv").open("w") as target:
        writer = csv.DictWriter(target, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
    (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, allow_nan=False) + "\n")

    fig, axes = plt.subplots(2, 3, figsize=(13.2, 7.4), sharey="row", layout="constrained")
    quantities = (("cumulative_generated_positions", 1e6, "Saved training positions (millions)"),
                  ("cumulative_positions_seen", 1e6, "Training samples processed (millions)"),
                  ("cumulative_optimizer_steps", 1e3, "Optimizer updates (thousands)"))
    for series, label, color in (("alphazero-rl-1", "Run 1: 10 x 128", "#0072B2"),
                                 ("alphazero-rl-2", "Run 2: 20 x 256", "#D55E00")):
        points = [r for r in rows if r["series"] == series and r["external_value_loss"] != ""]
        for col, (quantity, scale, xlabel) in enumerate(quantities):
            x = [r[quantity] / scale for r in points]
            for index, metric, factor in ((0, "external_value_loss", 1), (1, "external_policy_agreement", 100)):
                axes[index, col].plot(x, [r[metric] * factor for r in points], color=color, label=label,
                                      marker="o", markersize=4 if series.endswith("1") else 2, linewidth=1.4)
                axes[index, col].set_xlabel(xlabel)
                axes[index, col].grid(alpha=0.2)
    for col, (quantity, scale, _) in enumerate(quantities):
        right = max(r[quantity] for r in rows) / scale * 1.04
        for index in range(2):
            axes[index, col].set_xlim(0, right)
    for ax in axes[0]:
        ax.axhline(math.log(2), color="#666666", linestyle="--", linewidth=0.8)
    axes[0, 0].set_ylabel("Result cross-entropy (lower is better)")
    axes[1, 0].set_ylabel("Human move agreement, % (higher is better)")
    axes[0, 0].legend(frameon=False)
    fig.suptitle("Fixed validation: 811 games, 164,504 positions\n"
                 "Learning rates, targets and architectures differ; these are diagnostic metrics, not Elo.", fontsize=12)
    fig.savefig(out / "comparison.png", dpi=180)
    plt.close(fig)
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
