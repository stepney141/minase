import json
import subprocess
from pathlib import Path

DATA = Path('/home/stepney141/board-games/minase/data/nnue-generation-reset')
WORKTREE = '/home/stepney141/board-games/minase/data/worktrees/nnue-generation-reset-premerge'
a = json.loads((DATA / 'gen-1/train/config.json').read_text())['arguments']
output = DATA / 'gen-1/train-lambda075-stop1000'
log = DATA / 'gen-1/train-lambda075-stop1000.log'
if output.exists():
    raise SystemExit(f'{output} already exists')
subprocess.run(['systemd-run', '--user', '--unit=nnue-gen1-stop1000', '--collect', f'--working-directory={WORKTREE}',
                '-p', 'MemoryMax=20G', '-p', 'MemorySwapMax=0', '-p', 'Nice=19',
                '-p', f'StandardOutput=append:{log}', '-p', f'StandardError=append:{log}',
                str(DATA / 'bin/minase-trainer'), 'nnue', 'train', '--data', *a['data'], '--base', a['base'],
                '--lambda', '0.75', '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']),
                '--threads', str(a['threads']), '--output-dir', str(output), '--feature-set', a['feature_set'],
                '--width', str(a['width']), '--head', a['head'], '--batch', str(a['batch']), '--epochs', '1',
                '--patience', str(a['patience']), '--device', a['device'], '--validation-interval', str(a['validation_interval']),
                '--lr-schedule', 'cosine:12:0.1', '--train-subsample', 'positions:4:0'], check=True)
