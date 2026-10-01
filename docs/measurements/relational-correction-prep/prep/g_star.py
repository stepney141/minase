"""FM 1/4の保存対局と教師標本から、全域の共通層でG*を校正する。"""
import json
import subprocess
import sys

from common import BASE, DATA, OUTPUT, WT, identity, invocation, write_json

FM = BASE / 'fm-eval-tools'
FM_COMMIT = '01aedf589696fe228b72a6bfa98576ee06cd8ef6'
CANDIDATE = '3143b082ac15efceb889ba35ce79ed37abe8594d'
sys.path.insert(0, str(FM / 'tools/train/pst'))
from fm_distribution_comparison import compare, load, validate_evaluators
from mnsd import map_records, read_header, write_mnsd


class Commands:
    def __init__(self, destination):
        self.destination = destination
        self.rows = []

    def run(self, argv, cwd, *, check=True):
        argv = list(map(str, argv))
        number = len(self.rows)
        result = subprocess.run(argv, cwd=cwd, capture_output=True)
        prefix = self.destination / f'command-{number:02}'
        prefix.with_suffix('.stdout').write_bytes(result.stdout)
        prefix.with_suffix('.stderr').write_bytes(result.stderr)
        self.rows.append({'cwd': str(cwd), 'argv': argv,
                          'returncode': result.returncode,
                          'stdout': str(prefix.with_suffix('.stdout')),
                          'stderr': str(prefix.with_suffix('.stderr'))})
        # 途中で失敗しても、失敗を含むコマンド列を残す。
        with (self.destination / 'commands.json').open('w') as stream:
            json.dump(self.rows, stream, indent=2)
            stream.write('\n')
        if check:
            result.check_returncode()
        return result


def build(commands, worktree):
    commands.run(['cargo', 'build', '--offline', '--release', '--target-dir',
                  worktree / 'target', '--bin', 'pst_probe', '--bin', 'match_positions'],
                 worktree)
    return worktree / 'target/release'


def main():
    destination = OUTPUT / 'g-star'
    destination.mkdir(parents=True, exist_ok=False)
    commands = Commands(destination)
    head = commands.run(['git', 'rev-parse', 'HEAD'], FM).stdout.decode().strip()
    if head != FM_COMMIT:
        raise ValueError(f'fm-eval-tools HEAD changed: {head}')
    if commands.run(['git', 'status', '--porcelain', '--untracked-files=no'], FM).stdout:
        raise ValueError('fm-eval-tools has modified tracked files')
    match_run = DATA / 'matches/fm-quarter-vs-pst-stc'
    manifest = json.loads((match_run / 'manifest.json').read_text())
    candidate_identity = manifest['candidate']['identity']
    if candidate_identity['kind'] != 'commit' or candidate_identity['hash'] != CANDIDATE:
        raise ValueError('unexpected match candidate identity')
    weights = destination / 'fm-quarter-pst.bin'
    weights.write_bytes(commands.run(['git', 'show', f'{CANDIDATE}:nets/pst.bin'], FM).stdout)
    teacher = DATA / 'fm-eval-archive/fm-cand-lam1e-4/diagnostics/diagnostic-samples.bin'
    header = read_header(teacher)
    compatibility = destination / 'compatibility-sample.bin'
    write_mnsd(compatibility, map_records(teacher)[:1].copy(), seed=header.seed,
               network_checksum=header.network_checksum, rule_set=header.rule_set,
               generation_commit=header.generation_commit, teacher_nodes=header.teacher_nodes)
    binary_dir = build(commands, FM)
    probe_argv = [binary_dir / 'pst_probe', '--pst', weights, '--positions', compatibility]
    first_probe = commands.run(probe_argv, FM, check=False)
    probe_worktree = FM
    if first_probe.returncode != 0:
        if b'MNPT' not in first_probe.stderr:
            first_probe.check_returncode()
        # 指示書が明示した、重みを読めない場合だけの別コミットでのビルド。
        # その作成や再検査が失敗すれば停止する。
        probe_worktree = BASE / 'fm-quarter-probe-3143b08'
        commands.run(['git', 'worktree', 'add', '--detach', probe_worktree, CANDIDATE], FM)
        binary_dir = build(commands, probe_worktree)
        probe_argv[0] = binary_dir / 'pst_probe'
        first_probe = commands.run(probe_argv, probe_worktree)
    probe_rows = json.loads(first_probe.stdout)
    if len(probe_rows) != 1 or not {'eval', 'eval_pst'} <= probe_rows[0].keys():
        raise ValueError('probe lacks final and PST-only evaluations')
    match_dir = destination / 'match-positions'
    commands.run([binary_dir / 'match_positions', '--run-dir', match_run,
                  '--max-pairs', '16', '--pst', weights, '--output-dir', match_dir],
                 probe_worktree)
    metadata = json.loads((match_dir / 'metadata.json').read_text())
    counts = {'saved': header.record_count, 'match': metadata['records']}
    diagnostic_dirs = {}
    for name, source in (('saved', teacher), ('match', match_dir / 'positions.mnsd')):
        diagnostic_dirs[name] = destination / f'{name}-static-all'
        commands.run([sys.executable, '-B', FM / 'tools/train/pst/fm_strength_diagnostics.py',
                      '--positions', source, '--probe', binary_dir / 'pst_probe',
                      '--pst', weights, '--output-dir', diagnostic_dirs[name],
                      '--sample-size', counts[name], '--seed', '1'], FM)
    evaluators = validate_evaluators(diagnostic_dirs['saved'], diagnostic_dirs['match'])
    # CLIは中央範囲も併記するため、校正では同じ比較器の全域の関数だけを呼ぶ。
    comparison = compare(load(diagnostic_dirs['saved']), load(diagnostic_dirs['match']),
                         center_only=False)
    if comparison['source_counts'] != counts:
        raise ValueError('not all source records were compared')
    write_json(destination / 'distribution-comparison.json', comparison)
    g = comparison['difference']
    inputs = [teacher, match_run / 'manifest.json', weights,
              match_dir / 'positions.mnsd', match_dir / 'metadata.json']
    inputs += [match_run / source['file'] for source in metadata['sources']]
    inputs += [directory / filename for directory in diagnostic_dirs.values()
               for filename in ('sample.bin', 'probe.json', 'report.json')]
    report = {
        'candidate_commit': CANDIDATE, 'fm_tools_commit': head,
        'probe_worktree': str(probe_worktree),
        'G_FM_quarter_cp': g, 'G_star_cp': max(0.0, g),
        'definition': 'G_FM_quarter is measured with quarter weights; do not divide G again',
        'coverage': comparison['coverage'], 'sample_counts': counts,
        'common_counts': comparison['common_counts'],
        'common_stratum_count': comparison['common_stratum_count'],
        'match_weighted_mean_cp': comparison['match_common_mean'],
        'teacher_weighted_mean_cp': comparison['saved_standardized_mean'],
        'coverage_at_least_90_percent': comparison['coverage']['match'] >= 0.9,
        'strata': '5 phase bands x floor(PST-only evaluation / 500 cp); full range',
        'correction': 'final clamped evaluation minus PST-only evaluation, side-to-move cp',
        'weights': 'match frequencies normalized within common strata',
        'inputs': [identity(path) for path in inputs],
        'tools': [identity(binary_dir / name) for name in ('pst_probe', 'match_positions')]
                 + [identity(FM / 'tools/train/pst' / name) for name in
                    ('fm_strength_diagnostics.py', 'fm_distribution_comparison.py', 'taper.py')],
        'evaluator_checksums': evaluators,
        'command': invocation(), 'commands': commands.rows,
        'comparison_call': 'fm_distribution_comparison.compare(saved, match, center_only=False)',
    }
    write_json(destination / 'g_star.json', report)
    print(json.dumps({key: report[key] for key in
                      ('G_FM_quarter_cp', 'G_star_cp', 'coverage', 'sample_counts')}, indent=2))


if __name__ == '__main__':
    main()
