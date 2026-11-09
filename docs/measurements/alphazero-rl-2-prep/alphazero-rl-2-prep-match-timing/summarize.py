"""対局の実行ディレクトリから、1局の手数と経過時間を集計し、1局の固定費と1手あたりの時間を最小二乗法で求める。

使い方: python3 summarize.py <基準の実行ディレクトリ> <実行ディレクトリ>...
基準の実行ディレクトリ（1回目の学習のサイクル30の進捗測定）の手数の分布を、後に並べた実行ディレクトリの
合計から求めた固定費と1手あたりの時間に当てはめ、同時対局数2で同じ局数を指した場合の所要時間を見積もる。
"""

import json
import sys
from pathlib import Path


def games(run: Path) -> list[tuple[int, float]]:
    rows = []
    for path in sorted((run / "pairs").glob("*.json")):
        for game in json.loads(path.read_text(encoding="utf-8"))["games"]:
            rows.append((len(game["turns"]), game["wall_time_ns"] / 1e9))
    if not rows:
        raise ValueError(f"{run}: no finished games")
    return rows


def fit(rows: list[tuple[int, float]]) -> tuple[float, float]:
    count = len(rows)
    mean_turns = sum(turns for turns, _ in rows) / count
    mean_wall = sum(wall for _, wall in rows) / count
    spread = sum((turns - mean_turns) ** 2 for turns, _ in rows)
    if spread == 0:
        raise ValueError("all games have the same length")
    slope = sum((turns - mean_turns) * (wall - mean_wall) for turns, wall in rows) / spread
    return mean_wall - slope * mean_turns, slope


def describe(name: str, rows: list[tuple[int, float]]) -> None:
    turns = [turns for turns, _ in rows]
    print(f"{name}: games={len(rows)} turns={sum(turns)} mean_turns={sum(turns) / len(rows):.1f} "
          f"min_turns={min(turns)} max_turns={max(turns)} "
          f"wall_per_turn={sum(wall for _, wall in rows) / sum(turns):.4f}s")


def describe_fit(name: str, rows: list[tuple[int, float]]) -> tuple[float, float]:
    describe(name, rows)
    fixed, per_turn = fit(rows)
    residual = max(abs(wall - (fixed + per_turn * length)) for length, wall in rows)
    print(f"  fit: fixed={fixed:.2f}s per_turn={per_turn:.4f}s max_residual={residual:.2f}s")
    return fixed, per_turn


def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    reference = games(Path(sys.argv[1]))
    describe_fit(sys.argv[1], reference)
    measured = []
    for name in sys.argv[2:]:
        rows = games(Path(name))
        describe(name, rows)
        measured += rows
    fixed, per_turn = describe_fit("measured (all)", measured)
    seconds = (len(reference) * fixed + per_turn * sum(turns for turns, _ in reference)) / 2
    print(f"estimate for {len(reference)} games of the reference lengths at concurrency 2: "
          f"{seconds:.0f}s ({seconds / 3600:.2f}h)")


if __name__ == "__main__":
    main()
