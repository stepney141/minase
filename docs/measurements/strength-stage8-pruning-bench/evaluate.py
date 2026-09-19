#!/usr/bin/env python3
"""Stage 8 phase 1: loss means lost good moves / all eligible good moves.

Samples are whitespace separated, without headers, in the instruction's column
order. '-' denotes undefined improving or SEE. Ratios with zero denominators
are null, never zero. Activation uses the design's pre-exclusion denominator.
Quantiles use the nearest-rank convention. No third-party packages are needed.
"""
import argparse
from collections import Counter
from fractions import Fraction
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile


def ratio(numerator, denominator):
    return {"numerator": numerator, "denominator": denominator,
            "ratio": numerator / denominator if denominator else None}


def rate_pass(value, threshold=0.05):
    return value["denominator"] > 0 and value["numerator"] >= threshold * value["denominator"]


def read_rows(path, width):
    rows = []
    with path.open() as stream:
        for number, line in enumerate(stream, 1):
            cells = line.split()
            if len(cells) != width:
                raise ValueError(f"{path}:{number}: expected {width} columns")
            rows.append(tuple(None if cell == '-' else int(cell) for cell in cells))
    return rows


def metric(rows, predicate, parameters):
    selected = [row for row in rows if predicate(row)]
    return dict(parameters, activation=ratio(len(selected), len(rows)),
                loss=ratio(sum(row[-1] for row in selected), sum(row[-1] for row in rows)))


def choose(candidates, count, good, tie):
    if count < 1000:
        return None, "fewer_than_1000_targets"
    if good == 0:
        return None, "no_good_results"
    eligible = [c for c in candidates if 10 * c['loss']['numerator'] <= good]
    if not eligible:
        return None, "no_candidate_with_loss_at_most_10_percent"
    return max(eligible, key=lambda c: (c['activation']['numerator'], *tie(c))), None


def quantiles(values):
    values = sorted(values)
    return {str(p): values[math.ceil(len(values) * p / 100) - 1] if values else None
            for p in (50, 75, 90, 95, 99)}


def depth_groups(rows, depths=(1, 2, 3)):
    return [(str(d), [r for r in rows if r[0] == d]) for d in depths] + [('all', rows)]


def count_at(counts, name, depth):
    values = counts.get(name, {})  # An absent counter has observed zero events.
    return sum(values.values()) if depth == 'all' else values.get(depth, 0)


def applied_summary(report, parameters, denominator=None):
    chosen = [(d, report[str(d)]['selected']) for d in (1, 2, 3)]
    pruned = sum(c['activation']['numerator'] for _, c in chosen if c)
    lost = sum(c['loss']['numerator'] for _, c in chosen if c)
    if denominator is None:
        denominator = sum(report[str(d)]['targets'] for d in (1, 2, 3))
    activation = ratio(pruned, denominator)
    return dict(choices={str(d): {p: c[p] for p in parameters} if c else None for d, c in chosen},
                activation=activation, loss=ratio(lost, sum(report[str(d)]['good'] for d in (1, 2, 3))),
                at_least_5_percent=rate_pass(activation))


def chosen_activation(selected, denominator):
    value = ratio(selected['activation']['numerator'] if selected else 0, denominator)
    return {"activation": value, "at_least_5_percent": rate_pass(value)}


def see_report(rows, pawn, counts):
    report = {}
    for depth, group in depth_groups(rows):
        targets = [r for r in group if r[2] and r[3]]
        candidates = [metric(targets, lambda r: r[5] is not None and r[5] < -m, {'m': m})
                      for m in (0, pawn // 2, pawn, 2 * pawn, 3 * pawn)]
        selected, reason = choose(candidates, len(targets), sum(r[-1] for r in targets), lambda c: (c['m'],))
        report[depth] = dict(targets=len(targets), good=sum(r[-1] for r in targets),
                            exclusions_best_score=sum(not r[2] for r in group),
                            exclusions_royal=sum(not r[3] for r in group),
                            exclusions_union=sum(not (r[2] and r[3]) for r in group),
                            see_none=sum(r[5] is None for r in targets),
                            see_none_all_samples=sum(r[5] is None for r in group),
                            candidates=candidates, selected=selected, reason=reason,
                            **chosen_activation(selected, count_at(counts, 'capture_inspected', depth)))
    # Overall activation applies the per-depth selections, not a pooled threshold.
    report['all'].update(chosen_activation({"activation": {"numerator": sum(report[str(d)]['activation']['numerator'] for d in (1, 2, 3))}}, count_at(counts, 'capture_inspected', 'all')))
    report['all']['applied'] = applied_summary(report, ('m',), count_at(counts, 'capture_inspected', 'all'))
    return report


def quiet_report(rows, counts):
    report = {}
    for depth, group in depth_groups(rows):
        targets = [r for r in group if not r[3] and not r[4] and r[5] and r[6]]
        # Aggregate index counts to produce the 200-point curve without rescanning.
        index_count = Counter(r[1] for r in targets)
        index_good = Counter(r[1] for r in targets if r[-1])
        total_good = sum(r[-1] for r in targets)
        curve = [{"k": k, "activation": ratio(sum(v for i, v in index_count.items() if i >= k), len(targets)),
                  "loss": ratio(sum(v for i, v in index_good.items() if i >= k), total_good)} for k in range(1, 201)]
        candidates = []
        for k in (4, 6, 8, 12, 16, 24, 32):
            for h in (0, 32, 128):
                candidate = metric(targets, lambda r: r[1] >= k and r[2] <= h, {'k': k, 'h': h})
                comparisons = [p for p in curve if p['activation']['numerator'] >= candidate['activation']['numerator']]
                comparator = min(comparisons, key=lambda p: (p['activation']['numerator'], -p['k'])) if comparisons else None
                candidate['index_only_comparator'] = comparator
                candidate['history_claim_passes'] = bool(comparator and total_good > 0 and
                    3 * candidate['loss']['numerator'] <= 2 * comparator['loss']['numerator'])
                candidates.append(candidate)
        selected, reason = choose(candidates, len(targets), total_good, lambda c: (c['k'], -c['h']))
        report[depth] = dict(targets=len(targets), good=total_good, candidates=candidates, selected=selected,
                            reason=reason, index_only_curve=curve, history_quantiles=quantiles([r[2] for r in targets]),
                            good_index_quantiles=quantiles([r[1] for r in targets if r[-1]]),
                            futility_skipped=count_at(counts, 'futility_skipped', depth),
                            **chosen_activation(selected, count_at(counts, 'quiet_expanded', depth)))
    report['all'].update(chosen_activation({"activation": {"numerator": sum(report[str(d)]['activation']['numerator'] for d in (1, 2, 3))}}, count_at(counts, 'quiet_expanded', 'all')))
    report['all']['applied'] = applied_summary(report, ('k', 'h'), count_at(counts, 'quiet_expanded', 'all'))
    return report


def futility_report(rows, pawn):
    margins = {1: pawn // 2, 2: pawn * 3 // 2, 3: pawn * 3 // 2}
    report = {}
    for depth, group in depth_groups(rows):
        targets = [r for r in group if not r[4] and r[5] and r[6] and r[7] == 0]
        candidates = [metric(targets, lambda r: r[8] >= margins[r[0]] * q // 4,
                             {'q': str(Fraction(q, 4)), 'quarters': q}) for q in (4, 3, 2, 1)]
        selected, reason = choose(candidates, len(targets), sum(r[-1] for r in targets), lambda c: (c['quarters'],))
        report[depth] = dict(targets=len(targets), good=sum(r[-1] for r in targets), candidates=candidates,
                            selected=selected, reason=reason)
    report['all']['applied'] = applied_summary(report, ('q',))
    return report, margins


def lmr_report(rows, counts):
    # Below depth 3 the cap is zero. Preserve those counters separately in JSON,
    # and keep the all-depth LMR summary equal to the sum of depths >= 3.
    counts = {name: {d: n for d, n in values.items() if int(d) >= 3}
              for name, values in counts.items()}
    report = {}
    depths = sorted({3, *[r[0] for r in rows], *[int(d) for values in counts.values() for d in values if int(d) >= 3]})
    for depth, group in depth_groups(rows, depths):
        good = [r for r in group if r[7]]
        current = sum(not r[6] for r in good)
        new = sum(not r[5] for r in good)
        report[depth] = dict(targets=len(group), good=len(good), current_loss=ratio(current, len(good)),
                            new_loss=ratio(new, len(good)), difference=ratio(new - current, len(good)),
                            difference_percentage_points=100 * (new - current) / len(good) if good else None,
                            exceeds_10_points=(10 * (new - current) > len(good)) if good else None,
                            excluded_capped=count_at(counts, 'lmr_capped', depth),
                            excluded_improving=count_at(counts, 'lmr_improving', depth),
                            excluded_undefined=count_at(counts, 'lmr_undefined', depth))
    return report


def improving_report(rows, futility, margins):
    report = {}
    for depth, group in depth_groups(rows, sorted({1, 2, 3, *[r[0] for r in rows]})):
        targets = [r for r in group if r[2] or r[7] > 0]
        a = b = both = 0
        for r in targets:
            if r[3] != 0:
                continue
            selection = futility[str(r[0])]['selected'] if r[2] else None
            fut = bool(selection and r[4] + margins[r[0]] > r[5] and
                       r[4] + margins[r[0]] * selection['quarters'] // 4 <= r[5])
            lmr = r[8] > 0
            a += fut
            b += lmr
            both += fut and lmr
        activation = ratio(a + b - both, len(targets))
        report[depth] = dict(activation=activation, futility=ratio(a, len(targets)), lmr=ratio(b, len(targets)),
                            futility_only=ratio(a - both, len(targets)), lmr_only=ratio(b - both, len(targets)),
                            both=ratio(both, len(targets)), at_least_5_percent=rate_pass(activation))
    return report


def correction_report(raw):
    tables = []
    for table in raw['tables']:
        s = table['stats']
        improvement = ratio(s['absolute_error'] - s['corrected_error'], s['absolute_error'])
        nonzero = ratio(s['nonzero'], s['eligible_nodes'])
        tables.append(dict(table, improvement=improvement, reuse_rate=ratio(s['reuse'], s['observations']),
                           nonzero_rate=nonzero, at_least_5_percent=rate_pass(nonzero)))
    defined = [t for t in tables if t['improvement']['denominator']]
    best = max(defined, key=lambda t: Fraction(t['improvement']['numerator'], t['improvement']['denominator'])) if defined else None
    accepted = bool(best and rate_pass(best['improvement'], 0.1) and rate_pass(best['nonzero_rate']))
    return {'tables': tables, 'best': best, 'decision': 'candidate' if accepted else '見送り'}


def verification_report(path, captures=None, quiets=None):
    records = []
    with path.open() as stream:
        for line in stream:
            kind, number, good, reference = line.split()
            if kind not in ('captures', 'quiets') or good not in ('0', '1') or reference not in ('0', '1'):
                raise ValueError('invalid verification row')
            number, good, reference = int(number), int(good), int(reference)
            source = captures if kind == 'captures' else quiets
            if source is not None:
                if not 1 <= number <= len(source) or good != source[number - 1][-1]:
                    raise ValueError('verification row disagrees with its source sample')
                depth = source[number - 1][0]
            else:
                depth = None
            records.append((kind, depth, good, reference))
    def summarize(group):
        agreement = sum(r[2] and r[3] for r in group)
        forward = ratio(agreement, sum(r[2] for r in group))
        reverse = ratio(agreement, sum(r[3] for r in group))
        return dict(samples=len(group), reference_given_recorded_good=forward,
                    recorded_given_reference_good=reverse,
                    passes=rate_pass(forward, 0.8) and rate_pass(reverse, 0.8))
    result = {'all': summarize(records)}
    for d in (1, 2, 3):
        result[str(d)] = summarize([r for r in records if r[1] == d])
    for kind in ('captures', 'quiets'):
        result[kind] = summarize([r for r in records if r[0] == kind])
    return result


def evaluate(samples, lmr):
    captures = read_rows(samples / 'captures.txt', 9)
    quiets = read_rows(samples / 'quiets.txt', 11)
    nodes = read_rows(samples / 'nodes.txt', 9)
    correction = json.loads((samples / 'correction.json').read_text())
    counts = json.loads((samples / 'counts.json').read_text())['counts']
    pawn = correction['pawn_value']
    futility, margins = futility_report(quiets, pawn)
    return dict(pawn_value=pawn,
                definitions={'loss': 'lost good results / all eligible good results',
                             'activation': 'selected per-depth candidates / pre-exclusion design population',
                             'quantiles': 'nearest rank',
                             'correction_reference_population': 'eligible nodes; current LMR does not use static evaluation',
                             'pooled_selection': 'all.selected is descriptive; actual activation applies per-depth selections'},
                see=see_report(captures, pawn, counts), quiet=quiet_report(quiets, counts), futility=futility,
                lmr=lmr_report(read_rows(lmr, 8), json.loads(lmr.with_suffix('.counts.json').read_text())),
                lmr_exclusions_by_depth=json.loads(lmr.with_suffix('.counts.json').read_text()),
                improving=improving_report(nodes, futility, margins), correction=correction_report(correction),
                verification=verification_report(samples / 'verify.txt', captures, quiets))


def human_text(report):
    lines = ['Stage 8 phase 1, bench depth 6', '',
             'Loss = lost good results / all eligible good results.',
             'Activation = selected per-depth pruning / the design population before exclusions.',
             'The all-depth candidate is descriptive; actual activation uses per-depth choices.',
             f"Pawn value = {report['pawn_value']}.", '']

    def fmt(value):
        percent = 'undefined' if value['ratio'] is None else f"{100 * value['ratio']:.4f}%"
        return f"{value['numerator']}/{value['denominator']} ({percent})"

    def table(headers, rows):
        lines.append(' | '.join(headers))
        lines.append(' | '.join('---' for _ in headers))
        lines.extend(' | '.join(map(str, row)) for row in rows)
        lines.append('')

    for key, title, params in [('see', 'Capture SEE', ('m',)),
                               ('quiet', 'History-conditioned late quiet pruning', ('k', 'h')),
                               ('futility', 'Improving: futility margin', ('q',))]:
        lines.extend([title, ''])
        for depth, group in report[key].items():
            selected = group['selected']
            choice = ', '.join(f"{p}={selected[p]}" for p in params) if selected else group['reason']
            lines.append(f"Depth {depth}: {group['targets']} targets, {group['good']} good results; selected {choice}.")
            if 'activation' in group:
                lines.append(f"Design activation {fmt(group['activation'])}; at least 5%: {group['at_least_5_percent']}.")
            if key == 'see':
                lines.append(f"Excluded by best score: {group['exclusions_best_score']}; royal attack: {group['exclusions_royal']}; union: {group['exclusions_union']}; undefined SEE among targets: {group['see_none']}; among all samples: {group['see_none_all_samples']}.")
            if key == 'quiet':
                lines.append(f"Futility skipped {group['futility_skipped']} moves. History quantiles: {group['history_quantiles']}; good-result index quantiles: {group['good_index_quantiles']}.")
            if 'applied' in group:
                applied = group['applied']
                lines.append(f"Applied per-depth choices {applied['choices']}: activation {fmt(applied['activation'])}, loss {fmt(applied['loss'])}.")
            lines.append('')
            headers = [*params, 'Pruned / targets', 'Lost / good']
            if key == 'quiet':
                headers += ['Index-only k', 'Index-only pruned / targets', 'Index-only lost / good', 'History claim passes']
            rows = []
            for candidate in group['candidates']:
                row = [*[candidate[p] for p in params], fmt(candidate['activation']), fmt(candidate['loss'])]
                if key == 'quiet':
                    comp = candidate['index_only_comparator']
                    row += [comp['k'] if comp else None, fmt(comp['activation']) if comp else None,
                            fmt(comp['loss']) if comp else None, candidate['history_claim_passes']]
                rows.append(row)
            table(headers, rows)
            if key == 'quiet':
                lines.extend([f"Depth {depth}, complete index-only curve", ''])
                table(['k', 'Pruned / targets', 'Lost / good'],
                      [[c['k'], fmt(c['activation']), fmt(c['loss'])] for c in group['index_only_curve']])
    lines.extend(['Improving: extra LMR reduction', ''])
    table(['Depth', 'Targets', 'Current lost / full good', 'New lost / full good', 'Increase (points)', '>10 points', 'Capped', 'Improving', 'Undefined'],
          [[d, g['targets'], fmt(g['current_loss']), fmt(g['new_loss']), g['difference_percentage_points'], g['exceeds_10_points'],
            g['excluded_capped'], g['excluded_improving'], g['excluded_undefined']] for d, g in report['lmr'].items()])
    lines.extend(['Improving activation', ''])
    table(['Depth', 'Union', 'Futility only', 'LMR only', 'Both', 'At least 5%'],
          [[d, fmt(g['activation']), fmt(g['futility_only']), fmt(g['lmr_only']), fmt(g['both']), g['at_least_5_percent']] for d, g in report['improving'].items()])
    lines.extend(['Static evaluation correction', ''])
    table(['Key', 'D', 'Error improvement', 'Reuse', 'Nonzero references', 'Keys', 'Exact', 'Upper', 'Lower'],
          [[g['key'], g['denominator'], fmt(g['improvement']), fmt(g['reuse_rate']), fmt(g['nonzero_rate']),
            g['stats']['keys'], g['stats']['exact'], g['stats']['upper'], g['stats']['lower']] for g in report['correction']['tables']])
    best = report['correction']['best']
    lines.append(f"Best key={best['key']}, D={best['denominator']}; decision={report['correction']['decision']}." if best else 'No correction observations.')
    lines.extend(['', 'Search verification', ''])
    table(['Group', 'Samples', 'Reference good / recorded good', 'Recorded good / reference good', 'Passes 80%'],
          [[d, g['samples'], fmt(g['reference_given_recorded_good']), fmt(g['recorded_given_reference_good']), g['passes']]
           for d, g in report['verification'].items()])
    return '\n'.join(lines) + '\n'


def self_test():
    # Requirements: loss denominator is all good results, not pruned moves;
    # minimum 1000 samples, <=10% loss, maximal pruning, conservative ties.
    rows = [(1, i >= 900) for i in range(1000)]
    c = metric(rows, lambda r: not r[-1], {'m': 0})
    assert c['activation'] == ratio(900, 1000) and c['loss'] == ratio(0, 100)
    ten = dict(m=50, activation=ratio(910, 1000), loss=ratio(10, 100))
    eleven = dict(m=0, activation=ratio(911, 1000), loss=ratio(11, 100))
    tied = dict(ten, m=100)
    selected, _ = choose([c, ten, eleven, tied], 1000, 100, lambda c: (c['m'],))
    assert selected['m'] == 100
    assert choose([ten], 999, 100, lambda c: (c['m'],))[0] is None
    assert choose([ten], 1000, 0, lambda c: (c['m'],))[0] is None
    assert choose([eleven], 1000, 100, lambda c: (c['m'],))[0] is None
    # Handwritten types repeated to meet the specified sample-size gate.
    captures = [(1, 1, 1, 1, 0, -100, 1, 0, 0)] * 900 + [(1, 1, 1, 1, 0, 0, 0, 0, 1)] * 100
    result = see_report(captures, 100, {'capture_inspected': {'1': 1200}})
    assert result['1']['selected']['m'] == 50  # strict SEE < -m, tie favours 50
    assert result['all']['activation'] == ratio(900, 1200)
    quiets = [(1, 40, 0, 0, 0, 1, 1, 0, 40, 0, 0)] * 900 + [(1, 40, 200, 0, 0, 1, 1, 0, 0, 0, 1)] * 100
    q = quiet_report(quiets, {'quiet_expanded': {'1': 1100}})['1']
    assert (q['selected']['k'], q['selected']['h']) == (32, 0)
    assert q['selected']['history_claim_passes']
    f, _ = futility_report(quiets, 100)
    assert f['1']['selected']['q'] == '3/4'  # 37 catches 40; 50 does not
    assert f['2']['selected'] is None
    lmr = lmr_report([(3, 1, 0, 0, 1, 0, 1, 1), (3, 2, 0, 0, 1, 1, 1, 1)], {})['3']
    assert lmr['current_loss'] == ratio(0, 2) and lmr['new_loss'] == ratio(1, 2)
    assert lmr['difference_percentage_points'] == 50 and lmr['exceeds_10_points']
    correction = correction_report({'tables': [{'key': 'royals', 'denominator': 32,
        'stats': {'absolute_error': 100, 'corrected_error': 80, 'reuse': 4,
                  'observations': 5, 'nonzero': 1, 'eligible_nodes': 20}}]})
    assert correction['best']['improvement'] == ratio(20, 100)
    assert correction['decision'] == 'candidate'
    correction['tables'][0]['stats']['nonzero'] = 0
    assert correction_report({'tables': correction['tables']})['decision'] == '見送り'
    assert not rate_pass(ratio(0, 0)) and rate_pass(ratio(4, 5), 0.8)
    n = improving_report([(1, 1, 1, 0, 0, 40, 0, 0, 0), (3, 1, 0, 0, 0, 0, None, 1, 1)], f, {1: 50, 2: 150, 3: 150})
    assert n['all']['activation'] == ratio(2, 2)
    # Mutating only the independent reference must change process exit status.
    with tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parent) as directory:
        path = Path(directory) / 'verify.txt'
        path.write_text('captures 1 1 1\nquiets 1 0 0\n')
        command = [sys.executable, str(Path(__file__).resolve()), '--verify-only', str(path)]
        assert subprocess.run(command, capture_output=True).returncode == 0
        path.write_text('captures 1 1 0\nquiets 1 0 1\n')
        assert subprocess.run(command, capture_output=True).returncode == 1
    print('self-test: passed (loss, eligibility, ties, strict thresholds, activation, LMR, verification mutation)')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--samples', type=Path)
    parser.add_argument('--lmr', type=Path)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--text', type=Path)
    parser.add_argument('--verify-only', type=Path)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.verify_only:
        result = verification_report(args.verify_only)
        print(json.dumps(result, indent=2))
        return 0 if result['all']['passes'] else 1
    if any(value is None for value in (args.samples, args.lmr, args.json, args.text)):
        parser.error('--samples, --lmr, --json and --text are required')
    result = evaluate(args.samples, args.lmr)
    args.json.write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')
    args.text.write_text(human_text(result))
    print(f"results: {args.json}, {args.text}")
    if not result['verification']['all']['passes']:
        print('verification failed: conditional agreement is below 80% or undefined', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
