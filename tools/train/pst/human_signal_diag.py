"""診断B/C: 固定PSTのロジットに対する王の安全度10列の追加信号。

偶数ハッシュを検証用とする。5分割とブートストラップは対局単位で行う。
正則化は平均二値交差エントロピー + alpha/2 * ||beta||²（切片を除く）。
数値計算のため列を尺度変換するが、ペナルティと出力係数は元の単位。
"""
from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path

import numpy as np
import torch

from features import feature_indices
from king_features_diag import column_names
from mnsd import HEADER, hash64, map_records, read_header, read_provenance, sha256_file
from taper import phase_ratios
from train_pst import make_model, model_logits, read_mnpt

COLUMNS = (6, 7, 8, 9, 10, 11, 26, 29, 34, 37)
ALPHAS = (0.0, 1e-4, 1e-3, 1e-2, 1e-1)
GRADIENT_TOLERANCE = 1e-7
MAX_ITERATIONS = 100


def id_hash(identifier: str) -> int:
    return int.from_bytes(hashlib.sha256(identifier.encode('utf-8')).digest()[:8], 'little')


def pst_logits(records: np.ndarray, pst: Path, output_k: float) -> np.ndarray:
    if not np.isfinite(output_k) or output_k <= 0:
        raise ValueError('output K must be finite and positive')
    mg, eg, _, _ = read_mnpt(pst)
    initial = torch.as_tensor(np.column_stack((mg, eg)).astype(np.float32) / 8)
    model = make_model(initial, torch.device('cpu'), 'tapered').eval()
    result = np.empty(len(records))
    with torch.no_grad():
        for start in range(0, len(records), 4096):
            rows = records[start:start + 4096]
            active = feature_indices(rows['board'], rows['stm'], rows['lion'])
            phi = phase_ratios(rows['board']).astype(np.float32)
            result[start:start + len(rows)] = model_logits(
                model, torch.as_tensor(active), torch.as_tensor(phi), output_k).numpy()
    return result


def read_features(path: Path, checksum: bytes, count: int) -> np.ndarray:
    with path.open('rb') as stream:
        raw = stream.read(HEADER.size)
    if len(raw) != HEADER.size:
        raise ValueError('truncated MNKF header')
    magic, version, definition, columns, positions, digest = HEADER.unpack(raw)
    if (magic, version, definition, columns) != (b'MNKF', 1, 2, 118):
        raise ValueError('MNKF must use version 1, definition 2, and 118 columns')
    if positions != count or digest != checksum:
        raise ValueError('MNKF MNSD count or SHA-256 mismatch')
    if path.stat().st_size != HEADER.size + count * columns:
        raise ValueError('MNKF length mismatch')
    values = np.memmap(path, mode='r', offset=HEADER.size, dtype=np.uint8, shape=(count, columns))
    return values[:, COLUMNS].astype(np.float64)


def covariates(records: np.ndarray, mapping: dict | None, table: dict | None):
    pieces = np.count_nonzero(records['board'], axis=1)
    names = ['gote_to_move', 'pieces_62_77', 'pieces_47_61', 'pieces_le_46']
    columns = [records['stm'], (pieces >= 62) & (pieces < 78),
               (pieces >= 47) & (pieces < 62), pieces <= 46]
    if table is not None:
        details = []
        for number in records['game']:
            identifier = mapping[int(number)]
            if identifier not in table:
                raise ValueError(f'games table is missing {identifier}')
            detail = table[identifier]
            if detail['game'] != int(number):
                raise ValueError('games table game number differs from provenance')
            for key in ('sente_rating', 'gote_rating'):
                if type(detail[key]) not in (int, float) or not np.isfinite(detail[key]):
                    raise ValueError(f'{identifier}: {key} must be finite')
            if not isinstance(detail['speed'], str) or not detail['speed']:
                raise ValueError(f'{identifier}: speed is required')
            details.append(detail)
        sente = np.array([d['sente_rating'] for d in details])
        gote = np.array([d['gote_rating'] for d in details])
        columns.extend([(sente - gote) * (1 - 2 * records['stm'].astype(float)),
                        (sente + gote) / 2,
                        np.array([d['speed'] == 'correspondence' for d in details])])
        names.extend(['rating_difference', 'rating_mean', 'correspondence'])
        initial = np.full(len(records), np.nan)
        for i, detail in enumerate(details):
            clock = detail['clock']
            if clock is None:
                continue
            if not isinstance(clock, dict):
                raise ValueError('clock must be an object or null')
            if 'initial' in clock:
                value = clock['initial']
                if type(value) not in (int, float) or not np.isfinite(value) or value < 0:
                    raise ValueError('clock.initial must be finite nonnegative seconds')
                initial[i] = value
        # 固定の列定義。検証側の出現区分を見てモデルの列を変更しない。
        columns.extend([(initial >= 600) & (initial < 1800), initial >= 1800, np.isnan(initial)])
        names.extend(['clock_600_1799', 'clock_ge_1800', 'clock_missing'])
    return np.column_stack(columns).astype(float), names


@dataclass
class Sample:
    x: np.ndarray
    extra: np.ndarray
    offset: np.ndarray
    y: np.ndarray
    games: np.ndarray
    validation: np.ndarray
    names: list[str]
    counts: dict
    exclusions: dict
    assignments: list[dict]
    teacher: dict


def load_sample(data: Path, features: Path, pst: Path, output_k: float,
                origin: str, games_table: Path | None = None) -> Sample:
    header = read_header(data)
    records = map_records(data)
    checksum = sha256_file(data)
    teacher, _, mapping = read_provenance(data, checksum)
    if teacher.origin != origin:
        raise ValueError(f'{data}: expected {origin}, got {teacher.origin}')
    if (teacher.generation_commit, teacher.network_checksum, teacher.nodes, teacher.rule_set) != (
            header.generation_commit, header.network_checksum.hex(), header.teacher_nodes, header.rule_set):
        raise ValueError('provenance teacher differs from MNSD header')
    numbers = np.unique(records['game'])
    if mapping is not None:
        if not set(map(int, numbers)) <= mapping.keys():
            raise ValueError('provenance is missing a recorded game')
        if len(set(mapping.values())) != len(mapping):
            raise ValueError('one source game ID per game is required')
    hashes = (hash64(header.seed, numbers) if mapping is None else
              np.array([id_hash(mapping[int(g)]) for g in numbers], dtype=np.uint64))
    validation = (hashes[np.searchsorted(numbers, records['game'])] % 2) == 0
    all_features = read_features(features, checksum, len(records))
    midgame = np.count_nonzero(records['board'], axis=1) >= 47
    eligible = np.ones(len(records), dtype=bool) if origin == 'human-game' else midgame
    draws = records['result'] == 1
    keep = eligible & ~draws
    table = None if games_table is None else json.loads(games_table.read_text(encoding='utf-8'))
    if origin == 'human-game' and not isinstance(table, dict):
        raise ValueError('human games require a games table object')
    selected = records[keep]
    x, names = covariates(selected, mapping, table)
    counts = {}
    for name, mask in [('diagnostic', ~validation), ('validation', validation)]:
        counts[name] = {'games': int(np.unique(records['game'][keep & mask]).size),
                        'positions': int(np.count_nonzero(keep & mask)),
                        'input_games': int(np.unique(records['game'][mask]).size),
                        'input_positions': int(np.count_nonzero(mask)),
                        'excluded_draw_positions_in_scope': int(np.count_nonzero(draws & eligible & mask)),
                        'excluded_non_midgame_positions': int(np.count_nonzero(~eligible & mask))}
    assignments = []
    for game, hashed in zip(numbers, hashes):
        positions = np.flatnonzero(keep & (records['game'] == game))
        assignments.append({'game': int(game), 'id': None if mapping is None else mapping[int(game)],
                            'split': 'validation' if int(hashed) % 2 == 0 else 'diagnostic',
                            'record_indices': positions.tolist()})
    exclusions = {
        'draw_positions': int(np.count_nonzero(draws)),
        'draw_positions_in_scope': int(np.count_nonzero(draws & eligible)),
        'non_midgame_positions': int(np.count_nonzero(~eligible)),
        'games_without_midgame_records': (None if origin == 'human-game' else
            int(len(numbers) - np.unique(records['game'][midgame]).size)),
        'games_without_binary_results_in_scope': int(len(numbers) - np.unique(selected['game']).size),
        'mapped_games_without_records': None if mapping is None else len(mapping) - len(numbers),
        'generator_discarded_max_ply_games': None,
        'generator_exclusion_note': 'MNSD does not encode generator discards; consult generation report.',
    }
    return Sample(x, all_features[keep], pst_logits(selected, pst, output_k),
                  selected['result'].astype(float) / 2, selected['game'].copy(), validation[keep],
                  names, counts, exclusions, assignments, asdict(teacher))


@dataclass
class Fit:
    coefficients: np.ndarray
    converged: bool
    iterations: int
    gradient_norm: float | None


def fit_logistic(x: np.ndarray, offset: np.ndarray, y: np.ndarray, alpha: float,
                 max_iterations: int = MAX_ITERATIONS) -> Fit:
    """列の尺度で前処理したニュートン法。特異な列は最小ノルム解で扱う。"""
    design = np.column_stack((np.ones(len(x)), x))
    if not len(x) or np.unique(y).size < 2:
        return Fit(np.zeros(design.shape[1]), False, 0, None)
    scale = np.sqrt(np.mean(design ** 2, axis=0))
    scale[scale == 0] = 1  # 恒等的に0の列には尺度変換が不要。
    design /= scale
    penalty = alpha / scale ** 2
    penalty[0] = 0
    weights = np.zeros(design.shape[1])

    def objective(w):
        z = offset + design @ w
        return float(np.mean(np.logaddexp(0, z) - y * z) + .5 * np.dot(penalty, w * w))

    norm = None
    for iteration in range(max_iterations + 1):
        z = offset + design @ weights
        p = np.exp(-np.logaddexp(0, -z))
        gradient = design.T @ (p - y) / len(y) + penalty * weights
        norm = float(np.max(np.abs(gradient)))
        if np.isfinite(norm) and norm <= GRADIENT_TOLERANCE:
            return Fit(weights / scale, True, iteration, norm)
        if iteration == max_iterations or not np.isfinite(norm):
            break
        curvature = p * (1 - p)
        hessian = (design.T * curvature) @ design / len(y) + np.diag(penalty)
        try:
            step = np.linalg.lstsq(hessian, gradient, rcond=None)[0]
        except np.linalg.LinAlgError:
            break  # 数値的に解けなければ非収束として判定へ渡す。
        descent = float(gradient @ step)
        loss = objective(weights)
        fraction = 1.0
        for _ in range(40):
            updated = weights - fraction * step
            if descent > 0 and objective(updated) <= loss - 1e-4 * fraction * descent:
                weights = updated
                break
            fraction *= .5
        else:
            break
    return Fit(weights / scale, False, iteration, norm if np.isfinite(norm) else None)


def losses(x, offset, y, coefficients):
    z = offset + coefficients[0] + x @ coefficients[1:]
    return np.logaddexp(0, z) - y * z


def game_means(values: np.ndarray, games: np.ndarray) -> np.ndarray:
    _, inverse, counts = np.unique(games, return_inverse=True, return_counts=True)
    return np.bincount(inverse, weights=values, minlength=len(counts)) / counts


def cross_validation_folds(games: np.ndarray, seed: int) -> np.ndarray:
    unique, inverse = np.unique(games, return_inverse=True)
    order = np.random.default_rng(seed).permutation(len(unique))
    fold = np.empty(len(unique), dtype=int)
    fold[order] = np.arange(len(unique)) % 5
    return fold[inverse]


def select_model(x, offset, y, games, names, seed):
    if np.unique(games).size < 5:
        return None, {'converged': False, 'reason': 'fewer_than_5_diagnostic_games'}
    folds = cross_validation_folds(games, seed)
    scores = []
    failed = []
    for alpha in ALPHAS:
        held_out = np.empty(len(y))
        valid = True
        for fold in range(5):
            train = folds != fold
            test = ~train
            fit = fit_logistic(x[train], offset[train], y[train], alpha)
            if not fit.converged:
                failed.append({'alpha': alpha, 'fold': fold, 'iterations': fit.iterations,
                               'gradient_norm': fit.gradient_norm})
                valid = False
            held_out[test] = losses(x[test], offset[test], y[test], fit.coefficients)
        scores.append(float(game_means(held_out, games).mean()) if valid else None)
    available = [i for i, score in enumerate(scores) if score is not None]
    report = {'cv': [{'alpha': a, 'loss': s} for a, s in zip(ALPHAS, scores)],
              'cv_failures': failed, 'fold_games': [np.unique(games[folds == f]).tolist() for f in range(5)]}
    if not available:
        return None, dict(report, converged=False, reason='all_cv_fits_failed')
    best = min(available, key=lambda i: scores[i])
    fit = fit_logistic(x, offset, y, ALPHAS[best])
    report.update(alpha=ALPHAS[best], converged=fit.converged and not failed,
                  final_fit_converged=fit.converged, iterations=fit.iterations,
                  gradient_norm=fit.gradient_norm,
                  coefficients=dict(zip(['intercept'] + names, fit.coefficients.tolist())))
    return fit, report


def bootstrap_means(values: np.ndarray, repetitions: int, rng) -> np.ndarray:
    if repetitions < 2:
        raise ValueError('bootstrap repetitions must be at least 2')
    if not len(values):
        return np.full(repetitions, np.nan)
    samples = np.empty(repetitions)
    for i in range(repetitions):
        samples[i] = values[rng.integers(len(values), size=len(values))].mean()
    return samples


def interval(estimate: float, samples: np.ndarray) -> dict:
    finite = samples[np.isfinite(samples)]
    return {'estimate': float(estimate) if np.isfinite(estimate) else None,
            'lower_95_one_sided': float(np.quantile(finite, .05)) if len(finite) else None,
            'upper_95_one_sided': float(np.quantile(finite, .95)) if len(finite) else None,
            'standard_error': float(np.std(finite, ddof=1)) if len(finite) > 1 else None,
            'invalid_fraction': float(1 - len(finite) / len(samples))}


def diagnose_group(sample: Sample, repetitions: int, seed: int, rng):
    train = ~sample.validation
    test = sample.validation
    candidates = [('baseline', sample.x, sample.names),
                  ('candidate', np.column_stack((sample.x, sample.extra)),
                   sample.names + [column_names()[c] for c in COLUMNS])]
    report = {'counts': sample.counts, 'exclusions': sample.exclusions,
              'assignments': sample.assignments, 'teacher': sample.teacher, 'models': {}}
    held_out = []
    reasons = []
    for name, x, names in candidates:
        fit, details = select_model(x[train], sample.offset[train], sample.y[train],
                                    sample.games[train], names, seed)
        report['models'][name] = details
        if not details['converged']:
            reasons.append(f'{name}:regression_not_converged')
        if fit is not None and fit.converged:
            held_out.append(game_means(losses(x[test], sample.offset[test], sample.y[test],
                                             fit.coefficients), sample.games[test]))
    gain = held_out[0] - held_out[1] if len(held_out) == 2 else np.array([])
    samples = bootstrap_means(gain, repetitions, rng)
    report['g'] = interval(float(gain.mean()) if gain.size else np.nan, samples)
    report['game_losses'] = ([{'game': int(game), 'baseline': float(base), 'candidate': float(candidate)}
                             for game, base, candidate in zip(np.unique(sample.games[test]), *held_out)]
                            if len(held_out) == 2 else [])
    report['insufficient_reasons'] = reasons
    return report, samples


def decision(groups: dict, contrast: dict | None = None) -> tuple[str, list[str]]:
    reasons = [f'{name}:{reason}' for name, group in groups.items()
               for reason in group['insufficient_reasons']]
    is_start = contrast is not None
    if is_start:
        for name, group in groups.items():
            if group['counts']['validation']['games'] < 300:
                reasons.append(f'{name}:fewer_than_300_validation_games')
    for name, group in groups.items():
        if group['g']['invalid_fraction'] >= .05:
            reasons.append(f'{name}:bootstrap_invalid_at_least_5_percent')
    criteria = ([('human-start.g', groups['human-start']['g']), ('G', contrast)] if is_start else
                [('human-games.g', groups['human-games']['g'])])
    for name, metric in criteria:
        if metric['invalid_fraction'] >= .05 and name == 'G':
            reasons.append('G:bootstrap_invalid_at_least_5_percent')
        lower, upper = metric['lower_95_one_sided'], metric['upper_95_one_sided']
        if lower is None or upper is None:
            reasons.append(f'{name}:interval_unavailable')
        elif lower <= 0 <= upper:
            reasons.append(f'{name}:interval_contains_zero')
    if reasons:
        return '判断材料不足', reasons
    passed = all(metric['lower_95_one_sided'] > 0 for _, metric in criteria)
    return ('基準を満たす' if passed else '基準を満たさない'), []


def run(args) -> dict:
    if args.bootstrap < 2 or args.seed < 0:
        raise ValueError('bootstrap must be at least 2 and seed nonnegative')
    rngs = [np.random.default_rng(s) for s in np.random.SeedSequence(args.seed).spawn(2)]
    if args.command == 'human-games':
        sample = load_sample(args.data, args.king_features, args.pst, args.output_k,
                             'human-game', args.games_table)
        group, _ = diagnose_group(sample, args.bootstrap, args.seed, rngs[0])
        groups = {'human-games': group}
        contrast = None
    else:
        human = load_sample(args.human_start, args.human_start_features, args.pst, args.output_k,
                            'human-start-selfplay')
        random = load_sample(args.random_start, args.random_start_features, args.pst, args.output_k,
                             'random-selfplay')
        for key in ('generation_commit', 'network_checksum', 'nodes', 'rule_set', 'search_condition'):
            if human.teacher[key] != random.teacher[key]:
                raise ValueError(f'human/random generation conditions differ: {key}')
        h, hs = diagnose_group(human, args.bootstrap, args.seed, rngs[0])
        r, rs = diagnose_group(random, args.bootstrap, args.seed, rngs[1])
        groups = {'human-start': h, 'random-start': r}
        estimate = (h['g']['estimate'] - r['g']['estimate']
                    if h['g']['estimate'] is not None and r['g']['estimate'] is not None else np.nan)
        contrast = interval(estimate, hs - rs)
    verdict, reasons = decision(groups, contrast)
    return {'diagnostic': args.command, 'pst': str(args.pst), 'output_k': args.output_k,
            'bootstrap': args.bootstrap, 'seed': args.seed, 'columns': list(COLUMNS),
            'method': {'validation_hash_parity': 0, 'cv_folds': 5,
                       'objective': 'mean position BCE + alpha/2 * squared raw coefficients; intercept unpenalized',
                       'optimizer': 'NumPy Newton, Armijo line search, least-squares Hessian solve',
                       'gradient_tolerance': GRADIENT_TOLERANCE, 'max_iterations': MAX_ITERATIONS,
                       'interval': '5th and 95th percentiles; zero crossing uses this central 90% interval',
                       'piece_reference': '78 or more',
                       'clock_reference': 'initial < 600 seconds; missing has its own indicator',
                       'clock_columns': 'fixed 600<=initial<1800, initial>=1800, missing; speed correspondence separate'},
            'groups': groups, 'G': contrast, 'decision': verdict, 'insufficient_reasons': reasons}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    human = commands.add_parser('human-games')
    for flag in ('data', 'king-features', 'games-table'):
        human.add_argument('--' + flag, type=Path, required=True)
    start = commands.add_parser('human-start')
    for flag in ('human-start', 'random-start', 'human-start-features', 'random-start-features'):
        start.add_argument('--' + flag, type=Path, required=True)
    for command in (human, start):
        command.add_argument('--pst', type=Path, required=True)
        command.add_argument('--output-k', type=float, required=True)
        command.add_argument('--output', type=Path, required=True)
        command.add_argument('--bootstrap', type=int, default=2000)
        command.add_argument('--seed', type=int, default=1)
    args = parser.parse_args(argv)
    result = run(args)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
