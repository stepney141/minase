"""測定1で選んだ線形の経路の学習率のネットについて、補正前を候補、補正後を対照としてSTCで比べる。

使い方: run_fit.py <測定1で選んだ学習率（0.03か0.67903）>
0.03なら、補正前は対照f60eb691を学んだ学習の出力（SHA-256の先頭08b408d9）で、対照はf60eb691である。
0.67903なら、測定1の候補の補正前と補正後のコミットを使う。
補正前のネットは、第2世代のコミットc5910ea5を親とする比較用のコミットに埋め込む。
`H1`は、補正を外したほうが強いことを示す。
"""
import hashlib
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
OLD = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-standard-recipe'
MINASE = str(WORKTREE / 'target/release/minase')
MATCHES = ROOT / 'data/matches'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
STC = 'nnue-standard-recipe-fit-stc'
SEED = '20261300200000'


def git(*args, environment=None):
    return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                          capture_output=True, text=True).stdout.strip()


def resolve(reference):
    return git('rev-parse', '--verify', reference + '^{commit}')


def commit_trained_0_03():
    """0.03の補正前のネットを埋め込んだ比較用のコミットを作る。作成済みならそれを使う。"""
    reference = 'refs/nnue-standard-recipe/lr-linear-0.03-trained'
    net = OLD / 'gen-2/volume/full/train/nnue.bin'
    if not hashlib.sha256(net.read_bytes()).hexdigest().startswith('08b408d9'):
        raise SystemExit(f'{net}のSHA-256が対照の学習の出力と一致しません')
    existing = subprocess.run(['git', 'rev-parse', '--verify', '--quiet', reference], cwd=WORKTREE,
                              capture_output=True, text=True)
    if existing.returncode == 0:
        result = existing.stdout.strip()
    else:
        with tempfile.TemporaryDirectory() as temporary:
            environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}
            git('read-tree', PARENT, environment=environment)
            blob = git('hash-object', '-w', str(net), environment=environment)
            git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}', environment=environment)
            tree = git('write-tree', environment=environment)
            result = git('commit-tree', tree, '-p', PARENT,
                         '-m', 'feat(eval): embed the unfitted generation 2 net trained on 37 teacher files',
                         '-m', '標準レシピv1の測定2の候補。コミットf60eb691のネットの補正前（第2世代の教師37本、線形の経路の学習率0.03）。',
                         '-m', TRAILER, environment=environment)
            git('update-ref', reference, result, environment=environment)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True,
                              capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit(f'{reference}のネットが{net}と一致しません')
    return result


if len(sys.argv) != 2 or sys.argv[1] not in ('0.03', '0.67903'):
    raise SystemExit('使い方: run_fit.py {0.03|0.67903}')
if sys.argv[1] == '0.03':
    candidate, control = commit_trained_0_03(), resolve('f60eb6913d45f4c057031c4964788e1e95ea2442')
else:
    candidate = resolve('refs/nnue-standard-recipe/lr-linear-trained')
    control = resolve('refs/nnue-standard-recipe/lr-linear-fit')
print('candidate', candidate, 'control', control, flush=True)
directory = MATCHES / STC
if directory.exists():
    raise SystemExit(f'{directory} はすでにあります')
running = subprocess.run(['pgrep', '-af', '[m]inase (match run|spsa)|[m]atch_runner|[s]psa_runner'],
                         capture_output=True, text=True).stdout.strip()
if running:
    raise SystemExit(f'進行中の測定があります:\n{running}')
with (MATCHES / f'{STC}.log').open('ab') as log:
    subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', SEED,
                    '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + control,
                    '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                   cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)
print('finished', flush=True)
