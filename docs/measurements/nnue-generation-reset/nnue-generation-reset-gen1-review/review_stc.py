"""見直しの2本のSTCを統合前の作業ツリーから順に実行する。"""
import subprocess
import sys
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
PREMERGE = ROOT / 'data/worktrees/nnue-generation-reset-premerge'
MINASE = ROOT / 'data/nnue-generation-reset/bin/minase'
MATCHES = ROOT / 'data/matches'
GEN0 = '6f874692fb91544c94394ca0e8df25299ef2e02d'
LAMBDA1 = '5f81bb71dfed92070bbfe040fac488c4f7d0eecc'
commits = dict(line.split() for line in Path(sys.argv[1]).read_text().splitlines())
for name, candidate, baseline in (('nnue-generation-reset-gen1-stop1000-stc', commits['gen-1-lambda075-stop1000'], GEN0),
                                  ('nnue-generation-reset-gen1-lambda1-fit-stc', commits['gen-1-lambda1-fit'], LAMBDA1)):
    directory = MATCHES / name
    if directory.exists():
        raise SystemExit(f'{directory} already exists')
    print('==', name, flush=True)
    with (MATCHES / f'{name}.log').open('ab') as log:
        subprocess.run([str(MINASE), 'match', 'run', '--run-dir', str(directory), '--seed', '20261201000000',
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + baseline,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=PREMERGE, check=True, stdout=log, stderr=subprocess.STDOUT)
    print('   done', flush=True)
print('finished', flush=True)
