"""保存対局と固定教師標本から局所2駒表の自己選択の差Gを測る。"""
import argparse
import json
from pathlib import Path
import subprocess

from common import WT, identity, invocation, write_json
from phase3_common import evaluate, teacher_records, zero_weights, map_records
from mnpt_v3 import upgrade
# G*の校正で使った関数そのものを呼ぶ。独自の層別・重み付けは行わない。
from g_star import compare, load, FM

G_STAR_CP = 88.25442880689863


def comparison(saved, match):
    try:
        return compare(saved, match, center_only=False)
    except ValueError as error:
        if str(error) != 'no common strata':
            raise
        # 共通層が空のときも判定不能として件数と被覆率を保存する。
        return {'difference': None, 'coverage': {'saved': 0.0, 'match': 0.0},
                'source_counts': {'saved': len(saved[0]), 'match': len(match[0])},
                'common_counts': {'saved': 0, 'match': 0}, 'common_stratum_count': 0,
                'strata': [], 'reason': 'no common strata'}


def decision(candidate, baseline):
    base_coverage = baseline['coverage']['match']
    threshold = 0.9 if base_coverage >= 0.9 else base_coverage - 0.05
    covered = candidate['coverage']['match']
    g = candidate['difference']
    determinate = g is not None and covered >= threshold
    return {'G_cp': g, 'G_star_cp': G_STAR_CP,
            'G_le_G_star': None if g is None else g <= G_STAR_CP,
            'coverage_at_least_90_percent': covered >= 0.9,
            'determinate_at_90_percent': g is not None and covered >= 0.9,
            'coverage_threshold': threshold,
            'baseline_coverage_rule_applied': base_coverage < 0.9,
            'determinate': determinate,
            'meets_criterion': determinate and g <= G_STAR_CP,
            'reason': None if determinate else '判定不能: 共通層がないか被覆率が閾値未満',
            'strict_90_percent_reason': None if g is not None and covered >= 0.9
                else '判定不能: 共通層がないか被覆率が90%未満'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-dir', type=Path, required=True)
    parser.add_argument('--weights', type=Path, required=True)
    parser.add_argument('--teacher', type=Path, required=True)
    parser.add_argument('--probe', type=Path, required=True)
    parser.add_argument('--extractor', type=Path, required=True)
    parser.add_argument('--max-pairs', type=int, default=16)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args()
    if args.max_pairs <= 0:
        parser.error('--max-pairs must be positive')
    destination = args.output_dir.resolve()
    destination.mkdir(parents=True, exist_ok=False)
    zero = destination / 'pst-only.bin'
    model = zero_weights(args.weights, zero)
    manifest_path = args.run_dir / 'manifest.json'
    manifest = json.loads(manifest_path.read_text())
    baseline_identity = manifest['baseline']['identity']
    if baseline_identity['kind'] != 'commit':
        raise ValueError('baseline must be a commit')
    # S0は版2。明示的に零表つき版3へ変換し、候補のPSTと一致することを検査する。
    archived = subprocess.run(['git', 'show', f"{baseline_identity['hash']}:nets/pst.bin"],
                              cwd=WT, capture_output=True, check=True).stdout
    baseline_archive = destination / 'baseline-v2.bin'
    baseline_archive.write_bytes(archived)
    if upgrade(archived, model.radius) != zero.read_bytes():
        raise ValueError('candidate PST-only weights differ from baseline S0')
    teacher, paths = teacher_records(args.teacher)
    teacher_dir = destination / 'teacher'
    evaluate(args.probe, args.weights, zero, teacher, teacher_dir)
    saved = load(teacher_dir)
    comparisons, metadata = {}, {}
    commands = []
    # 基準の被覆率を先に求め、候補判定に使う閾値を定める。
    for side in ('baseline', 'candidate'):
        extracted = destination / f'{side}-positions'
        argv = [str(args.extractor.resolve()), '--run-dir', str(args.run_dir.resolve()),
                '--pst', str(args.weights.resolve()), '--max-pairs', str(args.max_pairs),
                '--output-dir', str(extracted)]
        if side == 'baseline':
            argv.append('--baseline')
        result = subprocess.run(argv, cwd=WT, capture_output=True, text=True, check=True)
        commands.append({'argv': argv, 'stdout': result.stdout, 'stderr': result.stderr})
        metadata[side] = json.loads((extracted / 'metadata.json').read_text())
        records = map_records(extracted / 'positions.mnsd')
        directory = destination / side
        evaluate(args.probe, args.weights, zero, records, directory)
        match = load(directory)
        if side == 'baseline':
            comparisons[side] = comparison((saved[0], saved[1], saved[2] * 0),
                                          (match[0], match[1], match[2] * 0))
        else:
            comparisons[side] = comparison(saved, match)
    inputs = [args.weights, args.teacher, manifest_path, baseline_archive, *paths]
    inputs += [args.run_dir / row['file'] for row in metadata['candidate']['sources']]
    report = {**decision(comparisons['candidate'], comparisons['baseline']),
              'coverage': comparisons['candidate']['coverage'],
              'sample_counts': comparisons['candidate']['source_counts'],
              'candidate': comparisons['candidate'], 'baseline': comparisons['baseline'],
              'selection': metadata, 'inputs': [identity(p) for p in inputs],
              'tools': [identity(p) for p in (args.probe, args.extractor, Path(__file__),
                        Path(__file__).with_name('phase3_common.py'),
                        FM / 'tools/train/pst/fm_distribution_comparison.py',
                        WT / 'tools/train/pst/taper.py')],
              'comparison_call': 'g_star.compare(saved, match, center_only=False)',
              'correction': 'final clamped evaluation minus PST-only evaluation, side-to-move cp',
              'command': invocation(), 'extraction_commands': commands}
    write_json(destination / 'g_candidate.json', report)
    print(json.dumps({k: report[k] for k in ('G_cp', 'coverage', 'sample_counts',
                     'coverage_threshold', 'determinate', 'meets_criterion')}, indent=2))


if __name__ == '__main__':
    main()
