#!/usr/bin/env python3
"""Aggregate worker-local stage12 correction JSON and verify bench passivity."""
import argparse
import json
from pathlib import Path


ARRAY_WIDTHS = {
    'observation_bounds': 3, 'candidate_absolute_error': 4, 'table_nonzero': 3,
    'blended_nonzero': 4, 'changed': 4, 'newly_pruned': 4, 'newly_retained': 4,
}


def ratio(numerator, denominator):
    return dict(numerator=numerator, denominator=denominator,
                percent=100 * numerator / denominator if denominator else None)


def bench(path):
    lines = path.read_text().splitlines()
    positions = []
    for line in lines:
        if line.startswith('position='):
            row = dict(cell.split('=', 1) for cell in line.split())
            positions.append({k: row[k] for k in ('position', 'depth', 'nodes', 'best', 'score')})
    summary = [line for line in lines if line.startswith('summary: ')]
    if len(positions) != 15 or len(summary) != 1:
        raise ValueError(f'{path}: expected 15 positions and one summary')
    total = sum(int(row['nodes']) for row in positions)
    fields = dict(cell.split('=', 1) for cell in summary[0].split()[1:])
    if total != int(fields['nodes']):
        raise ValueError(f'{path}: node total mismatch')
    return positions, total


def evaluate(path):
    workers = json.loads(path.read_text())
    if not workers:
        raise ValueError('no worker reports')
    settings = {(w['pawn'], w['correction_weight'], w['correction_cap']) for w in workers}
    if len(settings) != 1:
        raise ValueError('worker parameters differ')
    totals = {key: [0] * ARRAY_WIDTHS[key] if key in ARRAY_WIDTHS else 0
              for key in workers[0]['counts']}
    for worker in workers:
        counts = worker['counts']
        if counts.keys() != totals.keys():
            raise ValueError('worker counters differ')
        for key, value in counts.items():
            if key in ARRAY_WIDTHS:
                if len(value) != ARRAY_WIDTHS[key]:
                    raise ValueError(f'{key}: invalid array width')
                values = value
            else:
                values = [value]
            if any(type(v) is not int or v < 0 for v in values):
                raise ValueError(f'{key}: expected nonnegative integers')
            if key in ARRAY_WIDTHS:
                totals[key] = [a + b for a, b in zip(totals[key], value)]
            else:
                totals[key] += value
        if sum(counts['observation_bounds']) != counts['observations']:
            raise ValueError('observation bounds do not sum to observations')
        if any(n > counts['reference_nodes'] for n in counts['table_nonzero']):
            raise ValueError('nonzero count exceeds references')
        for i in range(4):
            if counts['changed'][i] != counts['newly_pruned'][i] + counts['newly_retained'][i]:
                raise ValueError('activation directions do not sum to changes')
            if counts['newly_retained'][i] > counts['current_pruned']:
                raise ValueError('retained count exceeds current pruned count')
            if counts['newly_pruned'][i] > counts['futility_quiets'] - counts['current_pruned']:
                raise ValueError('newly pruned count exceeds current retained count')
    n = totals['observations']
    error = totals['material_absolute_error']
    references = totals['reference_nodes']
    reference_rates = [ratio(v, references) for v in totals['table_nonzero']]
    candidates = []
    for i, name in enumerate(('a', 'b', 'c', 'a+b+c')):
        candidate_error = totals['candidate_absolute_error'][i]
        added = [i] if i < 3 else [0, 1, 2]
        improvement = ratio(error - candidate_error, error)
        prediction_pass = error > 0 and 10 * (error - candidate_error) >= error
        reference_pass = references > 0 and all(20 * totals['table_nonzero'][j] >= references for j in added)
        activation_pass = totals['futility_quiets'] > 0 and 20 * totals['changed'][i] >= totals['futility_quiets']
        candidates.append(dict(
            name=name, mean_absolute_error=candidate_error / n if n else None,
            improvement=improvement,
            added_table_references={('a', 'b', 'c')[j]: reference_rates[j] for j in added},
            activation=ratio(totals['changed'][i], totals['futility_quiets']),
            newly_pruned=totals['newly_pruned'][i], newly_retained=totals['newly_retained'][i],
            prediction_gate=prediction_pass, reference_gate=reference_pass,
            activation_gate=activation_pass, eligible=prediction_pass and reference_pass and activation_pass))
    pawn, weight, cap = next(iter(settings))
    return dict(workers=len(workers), pawn=pawn, correction_weight=weight, correction_cap=cap,
                counts=totals, baseline_mean_absolute_error=error / n if n else None,
                candidates=candidates)


def percent(value):
    return '未定義' if value is None else f'{value:.4f}%'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('raw', type=Path)
    parser.add_argument('--json', required=True, type=Path)
    parser.add_argument('--text', required=True, type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--disabled', type=Path)
    parser.add_argument('--enabled', type=Path)
    args = parser.parse_args()
    report = evaluate(args.raw)
    checks = (args.baseline, args.disabled, args.enabled)
    if any(checks):
        if not all(checks):
            parser.error('--baseline, --disabled and --enabled must be supplied together')
        baseline, total = bench(args.baseline)
        disabled, disabled_total = bench(args.disabled)
        enabled, enabled_total = bench(args.enabled)
        if not (baseline == disabled == enabled and total == disabled_total == enabled_total == 1677944):
            raise ValueError('depth6 bench passivity mismatch')
        if any(row['depth'] != '6' for row in baseline) or report['workers'] != 15:
            raise ValueError('expected depth6 and 15 worker reports')
        report['passivity'] = dict(passed=True, total_nodes=total, positions=baseline)
    lines = [f"共通の評価集合は{report['counts']['observations']:,}件、現行の平均絶対誤差は{report['baseline_mean_absolute_error']}である。", '',
             '| 候補 | 平均絶対誤差 | 改善率 | 追加表の非ゼロ参照率 | 発動率（分子 / 分母） |',
             '|---|---:|---:|---|---|']
    for candidate in report['candidates']:
        refs = ', '.join(f"{key}: {percent(r['percent'])} ({r['numerator']:,}/{r['denominator']:,})"
                         for key, r in candidate['added_table_references'].items())
        activation = candidate['activation']
        mae = candidate['mean_absolute_error']
        mae_text = '未定義' if mae is None else f'{mae:.6f}'
        lines.append(f"| {candidate['name']} | {mae_text} | {percent(candidate['improvement']['percent'])} | {refs} | {percent(activation['percent'])} ({activation['numerator']:,}/{activation['denominator']:,}) |")
    if 'passivity' in report:
        lines.extend(['', '基準、計測無効、計測有効の15局面で深さ、ノード数、最善手、探索値が完全に一致し、総ノード数は1,677,944だった。'])
    args.json.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    text = '\n'.join(lines) + '\n'
    args.text.write_text(text)
    print(text, end='')


if __name__ == '__main__':
    main()
