"""同じ対局の将来の探索値を、手数差による幾何加重平均へ変換する。"""

from __future__ import annotations

import numpy as np


def validate_lookahead(gamma: float, plies: int) -> None:
    if type(gamma) not in (int, float) or not np.isfinite(gamma) or not 0 < gamma < 1:
        raise ValueError("lookahead gamma must be finite and in (0, 1)")
    if type(plies) is not int or not 1 <= plies <= 4096:
        raise ValueError("lookahead plies must be an integer in 1..4096")


def lookahead_options(gamma: float | None, plies: int | None) -> dict | None:
    """CLIの2引数は、両方省略するか両方指定する。"""
    if gamma is None and plies is None:
        return None
    validate_lookahead(gamma, plies)
    return {"gamma": gamma, "plies": plies}


def _ordered(game: np.ndarray, ply: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    game, ply = np.asarray(game), np.asarray(ply)
    if (game.ndim != 1 or ply.shape != game.shape
            or game.dtype.kind not in "iu" or ply.dtype.kind not in "iu"):
        raise ValueError("game and ply must be matching one-dimensional integer arrays")
    if (np.any(game[1:] < game[:-1])
            or np.any((game[1:] == game[:-1]) & (ply[1:] <= ply[:-1]))):
        raise ValueError("records must be ordered by game and strictly increasing ply")
    return game, ply.astype(np.int64)


def window_statistics(game: np.ndarray, ply: np.ndarray, plies: int) -> dict[str, np.ndarray]:
    """窓内件数、最初の記録までの手数（退避時は0）、退避の真偽を返す。"""
    validate_lookahead(0.9, plies)
    game, ply = _ordered(game, ply)
    counts = np.zeros(len(game), dtype=np.int64)
    first = np.zeros(len(game), dtype=np.int64)
    boundaries = np.r_[0, np.flatnonzero(game[1:] != game[:-1]) + 1, len(game)]
    for start, end in zip(boundaries[:-1], boundaries[1:]):
        p = ply[start:end]
        counts[start:end] = np.searchsorted(p, p + plies, side="right") - np.arange(len(p)) - 1
    if len(ply) > 1:
        first[:-1] = np.where(counts[:-1] > 0, np.diff(ply), 0)
    return {"record_count": counts, "first_record_plies": first, "fallback": counts == 0}


def compute_lookahead(game: np.ndarray, ply: np.ndarray, score: np.ndarray,
                      gamma: float, plies: int) -> np.ndarray:
    """定義した窓だけを使い、元のscoreを変更せずfloat64で返す。"""
    validate_lookahead(gamma, plies)
    game, ply = _ordered(game, ply)
    score = np.asarray(score, dtype=np.float64)
    if score.shape != game.shape or not np.all(np.isfinite(score)):
        raise ValueError("score must be a matching finite one-dimensional array")
    result = score.copy()
    # 記録数の差で短いループを回し、各ブロックの局面を同時に計算する。
    # 手数が厳密増加なので、窓内の記録数もplies以下である。
    for start in range(0, len(game), 65536):
        end = min(start + 65536, len(game))
        numerator = np.zeros(end - start)
        denominator = np.zeros(end - start)
        for shift in range(1, min(plies, len(game) - start - 1) + 1):
            stop = min(end, len(game) - shift)
            size = stop - start
            delta = ply[start + shift:stop + shift] - ply[start:stop]
            valid = (game[start:stop] == game[start + shift:stop + shift]) & (delta <= plies)
            if not np.any(valid):
                break
            first_delta = ply[start + 1:stop + 1] - ply[start:stop]
            # 共通因子γ^(first_delta-1)を消し、疎な窓でも0/0を防ぐ。
            weight = np.zeros(size)
            weight[valid] = gamma ** (delta[valid] - first_delta[valid])
            numerator[:size] += weight * np.where(delta % 2 == 0, 1, -1) * score[start + shift:stop + shift]
            denominator[:size] += weight
        np.divide(numerator, denominator, out=result[start:end], where=denominator > 0)
    return result
