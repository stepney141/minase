"""生成と無関係なαβの棋譜から対局を固定抽出し、学習せずに診断する。

母集団は入力ファイル順、ファイル内では対局番号順に並べた対局であり、選別後の
局面が0件の対局も含む。Generator(PCG64(seed)).choiceで重複なく抽出し、選んだ
対局を母集団の順に並べる。sample_sha256は、その列の各シードをu64、対局番号を
u32としてリトルエンディアンで連結したバイト列のSHA-256である。
同じシードと対局番号の重複は、別ファイルにあってもエラーとする。

入力には学習器と同じ乱数開始の自己対局とengine-defaultの規則を要求する。
scanで全対局を数えた後、選択対局を含むファイルを1つずつ再生して、選んだ対局の
局面だけをread_gamesで展開する。選択外の対局も再生して検査するが、盤面・方策・
合法手の返却配列には保持しない。診断後に配列を破棄し、次のファイルを読む。

診断はtrain.diagnoseをfp32、128局面ずつ、value_lambda=0、auxiliary=Falseで呼ぶ。αβの評価値を
価値の教師に混ぜず、valueは出力しない。ファイル間も局面数で重み付けして平均する。
gamesは抽出した対局数、diagnostics.game_countは採用局面を持つ対局数であり、
採用局面が全体で0件ならエラーとする。elapsed_secondsは開始から読み込み、
検査和計算、診断の完了までの秒数で、JSONの書き込みは含まない。
"""

from collections.abc import Sequence
import hashlib
import json
from pathlib import Path
import struct
import time

import minase_mnsd_reader
import numpy as np
import torch

from .data import Dataset, FileInfo, _dataset, _file_info
from .encoding import policy_mask
from .train import POLICY_DIAGNOSTIC_NAMES, _load_checkpoint, _model, _sha256, _validate_device, diagnose


def _read_games(info: FileInfo, games: list[int]) -> Dataset:
    """1ファイルを再生し、指定対局の局面と疎な方策・合法手だけを保持する。"""
    raw = minase_mnsd_reader.read_games(str(info.path), {(info.header.seed, game) for game in games})
    if _file_info(info.path, raw, teacher_kind=0) != info:
        raise ValueError(f"{info.path}: file metadata changed between counting and replay")
    return _dataset(raw, info)


def external_eval(
    checkpoint: Path, data: Sequence[str | Path], games: int, seed: int, device: str, out: Path,
) -> None:
    """固定標本の来歴と診断値を1つのJSONへ書く。既存の出力先は上書きしない。"""
    started = time.perf_counter()
    if out.exists() or out.is_symlink():
        raise FileExistsError(f"output already exists: {out}")
    if not data:
        raise ValueError("at least one MNSD file is required")
    if games <= 0:
        raise ValueError("games must be positive")
    if not 0 <= seed <= 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("seed must fit in uint64")
    target = _validate_device(device, "fp32")
    infos = []
    population = []
    seen = set()
    for path in data:
        path = Path(path).resolve(strict=True)
        info = _file_info(path, minase_mnsd_reader.scan(str(path)), teacher_kind=0)
        infos.append(info)
        for game in sorted(info.game_numbers):
            key = (info.header.seed, game)
            if key in seen:
                raise ValueError(f"{path}: duplicate game (seed={key[0]}, game={key[1]})")
            seen.add(key)
            population.append(key)
    if games > len(population):
        raise ValueError(f"requested {games} games, but only {len(population)} are available")
    rng = np.random.Generator(np.random.PCG64(seed))
    selected = [population[int(index)] for index in np.sort(rng.choice(len(population), games, replace=False))]
    selected_set = set(selected)
    sample_sha256 = hashlib.sha256(b"".join(struct.pack("<QI", *key) for key in selected)).hexdigest()
    files = [
        {"path": str(path), "size": info.size, "sha256": _sha256(info.path)}
        for path, info in zip(data, infos, strict=True)
    ]
    checkpoint_sha256 = _sha256(checkpoint)
    state = _load_checkpoint(checkpoint)
    model = _model(state).to(target)
    mask = torch.from_numpy(policy_mask()).to(target)
    config = {"precision": "fp32", "batch_size": 128, "value_lambda": 0.0}
    sums = dict.fromkeys(("policy", "value_result", *POLICY_DIAGNOSTIC_NAMES), 0.0)
    game_count = position_count = 0
    for info in infos:
        numbers = [game for game in info.game_numbers if (info.header.seed, game) in selected_set]
        if not numbers:
            continue
        dataset = _read_games(info, numbers)
        if len(dataset) != 0:
            metrics = diagnose(model, dataset, np.arange(len(dataset)), target, config, mask, auxiliary=False)
            for name in sums:
                sums[name] += metrics[name] * len(dataset)
            position_count += len(dataset)
            game_count += len(np.unique(dataset.games))
        del dataset
    if position_count == 0:
        raise ValueError("diagnostic set must contain positions")
    summary = {
        "checkpoint_sha256": checkpoint_sha256,
        "blocks": state["config"]["blocks"], "channels": state["config"]["channels"],
        "files": files, "games": games, "seed": seed, "device": device,
        "selected_games": [{"seed": base_seed, "game": number} for base_seed, number in selected],
        "sample_sha256": sample_sha256,
        "diagnostics": {"game_count": game_count, "position_count": position_count,
                        **{name: value / position_count for name, value in sums.items()}},
        "elapsed_seconds": time.perf_counter() - started,
    }
    text = json.dumps(summary, ensure_ascii=False, allow_nan=False, indent=2) + "\n"
    with out.open("x", encoding="utf-8") as destination:
        destination.write(text)
