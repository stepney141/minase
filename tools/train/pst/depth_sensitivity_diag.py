"""段階9の診断A: 層別抽出と対局単位のブートストラップ（NumPyのみ）。

planで重みを固定し、sampleで予備・本番を別々に抽出する。
estimateは両探索が通常値を返した局面だけで深い探索の感度を推定する。
通し番号は元MNSD内の0始まり。対局の識別子は(seed, game)。
不足した層は両群の小さい方にそろえる。詰み割合の分母とn_pilotは
通常値による除外前の抽出数。推定不能な再標本は数えて区間計算から除く。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

import numpy as np

from features import COLOR_BY_BYTE, PIECE_STATE_BY_BYTE
from mnsd import hash64, map_records, read_header


MNKF_HEADER = struct.Struct("<4sIIIQ32s")
MNRS_DTYPE = np.dtype([
    ("status", "u1"), ("tactical", "u1"), ("score", "<i2"),
    ("depth", "<u4"), ("nodes", "<u8"),
])
GROUPS = ("exposed", "control")
SPLITS = ("pilot", "main")
SCORE_BOUNDS = (-500, -100, 100, 500)
CHUNK_SIZE = 65536


def checksum(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2,
                                    allow_nan=False) + "\n", encoding="utf-8")


def read_json(path, kind):
    value = json.loads(Path(path).read_text(encoding="utf-8"))
    if value["format"] != f"depth-sensitivity-{kind}" or value["version"] != 1:
        raise ValueError(f"{path}: unsupported {kind} format/version")
    return value


def load_sources(data, king_features=None):
    paths = [Path(path).resolve() for path in data]
    if not paths or len(set(paths)) != len(paths):
        raise ValueError("MNSD inputs must be nonempty and distinct")
    if len({path.name for path in paths}) != len(paths):
        raise ValueError("MNSD basenames must be distinct for target filenames")
    if king_features is not None and len(king_features) != len(paths):
        raise ValueError("one MNKF file is required per MNSD")
    sources, generations, checksums = [], [], set()
    for i, path in enumerate(paths):
        header = read_header(path)
        digest = checksum(path)
        if digest in checksums:
            raise ValueError("duplicate MNSD content")
        checksums.add(digest)
        if header.network_checksum not in generations:
            generations.append(header.network_checksum)
        if len(generations) > 3:
            raise ValueError("diagnostic A supports at most three generations")
        meta = dict(file=str(path), sha256=digest, record_count=header.record_count,
                    seed=header.seed, network_checksum=header.network_checksum.hex(),
                    generation=generations.index(header.network_checksum))
        source = dict(meta=meta, header=header, records=map_records(path))
        if king_features is not None:
            feature_path = Path(king_features[i]).resolve()
            with feature_path.open("rb") as stream:
                raw = stream.read(MNKF_HEADER.size)
            if len(raw) != MNKF_HEADER.size:
                raise ValueError(f"{feature_path}: truncated MNKF header")
            magic, version, definition, columns, count, source_hash = MNKF_HEADER.unpack(raw)
            if (magic, version, definition, columns) != (b"MNKF", 1, 2, 118):
                raise ValueError(f"{feature_path}: expected MNKF version 1, definition 2, 118 columns")
            if count != header.record_count or source_hash.hex() != digest:
                raise ValueError(f"{feature_path}: MNKF source checksum/count mismatch")
            if feature_path.stat().st_size != MNKF_HEADER.size + count * columns:
                raise ValueError(f"{feature_path}: MNKF length mismatch")
            meta.update(king_features=str(feature_path), king_features_sha256=checksum(feature_path))
            source["features"] = np.memmap(feature_path, mode="r", dtype="u1",
                                            offset=MNKF_HEADER.size, shape=(count, columns))
        sources.append(source)
    return sources


def classify(records, generation):
    """層番号と相互排他的な除外理由を返す。王将=11、太子=21。"""
    board, stm = records["board"], records["stm"]
    states = PIECE_STATE_BY_BYTE[board]
    royals = (states == 11) | (states == 21)
    own = COLOR_BY_BYTE[board] == stm[:, None]
    own_count = np.count_nonzero(royals & own, axis=1)
    opponent_count = np.count_nonzero(royals & ~own, axis=1)
    midgame = np.count_nonzero(board, axis=1) >= 47
    valid_own = own_count == 1
    valid_opponent = (opponent_count == 1) | (opponent_count == 2)
    strata = ((generation * 2 + stm.astype(int)) * 2 + opponent_count - 1) * 5
    strata += np.searchsorted(SCORE_BOUNDS, records["score"], side="right")
    return strata, midgame, valid_own, valid_opponent


def scan(sources):
    for file_index, source in enumerate(sources):
        for start in range(0, source["header"].record_count, CHUNK_SIZE):
            rows = source["records"][start:start + CHUNK_SIZE]
            hashes = hash64(source["header"].seed, rows["game"])
            train = hashes % np.uint64(20) != 0
            strata, midgame, own, opponent = classify(rows, source["meta"]["generation"])
            exclusions = dict(validation=int(np.count_nonzero(~train)),
                              below_47_pieces=int(np.count_nonzero(train & ~midgame)),
                              own_royals_not_one=int(np.count_nonzero(train & midgame & ~own)),
                              opponent_royals_not_one_or_two=int(np.count_nonzero(train & midgame & own & ~opponent)))
            eligible = train & midgame & own & opponent
            exposed = np.any(source["features"][start:start + len(rows), 6:12] != 0, axis=1)
            yield file_index, start, rows, strata, eligible, exposed, hashes % np.uint64(2), exclusions


def make_plan(data, king_features):
    sources = load_sources(data, king_features)
    count = (max(s["meta"]["generation"] for s in sources) + 1) * 20
    positions = np.zeros((count, 2, 2), dtype=np.int64)
    games = [[[set() for _ in GROUPS] for _ in SPLITS] for _ in range(count)]
    excluded = {key: 0 for key in ("validation", "below_47_pieces", "own_royals_not_one", "opponent_royals_not_one_or_two")}
    files = [dict(s["meta"], eligible_count=0, exclusions=dict(excluded)) for s in sources]
    for f, start, rows, strata, eligible, exposed, split, exclusions in scan(sources):
        files[f]["eligible_count"] += int(np.count_nonzero(eligible))
        for key, value in exclusions.items():
            excluded[key] += value
            files[f]["exclusions"][key] += value
        for h in np.unique(strata[eligible]):
            for p in range(2):
                for g in range(2):
                    mask = eligible & (strata == h) & (split == p) & (exposed == (g == 0))
                    positions[h, p, g] += np.count_nonzero(mask)
                    games[h][p][g].update((sources[f]["header"].seed, int(game))
                                          for game in np.unique(rows["game"][mask]))
    exposed_total = int(positions[:, :, 0].sum())
    if exposed_total == 0:
        raise ValueError("no eligible exposed positions; stratum weights are undefined")
    strata = []
    for h in range(count):
        strata.append(dict(
            id=h, generation=h // 20, stm=(h // 10) % 2,
            opponent_royals=(h // 5) % 2 + 1, score_bin=h % 5,
            w_h=float(positions[h, :, 0].sum() / exposed_total),
            counts={group: dict(positions=int(positions[h, :, g].sum()),
                                games=len(games[h][0][g] | games[h][1][g]))
                    for g, group in enumerate(GROUPS)},
            splits={name: {group: dict(positions=int(positions[h, p, g]), games=len(games[h][p][g]))
                           for g, group in enumerate(GROUPS)} for p, name in enumerate(SPLITS)}))
    return dict(format="depth-sensitivity-plan", version=1, files=files,
                exclusions=excluded, exposed_total=exposed_total, strata=strata,
                score_bins=["[-inf,-500)", "[-500,-100)", "[-100,100)", "[100,500)", "[500,inf)"],
                game_identity=["seed", "game"])


def weights_from(document):
    strata = document["strata"]
    if not strata or [h["id"] for h in strata] != list(range(len(strata))):
        raise ValueError("strata must have consecutive IDs")
    weights = np.array([h["w_h"] for h in strata], dtype=float)
    if not np.all(np.isfinite(weights)) or np.any(weights < 0) or not np.isclose(weights.sum(), 1):
        raise ValueError("stratum weights must be nonnegative and sum to one")
    return weights


def allocate(n, weights):
    """最大剰余法。同じ剰余は層番号順に割り当てる。"""
    quotas = n * weights
    result = np.floor(quotas).astype(np.int64)
    remaining = n - int(result.sum())
    result[np.argsort(-(quotas - result), kind="stable")[:remaining]] += 1
    return result


def sample_positions(plan, data, king_features, split, per_group, seed, output_dir):
    if split not in SPLITS or per_group <= 0 or seed < 0:
        raise ValueError("invalid split, per-group count, or seed")
    weights = weights_from(plan)
    sources = load_sources(data, king_features)
    if len(sources) != len(plan["files"]):
        raise ValueError("plan/input file count mismatch")
    for source, saved in zip(sources, plan["files"]):
        if any(saved[key] != value for key, value in source["meta"].items()):
            raise ValueError("plan/input metadata mismatch")
    requested = allocate(per_group, weights)
    rng = np.random.default_rng(seed)
    selections, allocations = {}, []
    for h, quota in enumerate(requested):
        available = [plan["strata"][h]["splits"][split][g]["positions"] for g in GROUPS]
        take = min(int(quota), *available)
        allocations.append(dict(stratum=h, requested_per_group=int(quota), selected_per_group=take,
                                shortage_per_group=int(quota) - take, available=dict(zip(GROUPS, available))))
        for g in range(2):
            selections[h, g] = np.sort(rng.choice(available[g], size=take, replace=False))
    seen = np.zeros((len(weights), 2), dtype=np.int64)
    selected = []
    for f, start, rows, strata, eligible, exposed, partitions, _ in scan(sources):
        eligible &= partitions == SPLITS.index(split)
        for h in np.unique(strata[eligible]):
            for g, group in enumerate(GROUPS):
                local = np.flatnonzero(eligible & (strata == h) & (exposed == (g == 0)))
                choices = selections[h, g]
                left, right = np.searchsorted(choices, [seen[h, g], seen[h, g] + len(local)])
                for row in local[choices[left:right] - seen[h, g]]:
                    selected.append(dict(file=sources[f]["meta"]["file"], index=int(start + row),
                                         stratum=int(h), group=group, game=int(rows["game"][row]),
                                         data_seed=sources[f]["header"].seed, seed=seed, split=split))
                seen[h, g] += len(local)
    expected = np.array([[h["splits"][split][g]["positions"] for g in GROUPS] for h in plan["strata"]])
    if not np.array_equal(seen, expected):
        raise ValueError("plan counts no longer match inputs")
    file_order = {s["meta"]["file"]: i for i, s in enumerate(sources)}
    selected.sort(key=lambda row: (file_order[row["file"]], row["index"]))
    result = dict(format="depth-sensitivity-sample", version=1, seed=seed, split=split,
                  requested_per_group=per_group, selected_per_group=sum(a["selected_per_group"] for a in allocations),
                  files=[s["meta"] for s in sources], strata=plan["strata"], allocations=allocations, positions=selected)
    output = Path(output_dir)
    output.mkdir(parents=True, exist_ok=True)
    for source in sources:
        path = source["meta"]["file"]
        indices = [row["index"] for row in selected if row["file"] == path]
        (output / (Path(path).name + ".targets.txt")).write_text("".join(f"{i}\n" for i in indices), encoding="ascii")
    write_json(output / "sample.json", result)
    return result


def read_rescore(path, source, targets, node_limit):
    path = Path(path)
    count = source["header"].record_count
    with path.open("rb") as stream:
        raw = stream.read(240)
    if len(raw) != 240 or path.stat().st_size != 240 + 16 * count:
        raise ValueError(f"{path}: MNRS length mismatch or incomplete file")
    if struct.unpack_from("<4sI", raw) != (b"MNRS", 1):
        raise ValueError(f"{path}: unsupported MNRS format/version")
    if raw[8:40].hex() != source["meta"]["sha256"] or struct.unpack_from("<Q", raw, 40)[0] != count:
        raise ValueError(f"{path}: MNRS source checksum/count mismatch")
    digest = hashlib.sha256(np.asarray(targets, dtype="<u8").tobytes()).digest()
    if raw[48:80] != digest or struct.unpack_from("<Q", raw, 80)[0] != len(targets):
        raise ValueError(f"{path}: MNRS sample targets mismatch")
    if struct.unpack_from("<I", raw, 120)[0] != node_limit:
        raise ValueError(f"{path}: wrong search node limit")
    rule = source["header"].rule_set.encode("utf-8").ljust(32, b"\0")
    if raw[124:156] != rule or struct.unpack_from("<I", raw, 156)[0] != 16 or raw[160] != 0:
        raise ValueError(f"{path}: wrong rule set, hash size, or standalone search condition")
    if any(raw[161:164]) or any(raw[236:240]) or any(c not in b"0123456789abcdefABCDEF" for c in raw[164:204]):
        raise ValueError(f"{path}: invalid MNRS reserved bytes or commit")
    records = np.memmap(path, mode="r", dtype=MNRS_DTYPE, offset=240, shape=(count,))
    actual_digest, actual_count = hashlib.sha256(), 0
    for start in range(0, count, CHUNK_SIZE):
        rows = records[start:start + CHUNK_SIZE]
        status = rows["status"]
        if (np.any(status > 2) or np.any(rows["tactical"] > 1)
                or np.any((status != 1) & ((rows["score"] != 0) | (rows["depth"] != 0) | (rows["tactical"] != 0)))
                or np.any((status == 0) & (rows["nodes"] != 0))
                or np.any((status == 1) & (rows["depth"] == 0))):
            raise ValueError(f"{path}: invalid MNRS record")
        actual = np.flatnonzero(status != 0).astype("<u8") + start
        actual_digest.update(actual.tobytes())
        actual_count += len(actual)
    if actual_count != len(targets) or actual_digest.digest() != digest:
        raise ValueError(f"{path}: target records must all have status 1 or 2")
    return raw, records[np.asarray(targets, dtype=np.int64)]


def weighted_estimate(differences, bins, weights, multiplicity):
    """群・層の集計を毎回作り直し、有効な層だけで重みを正規化する。"""
    counts = np.bincount(bins, weights=multiplicity, minlength=len(weights) * 2).reshape(-1, 2)
    sums = np.bincount(bins, weights=multiplicity * differences,
                       minlength=len(weights) * 2).reshape(-1, 2)
    retained = (counts > 0).all(axis=1) & (weights > 0)
    mass = float(weights[retained].sum())
    if mass == 0:
        return None, retained, counts
    means = sums[retained] / counts[retained]
    value = np.sum(weights[retained] * (means[:, 0] - means[:, 1])) / mass
    return float(value), retained, counts


def distribution(values):
    if not len(values):
        return dict(count=0, mean=None, min=None, p05=None, median=None, p95=None, max=None)
    quantiles = np.percentile(values, [5, 50, 95])
    return dict(count=len(values), mean=float(np.mean(values)), min=int(np.min(values)),
                p05=float(quantiles[0]), median=float(quantiles[1]), p95=float(quantiles[2]), max=int(np.max(values)))


def estimate(sample, shallow, deep, data, bootstrap=2000, seed=1):
    if bootstrap < 2 or seed < 0:
        raise ValueError("bootstrap must be at least 2 and seed nonnegative")
    weights = weights_from(sample)
    sources = load_sources(data)
    if not (len(sources) == len(sample["files"]) == len(shallow) == len(deep)):
        raise ValueError("sample, MNSD, shallow, and deep file counts must match")
    if sample["split"] not in SPLITS:
        raise ValueError("invalid sample split")
    by_file = {s["meta"]["file"]: [] for s in sources}
    seen = set()
    for row in sample["positions"]:
        key = row["file"], row["index"]
        if row["file"] not in by_file or key in seen or type(row["index"]) is not int or row["index"] < 0:
            raise ValueError("sample contains an unknown file, invalid index, or duplicate position")
        if row["group"] not in GROUPS or row["split"] != sample["split"] or row["seed"] != sample["seed"]:
            raise ValueError("sample group, split, or seed mismatch")
        seen.add(key)
        by_file[row["file"]].append(row)
    rows_all, low, high, provenance = [], [], [], []
    for source, saved, low_path, high_path in zip(sources, sample["files"], shallow, deep):
        if any(saved[key] != value for key, value in source["meta"].items()):
            raise ValueError("sample/input metadata mismatch")
        rows = sorted(by_file[source["meta"]["file"]], key=lambda row: row["index"])
        targets = [row["index"] for row in rows]
        if targets and targets[-1] >= source["header"].record_count:
            raise ValueError("sample index outside MNSD")
        originals = source["records"][np.asarray(targets, dtype=np.int64)]
        strata, midgame, own, opponent = classify(originals, source["meta"]["generation"])
        hashes = hash64(source["header"].seed, originals["game"])
        if not np.all(midgame & own & opponent & (hashes % np.uint64(20) != 0)
                      & (hashes % np.uint64(2) == SPLITS.index(sample["split"]))):
            raise ValueError("sample includes ineligible positions")
        for row, original, h in zip(rows, originals, strata):
            if (row["stratum"] != h or row["game"] != int(original["game"])
                    or row["data_seed"] != source["header"].seed):
                raise ValueError("sample stratum or game identity mismatch")
        low_header, low_rows = read_rescore(low_path, source, targets, 100_000)
        high_header, high_rows = read_rescore(high_path, source, targets, 10_000_000)
        if low_header[88:120] != high_header[88:120] or low_header[124:] != high_header[124:]:
            raise ValueError("shallow/deep teacher or execution conditions differ")
        provenance.append(dict(file=source["meta"]["file"], shallow=str(Path(low_path).resolve()),
                               deep=str(Path(high_path).resolve()), network_checksum=low_header[88:120].hex(),
                               generation_commit=low_header[164:204].decode("ascii"), binary_sha256=low_header[204:236].hex()))
        rows_all.extend(rows)
        low.append(low_rows)
        high.append(high_rows)
    low, high = np.concatenate(low), np.concatenate(high)
    if not rows_all:
        raise ValueError("sample has no positions")
    group = np.array([GROUPS.index(row["group"]) for row in rows_all])
    n_per_group = [int(np.count_nonzero(group == g)) for g in range(2)]
    if n_per_group[0] != n_per_group[1] or n_per_group[0] != sample["selected_per_group"]:
        raise ValueError("sample must have the recorded equal number in each group")
    strata = np.array([row["stratum"] for row in rows_all], dtype=int)
    if np.any(strata < 0) or np.any(strata >= len(weights)):
        raise ValueError("sample stratum outside plan")
    sampled_counts = np.bincount(strata * 2 + group, minlength=len(weights) * 2).reshape(-1, 2)
    if np.any(sampled_counts[:, 0] != sampled_counts[:, 1]):
        raise ValueError("sample must have equal group counts within each stratum")
    ordinary_low = (low["status"] == 1) & (np.abs(low["score"].astype(int)) < 29000)
    ordinary_high = (high["status"] == 1) & (np.abs(high["score"].astype(int)) < 29000)
    valid = ordinary_low & ordinary_high
    differences = high["score"].astype(float)[valid] - low["score"].astype(float)[valid]
    bins = strata[valid] * 2 + group[valid]
    value, retained, counts = weighted_estimate(differences, bins, weights, np.ones(len(bins)))
    if value is None:
        raise ValueError("no positive-weight stratum retains both ordinary-score groups")
    game_keys = [(row["data_seed"], row["game"]) for row in rows_all]
    game_numbers = {key: i for i, key in enumerate(sorted(set(game_keys)))}
    games = np.array([game_numbers[key] for key in game_keys])[valid]
    game_count = len(game_numbers)
    rng = np.random.default_rng(seed)
    replicates, dropped_masses = [], []
    for _ in range(bootstrap):
        multiplicity = np.bincount(rng.integers(game_count, size=game_count), minlength=game_count)
        result, kept, _ = weighted_estimate(differences, bins, weights, multiplicity[games])
        dropped_masses.append(float(weights[~kept].sum()))
        if result is not None:
            replicates.append(result)
    if len(replicates) < 2:
        raise ValueError("fewer than two estimable bootstrap replicates")
    lower, upper = np.percentile(replicates, [5, 95])
    se = float(np.std(replicates, ddof=1))
    summaries = {}
    for g, name in enumerate(GROUPS):
        mask = group == g
        mate = {}
        for sign, positive in (("win", True), ("loss", False)):
            low_mate = (low["status"] == 1) & ((low["score"] >= 29000) if positive else (low["score"] <= -29000))
            high_mate = (high["status"] == 1) & ((high["score"] >= 29000) if positive else (high["score"] <= -29000))
            mate[sign] = {label: dict(count=int(np.count_nonzero(mask & selected)),
                                      fraction=float(np.count_nonzero(mask & selected) / n_per_group[g]))
                          for label, selected in (("either", low_mate | high_mate), ("shallow", low_mate),
                                                  ("deep", high_mate), ("shallow_ordinary_to_deep", ordinary_low & high_mate))}
        summaries[name] = dict(sampled=n_per_group[g], ordinary=int(np.count_nonzero(mask & valid)),
                               excluded=int(np.count_nonzero(mask & ~valid)), mate=mate,
                               incomplete={label: int(np.count_nonzero(mask & (entries["status"] == 2)))
                                           for label, entries in (("shallow", low), ("deep", high))})
    normalized = np.where(retained, weights / weights[retained].sum(), 0)
    result = dict(format="depth-sensitivity-estimate", version=1, split=sample["split"], seed=seed,
                  D=value, standard_error=se, upper_one_sided_95=float(upper),
                  interval_two_sided_90=[float(lower), float(upper)],
                  criterion_met=bool(upper <= -30), decision="基準を満たす" if upper <= -30 else "満たさない",
                  dropped_weight=float(weights[~retained].sum()),
                  strata=[dict(id=h, w_h=float(weights[h]), normalized_weight=float(normalized[h]),
                               retained=bool(retained[h]), ordinary_counts=dict(zip(GROUPS, map(int, counts[h]))))
                          for h in range(len(weights))],
                  bootstrap=dict(requested=bootstrap, valid=len(replicates), undefined=bootstrap - len(replicates),
                                 undefined_policy="omit_and_report", games=game_count,
                                 dropped_weight_mean=float(np.mean(dropped_masses)),
                                 dropped_weight_max=float(max(dropped_masses))),
                  groups=summaries, rescores=provenance,
                  search={label: dict(nodes=distribution(entries["nodes"]), depth=distribution(entries["depth"]),
                                      by_group={name: dict(nodes=distribution(entries["nodes"][group == g]),
                                                           depth=distribution(entries["depth"][group == g]))
                                                for g, name in enumerate(GROUPS)})
                          for label, entries in (("shallow", low), ("deep", high))})
    if sample["split"] == "pilot":
        result.update(n_pilot=n_per_group[0], n_main=math.ceil(n_per_group[0] * (se / 12) ** 2))
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("plan", "sample", "estimate"):
        command = commands.add_parser(name)
        command.add_argument("--data", nargs="+", required=True)
        if name in ("plan", "sample"):
            command.add_argument("--king-features", nargs="+", required=True)
        if name in ("sample", "estimate"):
            command.add_argument("--seed", type=int, default=1)
        if name in ("plan", "estimate"):
            command.add_argument("--output", required=True)
        if name == "sample":
            command.add_argument("--plan", required=True)
            command.add_argument("--split", choices=SPLITS, required=True)
            command.add_argument("--per-group", type=int, required=True)
            command.add_argument("--output-dir", required=True)
        if name == "estimate":
            command.add_argument("--sample", required=True)
            command.add_argument("--shallow", nargs="+", required=True)
            command.add_argument("--deep", nargs="+", required=True)
            command.add_argument("--bootstrap", type=int, default=2000)
    args = parser.parse_args(argv)
    try:
        if args.command == "plan":
            write_json(args.output, make_plan(args.data, args.king_features))
        elif args.command == "sample":
            sample_positions(read_json(args.plan, "plan"), args.data, args.king_features,
                             args.split, args.per_group, args.seed, args.output_dir)
        else:
            write_json(args.output, estimate(read_json(args.sample, "sample"), args.shallow,
                                            args.deep, args.data, args.bootstrap, args.seed))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
