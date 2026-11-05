"""見直しの比較用のネットを、統合前のコミットの上に別のコミットとして作り、参照で保持する。"""
import os
import subprocess
from pathlib import Path

WORKTREE = '/home/stepney141/board-games/minase/data/worktrees/nnue-generation-reset'
GEN = Path('/home/stepney141/board-games/minase/data/nnue-generation-reset/gen-1')
PARENT = 'cae747ae'
NETWORK = 'crates/minase/nets/nnue.bin'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
index = Path(os.environ['SCRATCH']) / 'review.index'


def git(*args, **kwargs):
    env = {**os.environ, 'GIT_INDEX_FILE': str(index)}
    return subprocess.run(['git', *args], cwd=WORKTREE, env=env, check=True, capture_output=True, text=True, **kwargs).stdout.strip()


parent = git('rev-parse', '--verify', PARENT + '^{commit}')
for name, net, subject, body in (
        ('gen-1-lambda075-stop1000', GEN / 'train-lambda075-stop1000/nnue.bin',
         'feat(eval): embed the lambda 0.75 net stopped near its validation minimum',
         '同じ教師の4分の1の局面を1エポック（952更新）だけλ=0.75で学習した比較用のネット。参照gen-1には使わない。'),
        ('gen-1-lambda1-fit', GEN / 'diag/fit-lambda100/nnue.bin',
         'feat(eval): embed the lambda 1.0 net with its linear path fitted to game outcomes',
         'λ=1.0の比較用のネットにfit-linearの補正を加えたネット。参照gen-1には使わない。')):
    index.unlink(missing_ok=True)
    git('read-tree', parent)
    blob = git('hash-object', '-w', str(net))
    git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
    tree = git('write-tree')
    commit = git('commit-tree', tree, '-p', parent, '-m', subject, '-m', body, '-m', TRAILER)
    git('update-ref', f'refs/nnue-generation-reset/{name}', commit)
    embedded = subprocess.run(['git', 'show', f'{commit}:{NETWORK}'], cwd=WORKTREE, check=True, capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit(f'{name}: embedded net differs')
    print(name, commit, flush=True)
index.unlink(missing_ok=True)
