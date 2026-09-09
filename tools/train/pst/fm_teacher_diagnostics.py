"""FMとPSTの候補手比較に使う層別標本と、教師探索の診断集計。"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import subprocess

import numpy as np

from fm_strength_diagnostics import sha256, statistics
from mnsd import HEADER_LENGTH, RECORD_DTYPE, map_records, read_header, write_mnsd
from taper import band_indices, phase_ratios


def paired_sample(saved: list[dict], match: list[dict], count: int, seed: int) -> list[tuple[dict, dict]]:
    """対局を2巡し、同じ層の保存局面と組にする。各元対局は最大2局面。"""
    rng = np.random.default_rng(seed)
    pools = defaultdict(list)
    games = defaultdict(list)
    for row in saved:
        pools[tuple(row["stratum"])].append(row)
    for row in match:
        games[row["game_key"]].append(row)
    for group in list(pools.values()) + list(games.values()):
        rng.shuffle(group)
    game_order = sorted(games)
    rng.shuffle(game_order)
    saved_used = Counter()
    chosen_saved, chosen_match = set(), set()
    result = []
    for _ in range(2):
        for game in game_order:
            for right in games[game]:
                if right["source_index"] in chosen_match:
                    continue
                left = next((row for row in pools[tuple(right["stratum"])]
                             if saved_used[row["game_key"]] < 2
                             and row["source_index"] not in chosen_saved), None)
                if left is None:
                    continue
                result.append((left, right))
                saved_used[left["game_key"]] += 1
                chosen_saved.add(left["source_index"])
                chosen_match.add(right["source_index"])
                break
            if len(result) == count:
                return result
    return result


def diagnostic_rows(directory: Path) -> tuple[np.ndarray, list[dict]]:
    records = map_records(directory / "sample.bin")
    rows = json.loads((directory / "probe.json").read_text())
    report = json.loads((directory / "report.json").read_text())
    if [row["index"] for row in rows] != list(range(len(records))):
        raise ValueError("probe order does not match sample")
    if report["sample_indices"] != list(range(len(records))):
        raise ValueError("prepare requires complete static diagnostic samples")
    bands = band_indices(phase_ratios(records["board"]))
    central = [{"source_index": i, "stratum": [int(bands[i]), row["eval_pst"] // 500],
                "pst_eval": row["eval_pst"]}
               for i, row in enumerate(rows) if -1000 <= row["eval_pst"] < 1000]
    return records, central


def saved_origins(archive: Path, records: np.ndarray) -> tuple[list[dict], list[dict]]:
    settings = json.loads((archive / "settings.json").read_text())
    files = [Path(path) for path in settings["run"]["data"]]
    union = np.sort(np.concatenate([np.load(path, allow_pickle=False)
                                   for path in sorted((archive / "diagnostics").glob("diagnostic-indices-*.npy"))]))
    if len(union) != len(records) or len(np.unique(union)) != len(union):
        raise ValueError("original diagnostic index union does not match saved sample")
    origins = [None] * len(records)
    provenance = []
    offset = 0
    for path in files:
        header = read_header(path)
        mapped = np.memmap(path, mode="r", dtype=RECORD_DTYPE, offset=HEADER_LENGTH,
                           shape=(header.record_count,))
        members = np.flatnonzero((union >= offset) & (union < offset + len(mapped)))
        local = union[members] - offset
        if not np.array_equal(mapped[local], records[members]):
            raise ValueError(f"saved record mismatch against {path}")
        identity = f"{header.generation_commit}:{header.network_checksum.hex()}:{header.seed}"
        file_info = {"path": str(path.resolve()), "sha256": sha256(path),
                     "seed": header.seed, "generation_commit": header.generation_commit,
                     "network_checksum": header.network_checksum.hex(), "records": len(mapped)}
        provenance.append(file_info)
        for sample_index, record_index in zip(members.tolist(), local.tolist()):
            original = mapped[record_index]
            origins[sample_index] = {"game_key": f"{identity}:{int(original['game'])}",
                                     "source_file": str(path.resolve()), "record_index": record_index,
                                     "original_game": int(original["game"]), "ply": int(original["ply"])}
        offset += len(mapped)
    if any(row is None for row in origins):
        raise ValueError("original index is outside source files")
    return origins, provenance


def make_teacher(candidate: Path, baseline: Path, output: Path) -> dict:
    # 共通のMNPT検証器で固定PST・駒価値・Kを検査し、FMの埋め込みだけを零にする。
    from train_fm import read_mnpt_v3, validate_fixed_base, write_mnpt_v3
    baseline_object = "b0153cf27df655a32cb9a255fd380b21bba002b5:nets/pst.bin"
    historical = subprocess.run(["git", "show", baseline_object], check=True,
                                cwd=Path(__file__).resolve().parents[3], capture_output=True).stdout
    if historical != baseline.read_bytes():
        raise ValueError("baseline differs from the fixed historical PST")
    validate_fixed_base(baseline, candidate)
    mg, eg, material, k, embedding, signs, exponent = read_mnpt_v3(candidate)
    write_mnpt_v3(output, mg, eg, material, k, np.zeros_like(embedding), signs, exponent)
    written = read_mnpt_v3(output)
    if any(not np.array_equal(left, right) for left, right in zip((mg, eg, material, k), written[:4])):
        raise ValueError("teacher changed fixed PST, material, or scale")
    if np.any(written[4]) or not np.array_equal(written[5], signs) or written[6] != exponent:
        raise ValueError("teacher must zero only FM embeddings")
    validate_fixed_base(baseline, output)
    return {"candidate_sha256": sha256(candidate), "baseline_sha256": sha256(baseline),
            "baseline_git_object": baseline_object,
            "teacher_sha256": sha256(output), "fm_embeddings_all_zero": True,
            "fixed_pst_material_scale_identical": True,
            "candidate": str(candidate.resolve()), "baseline": str(baseline.resolve()),
            "teacher": str(output.resolve())}


def prepare(args: argparse.Namespace) -> None:
    saved_dir = args.diagnostics_root / "fm-strength-saved-static-all"
    match_dir = args.diagnostics_root / "fm-strength-match-static-all"
    saved_records, saved = diagnostic_rows(saved_dir)
    match_records, match = diagnostic_rows(match_dir)
    # RULES.md:485 defines these two spellings as the same rule set.
    for directory in (saved_dir, match_dir):
        if read_header(directory / "sample.bin").rule_set not in ("L0,P0,R1,E0", "engine-default"):
            raise ValueError("teacher diagnosis requires engine-default rules")
    for directory in (saved_dir, match_dir):
        report = json.loads((directory / "report.json").read_text())
        if report["inputs"]["pst"]["sha256"] != sha256(args.candidate):
            raise ValueError("static diagnostic weights differ from candidate")
    origins, source_files = saved_origins(args.archive_run, saved_records)
    for row in saved:
        row.update(origins[row["source_index"]])
    match_meta_path = args.diagnostics_root / "fm-match-positions" / "metadata.json"
    meta = json.loads(match_meta_path.read_text())
    if len(meta["positions"]) != len(match_records):
        raise ValueError("match metadata count mismatch")
    for row in match:
        origin = meta["positions"][row["source_index"]]
        if origin["index"] != row["source_index"] or origin["game_number"] != int(match_records[row["source_index"]]["game"]):
            raise ValueError("match metadata position mismatch")
        row.update({"game_key": f"pair{origin['pair']}:game{origin['game']}",
                    "original_game": origin["game_number"], "ply": origin["ply"],
                    "pair": origin["pair"], "game_number": origin["game_number"],
                    "turn_index": origin["turn_index"], "source_sha256": origin["source_sha256"]})
    pairs = paired_sample(saved, match, args.per_cohort, args.seed)
    if len(pairs) < args.pilot_per_cohort:
        raise ValueError("constraints leave fewer samples than the requested pilot")
    args.output_dir.mkdir(parents=True, exist_ok=False)
    teacher = make_teacher(args.candidate, args.baseline, args.output_dir / "teacher-pst.bin")
    samples, selected = [], []
    game_numbers = {}
    for cohort, records, side in (("saved", saved_records, 0), ("match", match_records, 1)):
        for pair_index, pair in enumerate(pairs):
            origin = pair[side]
            key = cohort + ":" + origin["game_key"]
            if key not in game_numbers:
                game_numbers[key] = len(game_numbers) + 1
            record = records[origin["source_index"]].copy()
            record["game"] = game_numbers[key]
            selected.append(record)
            samples.append({"index": len(samples), "pair_index": pair_index, "cohort": cohort,
                            "diagnostic_game": game_numbers[key], **origin})
    selected = np.array(selected, dtype=RECORD_DTYPE)
    pilot_indices = list(range(args.pilot_per_cohort)) + list(range(len(pairs), len(pairs) + args.pilot_per_cohort))
    for filename, indices in (("positions.mnsd", list(range(len(selected)))), ("pilot.mnsd", pilot_indices)):
        write_mnsd(args.output_dir / filename, selected[indices], seed=args.seed,
                   network_checksum=args.candidate.read_bytes()[48:80],
                   rule_set="L0,P0,R1,E0")
    manifest = {"diagnostic_only": True, "seed": args.seed, "requested_per_cohort": args.per_cohort,
                "actual_per_cohort": len(pairs), "max_per_source_game": 2,
                "central_pst_interval": "[-1000,1000)", "stratum": "phase band x floor(PST/500)",
                "pilot_indices": pilot_indices, "samples": samples, "teacher": teacher,
                "source_files": source_files, "match_metadata_sha256": sha256(match_meta_path),
                "diagnostic_inputs": {cohort: {name: {"path": str((directory / name).resolve()), "sha256": sha256(directory / name)}
                                               for name in ("sample.bin", "probe.json", "report.json")}
                                      for cohort, directory in (("saved", saved_dir), ("match", match_dir))},
                "original_indices": {str(path.resolve()): sha256(path) for path in sorted((args.archive_run / "diagnostics").glob("diagnostic-indices-*.npy"))},
                "archive_settings_sha256": sha256(args.archive_run / "settings.json"),
                "selection": "two shuffled rounds over match games, pair saved within same stratum; no FM correction selection",
                "files": {name: sha256(args.output_dir / name) for name in ("positions.mnsd", "pilot.mnsd")}}
    (args.output_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({key: manifest[key] for key in ("actual_per_cohort", "teacher")}, indent=2))


def score_kind(search: dict) -> str:
    kind = search["score"]["kind"]
    if kind not in ("cp", "mate", "terminal", "incomplete"):
        raise ValueError(f"unknown score kind {kind}")
    return kind


def parent_metrics(row: dict, budget: int) -> dict:
    """JSONLの親視点スコアをそのまま使い、FM選択の相対損失を返す。"""
    proposals = row["proposals"]
    fm, pst = proposals["fm"]["best_move"], proposals["pst"]["best_move"]
    selected_children = [child for child in row["children"] if child["budget"] == budget]
    children = {child["move"]: child["search"] for child in selected_children}
    if len(children) != len(selected_children):
        raise ValueError("duplicate child move at teacher budget")
    if len(children) != len(row["candidates"]) or set(children) != set(row["candidates"]):
        raise ValueError("child candidate coverage mismatch")
    kinds = Counter(score_kind(search) for search in children.values())
    result = {"proposal_same": fm is not None and fm == pst,
              "proposal_valid": fm is not None and pst is not None and all(
                  score_kind(proposals[name]) != "incomplete" for name in ("fm", "pst")),
              "child_score_kinds": dict(kinds), "gap": None, "candidate_set_losses": None}
    if not result["proposal_valid"]:
        return result
    if fm not in children or pst not in children:
        raise ValueError("proposal move missing from teacher children")
    if score_kind(children[fm]) != "cp" or score_kind(children[pst]) != "cp":
        return result
    fm_score = children[fm]["score"]["value"]
    pst_score = children[pst]["score"]["value"]
    result["gap"] = pst_score - fm_score
    if all(score_kind(search) == "cp" for search in children.values()):
        best = max(search["score"]["value"] for search in children.values())
        result["candidate_set_losses"] = {"fm": best - fm_score, "pst": best - pst_score}
    return result


def summarize_parents(items: list[dict]) -> dict:
    cp = [item for item in items if item["gap"] is not None]
    by_game = defaultdict(list)
    for item in cp:
        by_game[item["game_key"]].append(item["gap"])
    gaps = [item["gap"] for item in cp]
    valid = [item for item in items if item["proposal_valid"]]
    return {"parents": len(items), "cp_parents": len(cp),
            "proposal_valid_parents": len(valid),
            "same_proposal_count": sum(item["proposal_same"] for item in valid),
            "same_proposal_fraction": (sum(item["proposal_same"] for item in valid) / len(valid)) if valid else None,
            "parent_equal_gap": statistics(gaps),
            "game_equal_gap": statistics([float(np.mean(values)) for values in by_game.values()]),
            "positive_gap_count": sum(value > 0 for value in gaps),
            "zero_gap_count": sum(value == 0 for value in gaps),
            "negative_gap_count": sum(value < 0 for value in gaps),
            "gap_at_least_100cp_count": sum(value >= 100 for value in gaps),
            "gap_at_most_minus100cp_count": sum(value <= -100 for value in gaps),
            "child_score_kinds": dict(sum((Counter(item["child_score_kinds"]) for item in items), Counter()))}


def budget_stability(row: dict, first: int, second: int) -> dict:
    left, right = parent_metrics(row, first), parent_metrics(row, second)
    children = {budget: {child["move"]: child["search"] for child in row["children"] if child["budget"] == budget}
                for budget in (first, second)}
    candidates = row["candidates"]
    cp_moves = [move for move in candidates if all(score_kind(children[b][move]) == "cp" for b in (first, second))]
    agreements = []
    for i, move in enumerate(cp_moves):
        for other in cp_moves[i + 1:]:
            relations = [int(np.sign(children[b][move]["score"]["value"] - children[b][other]["score"]["value"]))
                         for b in (first, second)]
            agreements.append(relations[0] == relations[1])
    gap_available = left["gap"] is not None and right["gap"] is not None
    top_sets = []
    if len(cp_moves) == len(candidates):
        for budget in (first, second):
            best = max(children[budget][move]["score"]["value"] for move in cp_moves)
            top_sets.append({move for move in cp_moves if children[budget][move]["score"]["value"] == best})
    return {"first_budget": first, "second_budget": second,
            "gap_sign_stable": bool(np.sign(left["gap"]) == np.sign(right["gap"])) if gap_available else None,
            "gap_change": right["gap"] - left["gap"] if gap_available else None,
            "strict_gap_reversal": left["gap"] * right["gap"] < 0 if gap_available else None,
            "zero_nonzero_change": (left["gap"] == 0) != (right["gap"] == 0) if gap_available else None,
            "top_set_changed": top_sets[0] != top_sets[1] if top_sets else None,
            "top_sets": [sorted(moves) for moves in top_sets],
            "cp_common_moves": len(cp_moves), "candidate_count": len(candidates),
            "rank_pair_count": len(agreements), "rank_pair_agree": sum(agreements),
            "all_candidate_ranks_stable": all(agreements) if len(cp_moves) == len(candidates) and agreements else None}


def analyze_rows(samples: list[dict], rows: list[dict]) -> dict:
    if not rows or len(samples) != len(rows):
        raise ValueError("result count must match fixed sample")
    budgets = rows[0]["teacher_nodes"]
    if not budgets or budgets != sorted(set(budgets)):
        raise ValueError("teacher budgets must be strictly increasing")
    metrics, stability = [], []
    for index, (sample, row) in enumerate(zip(samples, rows)):
        if (row["index"] != index or row["game_number"] != sample["diagnostic_game"]
                or row["ply"] != sample["ply"] or row["teacher_nodes"] != budgets
                or row["proposal_nodes"] != rows[0]["proposal_nodes"]):
            raise ValueError("result does not match sample or budget contract")
        identity = {"index": index, "cohort": sample["cohort"], "game_key": sample["cohort"] + ":" + sample["game_key"]}
        for budget in budgets:
            metrics.append({**identity, "budget": budget, **parent_metrics(row, budget)})
        for first, second in zip(budgets, budgets[1:]):
            stability.append({**identity, **budget_stability(row, first, second)})
    summary = []
    for cohort in ("saved", "match"):
        for budget in budgets:
            summary.append({"cohort": cohort, "budget": budget, **summarize_parents(
                [item for item in metrics if item["cohort"] == cohort and item["budget"] == budget])})
    stable_summary = []
    for cohort in ("saved", "match"):
        for first, second in zip(budgets, budgets[1:]):
            selected = [item for item in stability if item["cohort"] == cohort and item["first_budget"] == first]
            signs = [item["gap_sign_stable"] for item in selected if item["gap_sign_stable"] is not None]
            ranks = [item["all_candidate_ranks_stable"] for item in selected if item["all_candidate_ranks_stable"] is not None]
            stable_summary.append({"cohort": cohort, "first_budget": first, "second_budget": second,
                                   "gap_sign_comparable": len(signs), "gap_sign_stable": sum(signs),
                                   "strict_gap_reversal_count": sum(item["strict_gap_reversal"] is True for item in selected),
                                   "zero_nonzero_change_count": sum(item["zero_nonzero_change"] is True for item in selected),
                                   "top_set_comparable": sum(item["top_set_changed"] is not None for item in selected),
                                   "top_set_changed_count": sum(item["top_set_changed"] is True for item in selected),
                                   "rank_comparable": len(ranks), "rank_stable": sum(ranks),
                                   "gap_change": statistics([item["gap_change"] for item in selected if item["gap_change"] is not None])})
    return {"proposal_nodes": rows[0]["proposal_nodes"], "teacher_nodes": budgets,
            "summary": summary, "stability_summary": stable_summary,
            "parents": metrics, "stability": stability}


def validate_result_inputs(manifest: dict, rows: list[dict], sample_kind: str) -> None:
    filename = "pilot.mnsd" if sample_kind == "pilot" else "positions.mnsd"
    expected = {"candidate_sha256": manifest["teacher"]["candidate_sha256"],
                "teacher_sha256": manifest["teacher"]["teacher_sha256"],
                "positions_sha256": manifest["files"][filename]}
    for row in rows:
        for name, value in expected.items():
            if row["metadata"][name] != value:
                raise ValueError(f"result {name} differs from preparation manifest")


def analyze(args: argparse.Namespace) -> None:
    manifest = json.loads(args.manifest.read_text())
    all_samples = manifest["samples"]
    samples = [all_samples[i] for i in manifest["pilot_indices"]] if args.sample_kind == "pilot" else all_samples
    rows = [json.loads(line) for line in args.results.read_text().splitlines()]
    validate_result_inputs(manifest, rows, args.sample_kind)
    report = analyze_rows(samples, rows)
    report["inputs"] = {"manifest_sha256": sha256(args.manifest), "results_sha256": sha256(args.results),
                         "sample_kind": args.sample_kind}
    report["definitions"] = {"gap": "T(PST move) - T(FM move), parent perspective cp; positive means FM worse",
                              "teacher": "fixed PST, FM embeddings zero; a budget-dependent reference, not ground truth",
                              "candidate_set_losses": "max T among finite proposed candidates minus T(selected); not full legal-move regret",
                              "mate_terminal_incomplete": "excluded from cp gap; score kinds counted separately",
                              "rank_stability": "pairwise ordering including ties among the same candidate moves"}
    with args.output.open("x") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"summary": report["summary"], "stability_summary": report["stability_summary"]}, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prep = commands.add_parser("prepare")
    for name in ("diagnostics-root", "archive-run", "candidate", "baseline", "output-dir"):
        prep.add_argument("--" + name, type=Path, required=True)
    for name in ("seed", "per-cohort", "pilot-per-cohort"):
        prep.add_argument("--" + name, type=int, required=True)
    analysis = commands.add_parser("analyze")
    for name in ("manifest", "results", "output"):
        analysis.add_argument("--" + name, type=Path, required=True)
    analysis.add_argument("--sample-kind", choices=("pilot", "full"), required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        if args.seed < 0 or not 0 < args.pilot_per_cohort <= args.per_cohort:
            parser.error("seed must be nonnegative; 0 < pilot-per-cohort <= per-cohort")
        prepare(args)
    else:
        analyze(args)


if __name__ == "__main__":
    main()
