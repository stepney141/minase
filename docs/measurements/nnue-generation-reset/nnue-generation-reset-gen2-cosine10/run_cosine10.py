"""第2世代のSTCの終了を待ち、学習率の予定だけを変えて学び直したネットを第2世代と対局で比べる。

同じ教師、初期値、およびシードで`cosine:10:0.1`により学習し、`fit-linear`で補正して別のコミットにする。
予定の総長の効果を測るので、早期終了で止めず10エポックまで学習する（`--epochs 10 --patience 50`）。
検証損失の最良は学習率がまだ高いエポック2だったので、下げきったエポック10のネットを比べる。
学習器は最良のネットだけを書き出すので、チェックポイントの複製の「最良」をエポック10へ書き換えて再開し、通常の経路で書き出す。
学習と補正の出力が残っていれば、その工程から再開する。参照`gen-2`は動かさない。
"""
import json
import math
import os
import subprocess
import re
import shutil
import tempfile
import time
import zipfile
from pathlib import Path

ROOT = Path('/home/stepney141/board-games/minase')
DATA = ROOT / 'data/nnue-generation-reset'
WORKTREE = ROOT / 'data/worktrees/nnue-generation-reset'
MINASE = str(DATA / 'bin/minase')
TRAINER = str(DATA / 'bin/minase-trainer')
MATCHES = ROOT / 'data/matches'
GEN = DATA / 'gen-2'
TRAIN = GEN / 'train-cosine10'
LAST = GEN / 'train-cosine10-last'
FIT = GEN / 'fit-cosine10-last'
DIAG = GEN / 'diag'
BASE = DATA / 'gen-1/fit/nnue.bin'
PARENT = 'c5910ea55fc0bf928690a730a264c48573aba04f'
NETWORK = 'crates/minase/nets/nnue.bin'
REFERENCE = 'refs/nnue-generation-reset/gen-2-cosine10'
STC = 'nnue-generation-reset-gen2-cosine10-stc'
TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
BOOKS = [str(path) for path in sorted((GEN / 'teacher').glob('book-*.mnsd'), key=lambda path: int(path.stem[5:]))]


def low_priority():
    os.nice(19)


def wait_for_generation_stc():
    unit = 'nnue-gen2-diag-stc'
    while subprocess.run(['systemctl', '--user', 'is-active', '--quiet', unit]).returncode == 0:
        time.sleep(60)
    lines = (GEN / 'diag-stc.log').read_text().splitlines()
    if not lines or lines[-1] != 'finished':
        raise SystemExit('第2世代の診断とSTCが正常に終わっていません')


def train():
    if (TRAIN / 'summary.json').is_file():
        return
    a = json.loads((GEN / 'train/config.json').read_text())['arguments']
    command = [TRAINER, 'nnue', 'train', '--data', *BOOKS, '--base', str(BASE), '--lambda', str(a['lambda']),
               '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', '8',
               '--output-dir', str(TRAIN), '--feature-set', a['feature_set'], '--width', str(a['width']),
               '--head', a['head'], '--batch', str(a['batch']), '--epochs', '10',
               '--patience', '50', '--device', a['device'],
               '--validation-interval', str(a['validation_interval']), '--lr-schedule', 'cosine:10:0.1']
    if (TRAIN / 'latest.npz').is_file():
        command.extend(['--resume', str(TRAIN)])
    with (GEN / 'train-cosine10.log').open('a') as log:
        subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


def training_command(output):
    a = json.loads((GEN / 'train/config.json').read_text())['arguments']
    return [TRAINER, 'nnue', 'train', '--data', *BOOKS, '--base', str(BASE), '--lambda', str(a['lambda']),
            '--teacher-cp-per-logit', 'calibrate', '--seed', str(a['seed']), '--threads', '8',
            '--output-dir', str(output), '--feature-set', a['feature_set'], '--width', str(a['width']),
            '--head', a['head'], '--batch', str(a['batch']), '--epochs', '10',
            '--patience', '50', '--device', a['device'],
            '--validation-interval', str(a['validation_interval']), '--lr-schedule', 'cosine:10:0.1']


def patched_checkpoint(source, destination, epoch, loss):
    """進捗の「最良」だけを書き換え、重みと最適化器の状態はそのまま保つ。"""
    with zipfile.ZipFile(source) as reader, zipfile.ZipFile(destination, 'w', zipfile.ZIP_STORED) as writer:
        for info in reader.infolist():
            data = reader.read(info.filename)
            if info.filename == 'meta.npy':
                size = int.from_bytes(data[8:10], 'little')
                header, body = data[10:10 + size], data[10 + size:]
                new_body, count = re.subn(rb'"best":\{"epoch":\d+,"validation_loss":[0-9.e-]+\}',
                                          f'"best":{{"epoch":{epoch},"validation_loss":{loss!r}}}'.encode(), body)
                if count != 1:
                    raise SystemExit('チェックポイントの最良の欄が見つかりません')
                old_shape = f'({len(body)},)'.encode()
                new_shape = f'({len(new_body)},)'.encode()
                if old_shape not in header or len(old_shape) != len(new_shape):
                    raise SystemExit('チェックポイントの配列の形を書き換えられません')
                data = data[:10] + header.replace(old_shape, new_shape) + new_body
            writer.writestr(info.filename, data)


def export_last():
    if (LAST / 'summary.json').is_file():
        return
    if LAST.exists():
        shutil.rmtree(LAST)
    LAST.mkdir()
    for name in ('config.json', 'training.jsonl', 'validation-interval.jsonl'):
        shutil.copyfile(TRAIN / name, LAST / name)
    rows = [json.loads(line) for line in (TRAIN / 'training.jsonl').read_text().splitlines()]
    last = rows[-1]
    if last['epoch'] != 10:
        raise SystemExit(f'最後のエポックが10ではありません: {last["epoch"]}')
    patched_checkpoint(TRAIN / 'latest.npz', LAST / 'latest.npz', last['epoch'], last['validation_loss'])
    shutil.copyfile(LAST / 'latest.npz', LAST / 'best.npz')
    with (GEN / 'train-cosine10-last.log').open('a') as log:
        subprocess.run([*training_command(LAST), '--resume', str(LAST)], check=True, stdout=log,
                       stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)
    summary = json.loads((LAST / 'summary.json').read_text())
    if summary['best_epoch'] != 10 or summary['total_steps'] != json.loads((TRAIN / 'summary.json').read_text())['total_steps']:
        raise SystemExit('書き出したネットがエポック10のものではありません')


def fit():
    status = json.loads((LAST / 'summary.json').read_text())['status']
    if status != 'candidate':
        raise SystemExit(f'学習結果がcandidateではありません: {status}')
    if not (FIT / 'summary.json').is_file():
        subprocess.run(['rm', '-rf', str(FIT)], check=True)
        with (GEN / 'fit-cosine10-last.log').open('a') as log:
            subprocess.run([TRAINER, 'nnue', 'fit-linear', '--base', str(LAST / 'nnue.bin'), '--data', *BOOKS,
                            '--tau', '10', '20', '40', '80', '--seed', '20261105', '--threads', '12',
                            '--output-dir', str(FIT)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)
    d = json.loads((FIT / 'summary.json').read_text())['decision']['d']
    if not math.isfinite(d) or d >= 0:
        raise SystemExit(f'補正のDが負の有限値ではありません: {d}')
    return d


def diagnose():
    # 補正後のネットは組の量子化前の重みを持たないので、その報告では符号反転の欄だけを読む。
    for name, net in (('removal-cosine10-trained', LAST / 'nnue.bin'), ('removal-cosine10', FIT / 'nnue.bin')):
        with (DIAG / f'{name}.log').open('w') as log:
            subprocess.run([TRAINER, 'diag', 'nnue-removal', '--float', str(LAST / 'best.npz'), '--net', str(net),
                            '--data', *BOOKS, '--sample', '10000', '--seed', '1', '--output', str(DIAG / f'{name}.json')],
                           check=True, stdout=log, stderr=subprocess.STDOUT, cwd=WORKTREE, preexec_fn=low_priority)


def commit(d):
    net = FIT / 'nnue.bin'
    with tempfile.TemporaryDirectory() as temporary:
        environment = {**os.environ, 'GIT_INDEX_FILE': str(Path(temporary) / 'index')}

        def git(*args):
            return subprocess.run(['git', *args], cwd=WORKTREE, env=environment, check=True,
                                  capture_output=True, text=True).stdout.strip()

        git('read-tree', PARENT)
        blob = git('hash-object', '-w', str(net))
        git('update-index', '--cacheinfo', f'100644,{blob},{NETWORK}')
        tree = git('write-tree')
        result = git('commit-tree', tree, '-p', PARENT, '-m', 'feat(eval): embed the generation 2 net trained on a longer learning rate schedule',
                     '-m', f'第2世代と同じ教師を`cosine:10:0.1`で学習し、エポック10のネットを補正した比較用のネット。D={d:.6f}。参照gen-2には使わない。',
                     '-m', TRAILER)
        git('update-ref', REFERENCE, result)
    embedded = subprocess.run(['git', 'show', f'{result}:{NETWORK}'], cwd=WORKTREE, check=True, capture_output=True).stdout
    if embedded != net.read_bytes():
        raise SystemExit('コミットのネットが補正後のネットと一致しません')
    return result


def stc(candidate):
    directory = MATCHES / STC
    if directory.exists():
        raise SystemExit(f'{directory} はすでにあります')
    with (MATCHES / f'{STC}.log').open('ab') as log:
        subprocess.run([MINASE, 'match', 'run', '--run-dir', str(directory), '--seed', '20261202500000',
                        '--candidate', 'commit:' + candidate, '--baseline', 'commit:' + PARENT,
                        '--each', 'time=10000+100', '--concurrency', '16', 'gsprt', '--max-pairs', '3000'],
                       cwd=WORKTREE, check=True, stdout=log, stderr=subprocess.STDOUT)


if len(BOOKS) != 37:
    raise SystemExit(f'教師ファイルの数が37ではありません: {len(BOOKS)}')
print('== 第2世代のSTCを待ちます', flush=True)
wait_for_generation_stc()
print('== 学習', flush=True)
train()
print('== エポック10のネットの書き出し', flush=True)
export_last()
print('== 補正', flush=True)
value = fit()
print('== 診断', flush=True)
diagnose()
print('== コミット', flush=True)
candidate_commit = commit(value)
print(candidate_commit, flush=True)
print('== STC', flush=True)
stc(candidate_commit)
print('finished', flush=True)
