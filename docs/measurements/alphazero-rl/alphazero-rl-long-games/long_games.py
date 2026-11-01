"""Summarize how self-play games of given cycles evolve, grouped by length.

Usage: long_games.py <cycle dir> [<cycle dir> ...]
Reads the non-resigned training files listed in each selfplay/summary.json.
Only full-search positions are recorded, so plies are sampled (~25%).
"""

import json
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np
import minase_mnsd_reader as reader

KINDS = ("Pawn GoBetween Lance ReverseChariot SideMover VerticalMover Bishop Rook DragonHorse "
         "DragonKing FreeKing King DrunkElephant FerociousLeopard BlindTiger CopperGeneral "
         "SilverGeneral GoldGeneral Kirin Phoenix Lion CrownPrince WhiteHorse Whale FlyingOx "
         "FreeBoar FlyingStag HornedFalcon SoaringEagle").split()
STRONG = {"Lion", "FreeKing", "DragonKing", "DragonHorse", "HornedFalcon", "SoaringEagle", "Rook",
          "Bishop", "FlyingOx", "FreeBoar", "FlyingStag", "Whale", "WhiteHorse"}


def decode(board: np.ndarray):
    """Return per-side piece counts, strong-piece counts, and royal counts for one board."""
    code = board & 0x7F
    black = (code >= 1) & (code <= 58)
    white = (code >= 65) & (code <= 122)
    payload = np.where(black, code - 1, np.where(white, code - 65, 0))
    kind = payload % 29
    promoted = payload >= 29
    names = [KINDS[k] for k in kind]
    out = {}
    for side, mask in (("b", black), ("w", white)):
        idx = np.nonzero(mask)[0]
        out[side] = len(idx)
        out[side + "_strong"] = sum(1 for i in idx if (names[i] in STRONG) or (promoted[i] and names[i] in {"Bishop", "Rook", "DragonHorse", "DragonKing"}))
        out[side + "_lion"] = sum(1 for i in idx if names[i] == "Lion" or (promoted[i] and names[i] == "Kirin"))
    return out


def games_of(path: Path):
    r = reader.read(str(path))
    games = np.frombuffer(r["games"], dtype="<u4")
    plies = np.frombuffer(r["plies"], dtype="<u2")
    boards = np.frombuffer(r["boards"], dtype=np.uint8).reshape(-1, 144)
    sides = np.frombuffer(r["sides"], dtype=np.uint8)
    values = np.frombuffer(r["values"], dtype="<i4")
    outcomes = np.frombuffer(r["outcomes"], dtype=np.uint8)
    order = np.lexsort((plies, games))
    groups = defaultdict(list)
    for i in order:
        groups[int(games[i])].append(i)
    for game, idx in groups.items():
        yield game, plies[idx], boards[idx], sides[idx], values[idx], int(outcomes[idx[0]])


def summarize(cycle_dir: Path):
    summary = json.loads((cycle_dir / "selfplay/summary.json").read_text())
    rows = []
    for name in summary["training_files"]:
        if name.endswith(".resigned.mnsd"):
            continue
        for game, plies, boards, sides, values, outcome in games_of(cycle_dir / "selfplay" / name):
            length = int(plies[-1])
            counts = [decode(b) for b in boards]
            total = np.array([c["b"] + c["w"] for c in counts])
            drops = np.nonzero(np.diff(total) < 0)[0]
            last_capture = int(plies[drops[-1] + 1]) if len(drops) else 0
            # value from the side to move, mapped to Black's win probability
            p = values / 65535.0
            p_black = np.where(sides == 0, p, 1.0 - p)
            tail = p_black[plies >= max(0, length - 300)]
            keys = [bytes(b) + bytes([s]) for b, s in zip(boards, sides)]
            seen, repeats = set(), 0
            for k in keys:
                repeats += k in seen
                seen.add(k)
            end = counts[-1]
            rows.append({
                "length": length, "outcome": outcome, "pieces_end": end["b"] + end["w"],
                "pieces_start": counts[0]["b"] + counts[0]["w"],
                "strong_end": end["b_strong"] + end["w_strong"], "lions_end": end["b_lion"] + end["w_lion"],
                "imbalance_end": abs(end["b"] - end["w"]),
                "since_capture": length - last_capture,
                "tail_decided": float(np.mean(np.abs(tail - 0.5) > 0.4)) if len(tail) else float("nan"),
                "tail_balanced": float(np.mean(np.abs(tail - 0.5) < 0.1)) if len(tail) else float("nan"),
                "repeat_fraction": repeats / len(keys),
            })
    return rows


def report(label: str, rows):
    bands = [(0, 400), (400, 800), (800, 1001)]
    print(f"== {label}: {len(rows)} games")
    for lo, hi in bands:
        b = [r for r in rows if lo <= r["length"] < hi]
        if not b:
            continue
        def med(key):
            return float(np.nanmedian([r[key] for r in b]))
        draws = sum(r["outcome"] == 2 for r in b) / len(b)
        print(f"  plies {lo}-{hi - 1}: n={len(b)} draws={draws:.1%} pieces end={med('pieces_end'):.0f}"
              f" (start {med('pieces_start'):.0f}) strong end={med('strong_end'):.0f} lions end={med('lions_end'):.0f}"
              f" imbalance={med('imbalance_end'):.0f} plies since capture={med('since_capture'):.0f}"
              f" tail decided={med('tail_decided'):.2f} balanced={med('tail_balanced'):.2f}"
              f" repeated positions={med('repeat_fraction'):.2f}")


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        report(arg, summarize(Path(arg)))
