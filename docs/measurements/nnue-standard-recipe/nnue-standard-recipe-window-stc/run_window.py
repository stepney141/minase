"""中断した系列の第1世代の26本と第2世代の37本で学んだネットを、第2世代の37本で学んだネットとSTCで比べる。

使い方: run_window.py <学習率（0.03か0.67903）> <補正（yes、no、またはpending）> <bucket数> <学習のスレッド数>
学習率と補正は測定1と測定2で選んだものを与える。学習の引数は測定1と同じで、--data、--lr-linear、および--threadsだけが変わる。
補正がpendingなら、学習と補正までを行って止まる。測定2のSTCの間に学習を済ませるためであり、
駒除去差分の診断は5 GBを超えるメモリを使うので、STCの走っていない再実行の時に行う。
測定2の結論が出たらyesかnoで再実行して、コミットとSTCへ進む。
補正の要否によらず補正を行い、Dと補正量を記録する。
bucket数が2以上なら、--bucket-positionsを入力の総局面数Nから N/B + 6√(N/B) で求め、一時ファイルはdata/の下に置く。
補正する場合は、補正の--dataに第2世代の37本だけを与え、対照と補正に使う対局をそろえる。
対照は測定2で採った側のネットである。`H1`は、前の世代の教師を併用すると強いことを示す。
出力はdata/nnue-standard-recipe/recipe/window-lr<学習率>/に置き、学習率ごとに分ける。
完了した工程の出力が残っていれば、その工程を飛ばす。
"""
import json
import math
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
OLD = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-standard-recipe'
MINASE = str(WORKTREE / 'target/release/minase')
TRAINER = str(OLD / 'bin/minase-trainer')
FIT_REPORT = WORKTREE / 'docs/measurements/nnue-outcome-regression/nnue-outcome-regression-fit/fit_report.py'
MATCHES = ROOT / 'data/matches'
CONTROL_TRAIN = OLD / 'gen-2/volume/full/train'
BASE = OLD / 'gen-1/fit/nnue.bin'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
STC = 'nnue-standard-recipe-window-stc'
SEED = '20261300300000'
CONTROLS = {('0.03', 'yes'): 'f60eb6913d45f4c057031c4964788e1e95ea2442',
            ('0.03', 'no'): 'refs/nnue-standard-recipe/lr-linear-0.03-trained',
            ('0.67903', 'yes'): 'refs/nnue-standard-recipe/lr-linear-fit',
            ('0.67903', 'no'): 'refs/nnue-standard-recipe/lr-linear-trained'}


def books(generation):
    directory = OLD / f'gen-{generation}/teacher'
    return [str(path) for path in sorted(directory.glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]


GEN1, GEN2 = books(1), books(2)


def low_priority():
    os.nice(19)


def run(log, command):
    with log.open('a') as stream:
        subprocess.run(command, check=True, stdout=stream, stderr=subprocess.STDOUT, cwd=WORKTREE,
                       preexec_fn=low_priority)


def train(lr, buckets, threads):
    directory = OUTPUT / 'train'
    if (directory / 'summary.json').is_file():
        return
    if (directory / 'latest.npz').is_file():
        raise SystemExit('途中の学習が残っています。/usr/bin/time -vの値が分かれるので、消してからやり直してください')
    a = json.loads((CONTROL_TRAIN / 'config.json').read_text())['arguments']
    command = ['/usr/bin/time', '-v', '-o', str(OUTPUT / 'train-time.txt'),
               TRAINER, 'nnue', 'train', '--data', *GEN1, *GEN2, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', str(threads),
               '--output-dir', str(directory), '--feature-set', a['feature_set'], '--width', str(a['width']),
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', str(a['epochs']),
               '--patience', str(a['patience']), '--device', a['device'], '--lr-schedule', 'cosine:3:0.1',
               '--lr-linear', lr]
    if buckets > 1:
        total = 0
        for path in (*GEN1, *GEN2):
            output = subprocess.run([MINASE, 'data', 'selfplay', 'inspect', path], check=True,
                                    capture_output=True, text=True).stdout
            fields = dict(line.split(':', 1) for line in output.splitlines() if ':' in line)
            total += int(fields['eligible_positions'].strip())
        share = total / buckets
        scratch = OUTPUT / 'scratch'
        scratch.mkdir(parents=True, exist_ok=True)
        command.extend(['--buckets', str(buckets), '--bucket-positions', str(math.ceil(share + 6 * math.sqrt(share))),
                        '--scratch-dir', str(scratch)])
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
                                  '--data', *GEN2, '--tau', '10', '20', '40', '80', '--seed', '20261105',
                                  '--threads', '4', '--output-dir', str(directory)])
    d = json.loads((directory / 'summary.json').read_text())['decision']['d']
    report = OUTPUT / 'fit_report.out'
    if not report.is_file():
        with report.open('w') as stream:
            subprocess.run(['python3', '-I', str(FIT_REPORT), '--summary', str(directory / 'summary.json'),
                            '--data', str(ROOT / 'data')], check=True, stdout=stream, cwd=FIT_REPORT.parent)
    return d


def diagnose(net):
    """比較に使うネットの駒除去差分の符号反転を、第2世代の教師の標本で測る。

    標本は測定1と測定2の診断と同じ（第2世代の37本、--sample 10000 --seed 1）である。
    補正後のネットは組の量子化前の重みを持たないので、その報告では符号反転の欄だけを読む。
    """
    diag = OUTPUT / 'diag'
    diag.mkdir(exist_ok=True)
    if not (diag / 'removal-candidate.json').is_file():
        run(diag / 'removal-candidate.log', [TRAINER, 'diag', 'nnue-removal', '--float', str(OUTPUT / 'train/best.npz'),
                                              '--net', str(net), '--data', *GEN2, '--sample', '10000', '--seed', '1',
                                              '--output', str(diag / 'removal-candidate.json')])


def commit(net, fitted, d):
    name = 'window-fit' if fitted else 'window-trained'
    reference = f'refs/nnue-standard-recipe/{name}'
    existing = subprocess.run(['git', 'rev-parse', '--verify', '--quiet', reference], cwd=WORKTREE,
                              capture_output=True, text=True)
    if existing.returncode == 0:
        result = existing.stdout.strip()
    else:
        detail = f'第2世代の37本だけで補正した。D={d:.6f}。' if fitted else '補正していない。'
        with tempfile.TemporaryDirectory() as temporary:
            environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}

            def git(*args):
                return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                                      capture_output=True, text=True).stdout.strip()

            git('read-tree', PARENT)
            blob = git('hash-object', '-w', str(net))
            git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
            tree = git('write-tree')
            result = git('commit-tree', tree, '-p', PARENT,
                         '-m', 'feat(eval): embed the net trained on generation 1 and 2 teacher files',
                         '-m', f'標準レシピv1の測定3の候補。中断した系列の第1世代の26本と第2世代の37本で学んだ。{detail}',
                         '-m', TRAILER)
            git('update-ref', reference, result)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True,
                              capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit(f'{reference}のネットが{net}と一致しません')
    return result


def stc(candidate, control):
    directory = MATCHES / STC
    if directory.exists():
        raise SystemExit(f'{directory} はすでにあります')
    while subprocess.run(['pgrep', '-f', '[m]inase (match run|spsa)|[m]atch_runner|[s]psa_runner'],
                         capture_output=True).returncode == 0:
        print('進行中の測定の終了を待ちます', flush=True)
        subprocess.run(['sleep', '60'], check=True)
    with (MATCHES / f'{STC}.log').open('ab') as log:
        subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', SEED,
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + control,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)


if len(sys.argv) != 5 or sys.argv[1] not in ('0.03', '0.67903') or sys.argv[2] not in ('yes', 'no', 'pending'):
    raise SystemExit('使い方: run_window.py {0.03|0.67903} {yes|no|pending} <bucket数> <学習のスレッド数>')
lr, correction, bucket_count, thread_count = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
OUTPUT = ROOT / f'data/nnue-standard-recipe/recipe/window-lr{lr}'
if len(GEN1) != 26 or len(GEN2) != 37:
    raise SystemExit(f'教師ファイルの数が26と37ではありません: {len(GEN1)}、{len(GEN2)}')
if bucket_count < 1 or thread_count < 1:
    raise SystemExit('bucket数とスレッド数は1以上にしてください')
print('== 学習', flush=True)
train(lr, bucket_count, thread_count)
print('== 補正', flush=True)
value = fit()
print(f'D={value:.6f}', flush=True)
if correction == 'pending':
    print('pending: 測定2の結論を待ちます', flush=True)
    sys.exit(0)
if correction == 'yes' and (not math.isfinite(value) or value >= 0):
    raise SystemExit(f'補正のDが負の有限値ではありません: {value}。利用者の判断が必要です')
control = subprocess.run(['git', 'rev-parse', '--verify', CONTROLS[(lr, correction)] + '^{commit}'], cwd=WORKTREE,
                         check=True, capture_output=True, text=True).stdout.strip()
net = OUTPUT / ('fit/nnue.bin' if correction == 'yes' else 'train/nnue.bin')
print('== 診断', flush=True)
diagnose(net)
candidate = commit(net, correction == 'yes', value)
print('candidate', candidate, 'control', control, flush=True)
print('== STC', STC, flush=True)
stc(candidate, control)
print('finished', flush=True)
