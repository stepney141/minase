"""探索なしの実戦対局を記録した検証用MNSD 1ファイルで、学習せずに診断する。

入力の教師はNoSearch、由来は実戦の対局、解釈した規則はlishogiであることを要求する。
指した手を持つ全項目を入力順に読み、最後の項目は含めない。対局または局面が0件の
ファイルは拒否する。チェックポイントのネットをfp32のevalモードで128局面ずつ推論し、
主の価値のlogitと手番側の結果の交差エントロピー、および指した手との一致を求める。
一致は合法手の最大logitのうち最小のラベルを選んで判定する。

値はfloat64で1局の中で平均してから、局面を持つ対局の間で単純に平均する。
手数はplies + 1とし、1〜40、41〜100、101〜200、201手目以降の4帯でも同様に、
その帯に局面を持つ対局の中と対局の間で平均する。空の帯は件数0、値nullとする。

JSONにはチェックポイントのSHA-256とネットの規模、データの正規化したパス・大きさ・
SHA-256・規則セット名・生成コミット・基本シード・対局数、機器・精度・バッチサイズを記録する。
diagnosticsとbandsには対局数・局面数・価値の損失・一致率を記録し、bandsには手数の
上下限も記録する。elapsed_secondsは読み込みと検査和計算を含む開始から診断完了までの
秒数で、JSONの書き込みは含まない。既存の出力先は上書きせず、チェックポイントは変更しない。
"""

import json
from pathlib import Path
import time

import minase_mnsd_reader
import numpy as np
from numpy.typing import NDArray
import torch
from torch.nn import functional as F

from .data import _dataset, _file_info
from .encoding import policy_mask
from .train import _batch, _legal_batch, _load_checkpoint, _model, _policy_diagnostics_per_position, _sha256, _validate_device


def _average_games(games: NDArray[np.uint32], values: NDArray[np.float64]) -> dict:
    """局面を持つ対局を等しい重みで平均し、空集合では件数0と値Noneを返す。"""
    numbers, indices, counts = np.unique(games, return_inverse=True, return_counts=True)
    summary = {"game_count": len(numbers), "position_count": len(games)}
    for name, column in zip(("value_result", "policy_agreement"), values.T, strict=True):
        if len(numbers) == 0:
            summary[name] = None
        else:
            totals = np.bincount(indices, weights=column)
            summary[name] = float((totals / counts).mean())
    return summary


@torch.inference_mode()
def external_eval(checkpoint: Path, data: Path, device: str, out: Path) -> None:
    """検証用データ全体と手数の帯ごとの診断値を、来歴とともに新しいJSONへ書く。"""
    started = time.perf_counter()
    if out.exists() or out.is_symlink():
        raise FileExistsError(f"output already exists: {out}")
    path = Path(data).resolve(strict=True)
    raw = minase_mnsd_reader.read_validation(str(path))
    info = _file_info(path, raw)
    header = info.header
    if header.game_count == 0:
        raise ValueError(f"{path}: diagnostic set must contain games")
    if header.origin != 2:
        raise ValueError(f"{path}: origin must be human games")
    if not header.lishogi:
        raise ValueError(f"{path}: rules must equal lishogi")
    dataset = _dataset(raw, info)
    if len(dataset) == 0:
        raise ValueError(f"{path}: diagnostic set must contain positions")
    target = _validate_device(device, "fp32")
    checkpoint_sha256 = _sha256(checkpoint)
    data_sha256 = _sha256(path)
    state = _load_checkpoint(checkpoint)
    model = _model(state).to(device=target, dtype=torch.float32).eval()
    mask = torch.from_numpy(policy_mask()).to(target)
    config = {"precision": "fp32", "batch_size": 128}
    values = np.empty((len(dataset), 2), dtype=np.float64)
    for start in range(0, len(dataset), config["batch_size"]):
        end = min(start + config["batch_size"], len(dataset))
        batch = np.arange(start, end, dtype=np.int64)
        logits, policy, results, _ = _batch(model.forward_logits, dataset, batch, target, config)
        value_result = F.binary_cross_entropy_with_logits(logits.value.flatten(), results, reduction="none")
        legal_rows, legal_labels = _legal_batch(dataset, batch, target)
        _, agreement = _policy_diagnostics_per_position(
            logits.policy, policy.rows, policy.labels, policy.probabilities,
            legal_rows, legal_labels, mask,
        )
        if not all(torch.isfinite(column).all().item() for column in (value_result, agreement)):
            raise ValueError("non-finite diagnostic value")
        values[start:end, 0] = value_result.cpu().numpy()
        values[start:end, 1] = agreement.cpu().numpy()
    moves = dataset.plies.astype(np.int64) + 1
    bands = []
    for first, last in ((1, 40), (41, 100), (101, 200), (201, None)):
        selected = moves >= first
        if last is not None:
            selected &= moves <= last
        bands.append({"first_move": first, "last_move": last,
                      **_average_games(dataset.games[selected], values[selected])})
    summary = {
        "checkpoint_sha256": checkpoint_sha256,
        "blocks": state["config"]["blocks"], "channels": state["config"]["channels"],
        "data": {"path": str(path), "size": info.size, "sha256": data_sha256,
                 "rule_set": header.rule_set, "generation_commit": header.generation_commit,
                 "seed": header.seed, "game_count": header.game_count},
        "device": device, **config,
        "diagnostics": _average_games(dataset.games, values), "bands": bands,
        "elapsed_seconds": time.perf_counter() - started,
    }
    text = json.dumps(summary, ensure_ascii=False, allow_nan=False, indent=2) + "\n"
    with out.open("x", encoding="utf-8") as destination:
        destination.write(text)
