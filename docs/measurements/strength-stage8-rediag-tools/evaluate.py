#!/usr/bin/env python3
"""Stage 8 rediagnosis: loss means lost good moves / all eligible good moves.

Apply the adjacent diagnostic.patch to 7784ef0c0757d2d7c80cc8793aa6c3202c932828.
Build only with:
  nice -n 19 env CARGO_TARGET_DIR=target/diag cargo build --release --bin bench -j 2
Run each bench separately with nice -n 19, --depth 6 --threads 1. Run A sets
MINASE_STAGE8_DIAG to a directory; the matching verification run also sets
MINASE_STAGE8_VERIFY=1. Run B sets MINASE_STAGE8_LMR_DIAG to a file path.
Pass the verification directory to --samples and the B file to --lmr.

Samples are whitespace separated, without headers. Capture columns are depth,
index, best_ok, royal_ok, improving, SEE, SEE_negative, promote, pruned, good.
Quiet columns are depth, index, history, killer, promote, best_ok, royal_ok,
improving, alpha_minus_eval, reduction, good. LMR columns are depth, index,
history, current_reduction, new_reduction, new_good, current_good, full_good,
overlaps_sample2. '-' denotes undefined improving, SEE, or an unsearched outcome.
Ratios with zero denominators are null, never zero. Move activation counts
unions once, including the separately counted LMR population outside sample 2.
Correction selection uses the common observation set's pre-update errors.
The nonzero gate uses eligible-node references; common nonzero predictions and
variant-C-population errors are reference metrics only. For correction alone,
use --correction-only with correction.json, --json and --text.
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
        if any(r[8] not in (0, 1) or (r[-1] is None) != bool(r[8]) for r in group):
            raise ValueError('pruned captures must have unknown outcomes; searched captures must have outcomes')
        targets = [r for r in group if r[2] and r[3] and not r[8]]
        candidates = [metric(targets, lambda r: r[5] is not None and r[5] < -m, {'m': m})
                      for m in (0, pawn // 2, pawn, 2 * pawn, 3 * pawn)]
        selected, reason = choose(candidates, len(targets), sum(r[-1] for r in targets), lambda c: (c['m'],))
        report[depth] = dict(targets=len(targets), good=sum(r[-1] for r in targets),
                            exclusions_see_pruned=sum(r[8] for r in group),
                            exclusions_best_score=sum(not r[2] for r in group),
                            exclusions_royal=sum(not r[3] for r in group),
                            exclusions_union=sum(not (r[2] and r[3]) or r[8] for r in group),
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


def improving_move_report(quiets, futility, margins, counts, lmr_rows, lmr_counts, apply_lmr):
    """Union of expanded futility quiets and quiets passed to lmr_reduction.

    Sample 2 excludes TT moves; quiet_expanded includes those protected moves.
    Every LMR quiet in an eligible node is in sample 2. Outside counters cover
    its complement (PV, deeper nodes, and mate windows), never the overlap.
    First moves and capped reductions remain in the denominator but cannot
    contribute an actual reduction change. B's last column marks sample-2 overlap.
    """
    for name in ('quiet_expanded', 'lmr_outside_quiets', 'lmr_changed_outside_quiets'):
        if counts.get(name, {}) != lmr_counts.get(name, {}):
            raise ValueError(f'A/B population mismatch: {name}')
    depths = sorted({1, 2, 3, *[r[0] for r in quiets],
                     *[int(d) for name in ('lmr_outside_quiets', 'quiet_expanded')
                       for d in counts.get(name, {})]})
    report = {}
    for depth, group in depth_groups(quiets, depths):
        expanded = count_at(counts, 'quiet_expanded', depth)
        outside = count_at(counts, 'lmr_outside_quiets', depth)
        outside_changed = count_at(counts, 'lmr_changed_outside_quiets', depth)
        if expanded < len(group) or outside_changed > outside:
            raise ValueError('invalid move population counts')
        overlap = sum(not r[3] for r in group)
        fut = lmr = both = potential_overlap = 0
        for r in group:
            selection = futility[str(r[0])]['selected']
            pruned = bool(selection and not r[4] and r[5] and r[6] and r[7] == 0
                          and r[8] >= margins[r[0]] * selection['quarters'] // 4)
            # LMR_MAX_REDUCTION=3; the depth cap and first-move exclusion are
            # the design's unchanged limits, not an estimated activation.
            reduced = bool(not r[3] and r[7] == 0 and r[1] >= 1
                           and r[9] < min(3, max(0, r[0] - 2)))
            potential_overlap += reduced
            fut += pruned
            lmr += reduced and apply_lmr
            both += pruned and reduced and apply_lmr
        b_rows = [r for r in lmr_rows if depth == 'all' or str(r[0]) == depth]
        if sum(r[8] == 0 for r in b_rows) != outside_changed or sum(r[8] == 1 for r in b_rows) != potential_overlap:
            raise ValueError('LMR samples disagree with move activation counts')
        lmr += outside_changed if apply_lmr else 0
        denominator = expanded + outside
        activation = ratio(fut + lmr - both, denominator)
        report[depth] = dict(
            activation=activation, futility=ratio(fut, denominator), lmr=ratio(lmr, denominator),
            futility_only=ratio(fut - both, denominator), lmr_only=ratio(lmr - both, denominator),
            both=ratio(both, denominator), at_least_5_percent=rate_pass(activation),
            futility_population=expanded, lmr_population=overlap + outside,
            population_overlap=overlap, lmr_outside_sample2=outside,
            lmr_changed_outside_sample2=outside_changed,
            sample2=len(group), protected_tt_quiets=expanded - len(group),
            potential_lmr_changes=potential_overlap + outside_changed,
            extra_lmr_applied=apply_lmr)
    return report


def correction_report(raw):
    identities = {(t['variant'], t['key'], t['denominator']) for t in raw['tables']}
    expected = {(v, k, d) for v in ('A', 'B', 'C') for k in ('royals', 'material', 'combined') for d in (32, 64, 128)}
    if identities != expected or len(raw['tables']) != 27:
        raise ValueError('expected exactly 27 correction tables')
    populations = {(t['stats']['observations'], t['stats']['absolute_error'],
                    t['stats']['common_observations'], t['stats']['common_absolute_error'],
                    t['stats']['variant_c_observations'], t['stats']['variant_c_absolute_error']) for t in raw['tables']}
    if len(populations) != 1:
        raise ValueError('correction tables must share evaluation populations')
    types = {f'{b}:capture={c}:attacked={a}': raw['observation_types'].get(f'{b}:capture={c}:attacked={a}', 0)
             for b in ('exact', 'upper', 'lower') for c in (0, 1) for a in (0, 1)}
    observations, _, common, _, variant_c, _ = next(iter(populations))
    if sum(types.values()) != observations or sum(types[f'{b}:capture=0:attacked=0'] for b in ('exact', 'upper', 'lower')) != common:
        raise ValueError('observation breakdown disagrees with correction population')
    if variant_c != common + types['upper:capture=1:attacked=0']:
        raise ValueError('observation breakdown disagrees with variant C population')
    tables = []
    for table in raw['tables']:
        s = table['stats']
        if s['updates'] != {'A': observations, 'B': common, 'C': variant_c}[table['variant']]:
            raise ValueError('correction update population disagrees with variant')
        improvement = ratio(s['absolute_error'] - s['corrected_error'], s['absolute_error'])
        common_improvement = ratio(s['common_absolute_error'] - s['common_corrected_error'], s['common_absolute_error'])
        variant_c_improvement = ratio(s['variant_c_absolute_error'] - s['variant_c_corrected_error'], s['variant_c_absolute_error'])
        nonzero = ratio(s['nonzero'], s['eligible_nodes'])
        common_nonzero = ratio(s['common_nonzero'], s['common_observations'])
        tables.append(dict(table, improvement=improvement, common_improvement=common_improvement,
                           variant_c_improvement=variant_c_improvement,
                           reuse_rate=ratio(s['reuse'], s['observations']), nonzero_rate=nonzero,
                           common_nonzero_rate=common_nonzero, at_least_5_percent=rate_pass(nonzero)))
    defined = [t for t in tables if t['common_improvement']['denominator']]
    best = max(defined, key=lambda t: (
        Fraction(t['common_improvement']['numerator'], t['common_improvement']['denominator']),
        -('A', 'B', 'C').index(t['variant']),
        -('royals', 'material', 'combined').index(t['key']),
        -t['denominator'])) if defined else None
    reasons = []
    if best is None:
        reasons.append('no_common_error_population')
    else:
        if not rate_pass(best['common_improvement'], 0.1):
            reasons.append('common_improvement_below_10_percent')
        if not rate_pass(best['nonzero_rate']):
            reasons.append('nonzero_reference_rate_below_5_percent')
    return {'tables': tables, 'best': best, 'decision': '見送り' if reasons else 'candidate',
            'reasons': reasons, 'observation_types': types}


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
    captures = read_rows(samples / 'captures.txt', 10)
    quiets = read_rows(samples / 'quiets.txt', 11)
    nodes = read_rows(samples / 'nodes.txt', 9)
    correction = json.loads((samples / 'correction.json').read_text())
    counts = json.loads((samples / 'counts.json').read_text())['counts']
    pawn = correction['pawn_value']
    futility, margins = futility_report(quiets, pawn)
    lmr_rows = read_rows(lmr, 9)
    lmr_counts = json.loads(lmr.with_suffix('.counts.json').read_text())
    lmr_result = lmr_report(lmr_rows, lmr_counts)
    apply_lmr = lmr_result['all']['exceeds_10_points'] is not True
    return dict(pawn_value=pawn,
                definitions={'loss': 'lost good results / all eligible good results',
                             'activation': 'selected per-depth candidates / pre-exclusion design population',
                             'quantiles': 'nearest rank',
                             **correction_definitions(),
                             'improving_moves': 'quiet_expanded + LMR outside sample 2; numerator=futility + LMR - both; overlapping moves count once',
                             'improving_population_overlap': 'all eligible-node LMR quiets occur in sample 2; protected TT quiets counted by quiet_expanded; PV/deep/mate-window LMR quiets counted separately by depth',
                             'improving_first_moves': 'LMR judgments on first moves and capped moves count in the denominator, but never as reduction changes',
                             'improving_lmr_gate': 'remove extra LMR if all-depth loss increases by more than 10 percentage points; undefined loss is disclosed and does not itself trigger removal',
                             'capture_columns': 'depth index best_ok royal_ok improving see see_negative promote pruned good; pruned good is unknown and excluded from targets and good counts',
                             'lmr_columns': 'depth index history current_reduction new_reduction new_good current_good full_good overlaps_sample2',
                             'pooled_selection': 'all.selected is descriptive; actual activation applies per-depth selections'},
                see=see_report(captures, pawn, counts), quiet=quiet_report(quiets, counts), futility=futility,
                lmr=lmr_result,
                lmr_exclusions_by_depth=lmr_counts,
                improving_moves=improving_move_report(quiets, futility, margins, counts, lmr_rows, lmr_counts, True),
                improving_moves_applied=improving_move_report(quiets, futility, margins, counts, lmr_rows, lmr_counts, apply_lmr),
                improving=improving_report(nodes, futility, margins), correction=correction_report(correction),
                verification=verification_report(samples / 'verify.txt', captures, quiets))


def human_text(report):
    lines = ['Stage 8 rediagnosis tools, bench depth 6 smoke check', '',
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
                lines.append(f"Excluded because SEE pruned (unknown outcome): {group['exclusions_see_pruned']}; best score: {group['exclusions_best_score']}; royal attack: {group['exclusions_royal']}; union: {group['exclusions_union']}; undefined SEE among targets: {group['see_none']}; among all samples: {group['see_none_all_samples']}.")
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
    lines.extend(['Improving activation per move', '', report['definitions']['improving_moves'],
                  report['definitions']['improving_population_overlap'], report['definitions']['improving_first_moves'],
                  report['definitions']['improving_lmr_gate'], ''])
    table(['Depth', 'Union', 'Futility', 'LMR', 'Both', 'Futility population', 'LMR population', 'Population overlap', 'Union after LMR loss gate', 'LMR applied', 'At least 5%'],
          [[d, fmt(g['activation']), fmt(g['futility']), fmt(g['lmr']), fmt(g['both']), g['futility_population'],
            g['lmr_population'], g['population_overlap'], fmt(report['improving_moves_applied'][d]['activation']),
            report['improving_moves_applied'][d]['extra_lmr_applied'], g['at_least_5_percent']]
           for d, g in report['improving_moves'].items()])
    lines.extend(['Improving activation per node (reference only)', ''])
    table(['Depth', 'Union', 'Futility only', 'LMR only', 'Both', 'At least 5%'],
          [[d, fmt(g['activation']), fmt(g['futility_only']), fmt(g['lmr_only']), fmt(g['both']), g['at_least_5_percent']] for d, g in report['improving'].items()])
    lines.append(correction_text(report))
    lines.extend(['', 'Search verification', ''])
    table(['Group', 'Samples', 'Reference good / recorded good', 'Recorded good / reference good', 'Passes 80%'],
          [[d, g['samples'], fmt(g['reference_given_recorded_good']), fmt(g['recorded_given_reference_good']), g['passes']]
           for d, g in report['verification'].items()])
    return '\n'.join(lines) + '\n'


def correction_definitions():
    return {
        'correction_reference_population': 'eligible nodes; the 5% gate is nonzero / eligible_nodes; current LMR does not use static evaluation',
        'correction_common_population': 'direction-confirming non-mate observations with a noncapture best move and no royal attack; predictions read before update',
        'correction_variant_c_population': 'direction-confirming non-mate observations with no royal attack and either an upper bound or a noncapture best move; reference only, never used for selection',
        'correction_selection': 'maximize common error improvement; require at least 10% common improvement and 5% eligible-node nonzero references; preserve existing exact ties: A/B/C, royals/material/combined, then 32/64/128',
    }


def correction_text(report):
    correction = report['correction']
    lines = ['Static evaluation correction', '']
    lines.extend(correction_definitions().values())
    lines.extend(['', 'Variant | Key | D | Common improvement (%) | Common nonzero (%) | Eligible-node nonzero (%) | Common count | Common absolute error | Common corrected error | C count | C absolute error | C corrected error | C improvement (%)',
                  '--- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | ---'])
    def percent(value):
        return 'undefined' if value['ratio'] is None else f"{100 * value['ratio']:.6f}"
    for t in correction['tables']:
        stats = t['stats']
        lines.append(' | '.join(map(str, [
            t['variant'], t['key'], t['denominator'], percent(t['common_improvement']),
            percent(t['common_nonzero_rate']), percent(t['nonzero_rate']),
            stats['common_observations'], stats['common_absolute_error'], stats['common_corrected_error'],
            stats['variant_c_observations'], stats['variant_c_absolute_error'], stats['variant_c_corrected_error'],
            percent(t['variant_c_improvement'])])))
    best = correction['best']
    if best:
        rate = best['nonzero_rate']
        lines.extend(['', f"Best variant={best['variant']}, key={best['key']}, D={best['denominator']}; decision={correction['decision']}.",
                      f"Nonzero references: {rate['numerator']}/{rate['denominator']} ({percent(rate)}%)."])
    else:
        lines.extend(['', 'No correction observations.'])
    lines.append(f"Deferral reasons: {correction['reasons']}.")
    lines.extend(['', 'Boundary / capture / royal attack | Observations', '--- | ---'])
    lines.extend(f'{kind} | {count}' for kind, count in correction['observation_types'].items())
    return '\n'.join(lines) + '\n'


def rediagnosis_self_test():
    # Instruction 1: unknown pruned results cannot enter target or loss totals.
    captures = [(1, 1, 1, 1, 0, -100, 1, 0, 1, None),
                (1, 2, 1, 1, 0, 0, 0, 0, 0, 1)]
    see = see_report(captures, 100, {'capture_inspected': {'1': 2}})['1']
    assert (see['targets'], see['good'], see['exclusions_see_pruned']) == (1, 1, 1)

    def rejects(call):
        try:
            call()
        except ValueError:
            return
        raise AssertionError('inconsistent diagnostic input was accepted')

    rejects(lambda: see_report([captures[0][:-1] + (0,)], 100, {}))
    # Instruction 2: handwritten overlapping sets. TT quiets, first moves,
    # capped moves, promotions, unsafe nodes and undefined improving are distinct.
    quiets = [
        (3, 1, 0, 0, 0, 1, 1, 0, 100, 0, 0),  # both
        (3, 2, 0, 1, 0, 1, 1, 0, 100, 0, 0),  # futility only (killer)
        (3, 3, 0, 0, 1, 1, 1, 0, 100, 0, 0),  # LMR only (promotion)
        (3, 4, 0, 0, 0, 1, 1, 0, 0, 1, 0),    # capped
        (3, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0),    # first
        (3, 5, 0, 0, 0, 1, 1, 1, 100, 0, 0),  # improving
        (3, 6, 0, 0, 0, 1, 1, None, 100, 0, 0),
        (3, 7, 0, 0, 0, 0, 1, 0, 100, 1, 0),  # mate-band best
        (3, 8, 0, 0, 0, 1, 0, 0, 100, 1, 0),  # royal attack
    ]
    futility = {str(d): {'selected': {'quarters': 2}} for d in (1, 2, 3)}
    margins = {1: 50, 2: 150, 3: 150}
    counts = {'quiet_expanded': {'3': 10}, 'lmr_outside_quiets': {'3': 2, '4': 3, '6': 1},
              'lmr_changed_outside_quiets': {'3': 1, '4': 2}}
    lmr_rows = [(3, 1, 0, 0, 1, 1, 1, 1, 1), (3, 3, 0, 0, 1, 1, 1, 1, 1),
                (3, 1, 0, 0, 1, 1, 1, 1, 0), (4, 1, 0, 0, 1, 1, 1, 1, 0),
                (4, 2, 0, 0, 1, 1, 1, 1, 0)]
    args = (quiets, futility, margins, counts, lmr_rows, counts)
    result = improving_move_report(*args, True)
    assert result['all']['activation'] == ratio(6, 16)
    assert result['all']['futility'] == ratio(2, 16)
    assert result['all']['lmr'] == ratio(5, 16)
    assert result['all']['both'] == ratio(1, 16)
    assert result['all']['population_overlap'] == 8
    assert result['all']['protected_tt_quiets'] == 1
    assert result['3']['activation'] == ratio(4, 12)
    assert result['4']['activation'] == ratio(2, 3)
    assert result['6']['activation'] == ratio(0, 1)
    assert improving_move_report(*args, False)['all']['activation'] == ratio(2, 16)
    rejects(lambda: improving_move_report(quiets, futility, margins, counts, lmr_rows[:-1], counts, True))
    rejects(lambda: improving_move_report(quiets, futility, margins, counts, lmr_rows, {}, True))
    empty = improving_move_report([], futility, margins, {}, [], {}, True)['all']
    assert empty['activation'] == ratio(0, 0) and not empty['at_least_5_percent']

    # Variant C instructions: common evaluation is B's set; C adds only
    # unattacked upper-bound captures to its updates and reference population.
    raw = {'tables': [], 'observation_types': {
        'lower:capture=0:attacked=0': 20, 'lower:capture=1:attacked=0': 10,
        'upper:capture=1:attacked=0': 7, 'upper:capture=1:attacked=1': 3}}
    for v in ('A', 'B', 'C'):
        for k in ('royals', 'material', 'combined'):
            for d in (32, 64, 128):
                raw['tables'].append(dict(variant=v, key=k, denominator=d, stats=dict(
                    observations=40, absolute_error=2000, corrected_error=100 if v == 'A' else 1900,
                    common_observations=20, common_absolute_error=1000, common_corrected_error=1100,
                    variant_c_observations=27, variant_c_absolute_error=1500,
                    variant_c_corrected_error=0 if v == 'A' else 1600,
                    common_nonzero=0, nonzero=5, eligible_nodes=100, reuse=30,
                    updates={'A': 40, 'B': 20, 'C': 27}[v])))
    winner = next(t for t in raw['tables'] if (t['variant'], t['key'], t['denominator']) == ('C', 'material', 64))
    winner['stats']['common_corrected_error'] = 900
    report = correction_report(raw)
    assert len(report['tables']) == 27 and len(report['observation_types']) == 12
    assert (report['best']['variant'], report['best']['key'], report['best']['denominator']) == ('C', 'material', 64)
    assert report['best']['common_improvement'] == ratio(100, 1000)
    assert report['best']['variant_c_improvement'] == ratio(-100, 1500)
    assert report['best']['common_nonzero_rate'] == ratio(0, 20)
    assert report['best']['nonzero_rate'] == ratio(5, 100)
    assert report['decision'] == 'candidate'  # inclusive 10% and 5%; common zero is irrelevant
    winner['stats']['common_corrected_error'] = 901
    assert correction_report(raw)['reasons'] == ['common_improvement_below_10_percent']
    winner['stats']['common_corrected_error'] = 900
    winner['stats']['nonzero'] = 4
    winner['stats']['common_nonzero'] = 20
    assert correction_report(raw)['reasons'] == ['nonzero_reference_rate_below_5_percent']
    winner['stats']['eligible_nodes'] = 0
    winner['stats']['nonzero'] = 0
    assert correction_report(raw)['reasons'] == ['nonzero_reference_rate_below_5_percent']
    winner['stats']['common_observations'] = 19
    rejects(lambda: correction_report(raw))
    winner['stats']['common_observations'] = 20
    winner['stats']['variant_c_observations'] = 28
    rejects(lambda: correction_report(raw))
    winner['stats']['variant_c_observations'] = 27
    raw['observation_types']['upper:capture=1:attacked=0'] = 6
    raw['observation_types']['upper:capture=1:attacked=1'] = 4
    rejects(lambda: correction_report(raw))
    raw['observation_types']['upper:capture=1:attacked=0'] = 7
    raw['observation_types']['upper:capture=1:attacked=1'] = 3
    winner['stats']['updates'] = 20
    rejects(lambda: correction_report(raw))
    winner['stats']['updates'] = 27
    rejects(lambda: correction_report(dict(raw, tables=raw['tables'][:-1])))
    # The existing tie rule (32 before 64 before 128) must not depend on file order.
    for t in raw['tables']:
        t['stats']['common_corrected_error'] = 900
    tied = correction_report(dict(raw, tables=list(reversed(raw['tables']))))['best']
    assert (tied['variant'], tied['key'], tied['denominator']) == ('A', 'royals', 32)
    for t in raw['tables']:
        t['stats']['common_absolute_error'] = 0
        t['stats']['common_corrected_error'] = 0
    assert correction_report(raw)['reasons'] == ['no_common_error_population']
    raw['observation_types']['lower:capture=1:attacked=0'] = 11
    rejects(lambda: correction_report(raw))


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
    captures = [(1, 1, 1, 1, 0, -100, 1, 0, 0, 0)] * 900 + [(1, 1, 1, 1, 0, 0, 0, 0, 0, 1)] * 100
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
    rediagnosis_self_test()
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
    print('self-test: passed (loss, eligibility, ties, strict thresholds, activation union, 27 correction tables, common improvement and eligible-node reference gates, LMR, verification mutation)')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--samples', type=Path)
    parser.add_argument('--lmr', type=Path)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--text', type=Path)
    parser.add_argument('--verify-only', type=Path)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--correction-only', type=Path, metavar='CORRECTION_JSON')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.verify_only:
        result = verification_report(args.verify_only)
        print(json.dumps(result, indent=2))
        return 0 if result['all']['passes'] else 1
    if args.correction_only:
        if args.json is None or args.text is None:
            parser.error('--correction-only requires --json and --text')
        raw = json.loads(args.correction_only.read_text())
        result = dict(pawn_value=raw['pawn_value'], definitions=correction_definitions(),
                      correction=correction_report(raw))
        args.json.write_text(json.dumps(result, indent=2, ensure_ascii=False) + '\n')
        args.text.write_text(correction_text(result))
        print(f'results: {args.json}, {args.text}')
        return 0
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
