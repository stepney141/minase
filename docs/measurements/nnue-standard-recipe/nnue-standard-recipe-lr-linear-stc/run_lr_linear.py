"""線形の経路の学習率だけを0.67903に変えて第2世代の教師で学び、補正したネットを0.03のネットとSTCで比べる。

学習の引数は、対照のネット（コミットf60eb691）を学んだときのconfig.jsonから読み、`--lr-linear`だけを変える。
学習の最大常駐メモリを/usr/bin/time -vで測り、63本の学習に`--buckets`が要るかの判断に使う。
補正前と補正後のネットを、第2世代のコミットc5910ea5を親とする比較用のコミットにし、
refs/nnue-standard-recipe/の下の参照で保持する。補正前のコミットは測定2で使う。
駒除去差分の符号反転は、候補と対照の補正後のネットについて同じ標本で測る。
完了した工程の出力が残っていれば、その工程を飛ばす。
"""
import json
import math
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
OLD = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-standard-recipe'
MINASE = str(WORKTREE / 'target/release/minase')
TRAINER = str(OLD / 'bin/minase-trainer')
FIT_REPORT = WORKTREE / 'docs/measurements/nnue-outcome-regression/nnue-outcome-regression-fit/fit_report.py'
MATCHES = ROOT / 'data/matches'
OUTPUT = ROOT / 'data/nnue-standard-recipe/recipe/lr-linear'
CONTROL = OLD / 'gen-2/volume/full'
BASE = OLD / 'gen-1/fit/nnue.bin'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
CONTROL_COMMIT = 'f60eb6913d45f4c057031c4964788e1e95ea2442'
NETWORK = 'crates/minase/nets/nnue.bin'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
LR_LINEAR = '0.67903'
BOOKS = [str(path) for path in sorted((OLD / 'gen-2/teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]
STC = 'nnue-standard-recipe-lr-linear-stc'
SEED = '20261300100000'


def low_priority():
    os.nice(19)


def run(log, command):
    with log.open('a') as stream:
        subprocess.run(command, check=True, stdout=stream, stderr=subprocess.STDOUT, cwd=WORKTREE,
                       preexec_fn=low_priority)


def train():
    directory = OUTPUT / 'train'
    if (directory / 'summary.json').is_file():
        return
    a = json.loads((CONTROL / 'train/config.json').read_text())['arguments']
    if a['data'] != BOOKS or a['lr_linear'] != 0.03:
        raise SystemExit('対照の学習の引数が想定と異なります')
    command = ['/usr/bin/time', '-v', '-o', str(OUTPUT / 'train-time.txt'),
               TRAINER, 'nnue', 'train', '--data', *BOOKS, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', str(a['threads']),
               '--output-dir', str(directory), '--feature-set', a['feature_set'], '--width', str(a['width']),
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', str(a['epochs']),
               '--patience', str(a['patience']), '--device', a['device'], '--lr-schedule', 'cosine:3:0.1',
               '--lr-linear', LR_LINEAR]
    if (directory / 'latest.npz').is_file():
        raise SystemExit('途中の学習が残っています。/usr/bin/time -vの値が分かれるので、消してからやり直してください')
    directory.parent.mkdir(parents=True, exist_ok=True)
    run(OUTPUT / 'train.log', command)


def fit():
    status = json.loads((OUTPUT / 'train/summary.json').read_text())['status']
    if status != 'candidate':
        raise SystemExit(f'学習結果がcandidateではありません: {status}')
    directory = OUTPUT / 'fit'
    if not (directory / 'summary.json').is_file():
        subprocess.run(['rm', '-rf', str(directory)], check=True)
        run(OUTPUT / 'fit.log', [TRAINER, 'nnue', 'fit-linear', '--base', str(OUTPUT / 'train/nnue.bin'),
                                  '--data', *BOOKS, '--tau', '10', '20', '40', '80', '--seed', '20261105',
                                  '--threads', '12', '--output-dir', str(directory)])
    d = json.loads((directory / 'summary.json').read_text())['decision']['d']
    if not math.isfinite(d) or d >= 0:
        raise SystemExit(f'補正のDが負の有限値ではありません: {d}。利用者の判断が必要です')
    return d


def diagnose():
    """補正後のネットは組の量子化前の重みを持たないので、報告では符号反転の欄だけを読む。"""
    diag = OUTPUT / 'diag'
    diag.mkdir(exist_ok=True)
    for name, checkpoint, net in (('removal-candidate', OUTPUT / 'train/best.npz', OUTPUT / 'fit/nnue.bin'),
                                  ('removal-control', CONTROL / 'train/best.npz', CONTROL / 'fit/nnue.bin')):
        if (diag / f'{name}.json').is_file():
            continue
        run(diag / f'{name}.log', [TRAINER, 'diag', 'nnue-removal', '--float', str(checkpoint), '--net', str(net),
                                    '--data', *BOOKS, '--sample', '10000', '--seed', '1',
                                    '--output', str(diag / f'{name}.json')])
    report = OUTPUT / 'fit_report.out'
    if not report.is_file():
        with report.open('w') as stream:
            subprocess.run(['python3', '-I', str(FIT_REPORT), '--summary', str(OUTPUT / 'fit/summary.json'),
                            '--data', str(ROOT / 'data')], check=True, stdout=stream, cwd=FIT_REPORT.parent)


def commit(name, net, subject, body):
    reference = f'refs/nnue-standard-recipe/{name}'
    existing = subprocess.run(['git', 'rev-parse', '--verify', '--quiet', reference], cwd=WORKTREE,
                              capture_output=True, text=True)
    if existing.returncode == 0:
        result = existing.stdout.strip()
    else:
        with tempfile.TemporaryDirectory() as temporary:
            environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}

            def git(*args):
                return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                                      capture_output=True, text=True).stdout.strip()

            git('read-tree', PARENT)
            blob = git('hash-object', '-w', str(net))
            git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
            tree = git('write-tree')
            result = git('commit-tree', tree, '-p', PARENT, '-m', subject, '-m', body, '-m', TRAILER)
            git('update-ref', reference, result)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True,
                              capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit(f'{reference}のネットが{net}と一致しません')
    return result


def stc(candidate):
    directory = MATCHES / STC
    if directory.exists():
        raise SystemExit(f'{directory} はすでにあります')
    running = subprocess.run(['pgrep', '-af', '[m]inase (match run|spsa)|[m]atch_runner|[s]psa_runner'],
                             capture_output=True, text=True).stdout.strip()
    if running:
        raise SystemExit(f'進行中の測定があります:\n{running}')
    with (MATCHES / f'{STC}.log').open('ab') as log:
        subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', SEED,
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + CONTROL_COMMIT,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
print('== 学習', flush=True)
train()
print('== 補正', flush=True)
value = fit()
print('== 診断', flush=True)
diagnose()
trained = commit('lr-linear-trained', OUTPUT / 'train/nnue.bin',
                 'feat(eval): embed the generation 2 net trained with the linear learning rate 0.67903',
                 f'標準レシピv1の測定1の候補の補正前のネット。第2世代の教師37本を線形の経路の学習率{LR_LINEAR}で学んだ。')
fitted = commit('lr-linear-fit', OUTPUT / 'fit/nnue.bin',
                'feat(eval): embed the fitted generation 2 net trained with the linear learning rate 0.67903',
                f'標準レシピv1の測定1の候補。第2世代の教師37本を線形の経路の学習率{LR_LINEAR}で学んで補正したネット。D={value:.6f}。')
print('trained', trained, 'fit', fitted, f'D={value:.6f}', flush=True)
print('== STC', STC, flush=True)
stc(fitted)
print('finished', flush=True)
