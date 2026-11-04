#!/usr/bin/env python3
"""生成機で世代ごとの教師を生成し、完了した本だけを公開する。

使い方: python3 /workspace/job/generation_worker.py {run,cpu-info,stop}
generation_chain.pyが本ファイルとworker.jsonを/workspace/job/へ配置する。
job.jsonを作業の入力とし、teacher/に本、logs/にログ、work/に生成中の出力を置く。
Python 3.11の標準ライブラリだけで動作し、作業機のcommon.pyは必要としない。
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any
import urllib.error
import urllib.request


def cpu_info(cgroup: Path) -> dict[str, Any]:
    """cpusetで許されたスレッド数と、cgroupのCPU時間上限を返す。"""
    v2 = cgroup / 'cpu.max'
    v1 = cgroup / 'cpu/cpu.cfs_quota_us'
    if v2.is_file():
        quota, period = v2.read_text().split()
        if int(period) <= 0:
            raise ValueError('cgroupのCPU周期が正数ではありません')
        limit = None if quota == 'max' else int(quota) / int(period)
    elif v1.is_file():
        quota = int(v1.read_text())
        period = int((cgroup / 'cpu/cpu.cfs_period_us').read_text())
        if period <= 0:
            raise ValueError('cgroupのCPU周期が正数ではありません')
        limit = None if quota == -1 else quota / period
    else:
        raise ValueError('cgroupのCPU上限を取得できません')
    if limit is not None and limit <= 0:
        raise ValueError('cgroupのCPU上限が正数ではありません')
    return {'nproc': len(os.sched_getaffinity(0)), 'cpu_limit': limit}


def stop_instance(environ: Path) -> int:
    """コンテナ専用の鍵で停止を要求し、通信失敗と5xxだけを30分まで再試行する。"""
    values = dict(item.split(b'=', 1) for item in environ.read_bytes().split(b'\0') if b'=' in item)
    instance = values[b'CONTAINER_ID'].decode('ascii')
    if not instance.isdecimal():
        raise ValueError('CONTAINER_IDが整数ではありません')
    key = values[b'CONTAINER_API_KEY'].decode('ascii')
    request = urllib.request.Request(
        f'https://console.vast.ai/api/v0/instances/{instance}/',
        data=b'{"state":"stopped"}', method='PUT',
        headers={'Authorization': 'Bearer ' + key, 'Content-Type': 'application/json'})
    end = time.monotonic() + 1800
    while (remaining := end - time.monotonic()) > 0:
        try:
            with urllib.request.urlopen(request, timeout=min(60, remaining)) as response:
                code = response.status
        except urllib.error.HTTPError as error:
            code = error.code
            error.close()
        except (OSError, urllib.error.URLError):
            # 例外の本文には認証情報が含まれ得るので出力しない。
            code = None
        print(f'生成機 {instance}: 停止要求のHTTP応答={code}', flush=True)
        if code is not None:
            if 200 <= code < 300:
                return 0
            if not 500 <= code < 600:
                return 1
        remaining = end - time.monotonic()
        if remaining > 0:
            time.sleep(min(60, remaining))
    print(f'生成機 {instance}: 30分以内に停止を要求できませんでした', file=sys.stderr)
    return 1


def next_book(job: dict[str, Any], root: Path) -> int | None:
    """検査済みでも生成済みでもない、目標範囲内の最小の番号を返す。"""
    skipped = set(job['skip_books'])
    directory = root / 'teacher' / f'gen-{job["generation"]}'
    for number in range(job['target_books']):
        if number not in skipped and not (directory / f'book-{number}.mnsd').is_file():
            return number
    return None


def prepare_commit(root: Path, job: dict[str, Any], log: Any) -> Path:
    """bundleから指定コミットを展開し、起動後の最初の本の前にビルドする。"""
    repository = root / 'minase'
    if not repository.exists():
        subprocess.run(['git', 'init', str(repository)], check=True, stdout=log, stderr=subprocess.STDOUT)
    head = subprocess.run(['git', 'rev-parse', '--verify', 'HEAD'], cwd=repository,
                          text=True, stdout=subprocess.PIPE, stderr=log)
    if head.returncode not in (0, 1, 128):
        head.check_returncode()
    if head.returncode or head.stdout.strip() != job['commit']:
        reference = f'refs/nnue-generation-reset/gen-{job["generation"] - 1}'
        subprocess.run(['git', 'fetch', job['bundle'], reference], cwd=repository,
                       check=True, stdout=log, stderr=subprocess.STDOUT)
        subprocess.run(['git', 'checkout', '--detach', job['commit']], cwd=repository,
                       check=True, stdout=log, stderr=subprocess.STDOUT)
    actual = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=repository, check=True,
                            text=True, stdout=subprocess.PIPE, stderr=log).stdout.strip()
    if actual != job['commit']:
        raise ValueError(f'生成コミットが一致しません: {actual}')
    # HEADの更新後にビルドが中断していても、古いバイナリを再利用しない。
    subprocess.run(['cargo', 'build', '--release', '--locked', '--bin', 'minase'],
                   cwd=repository, check=True, stdout=log, stderr=subprocess.STDOUT)
    return repository


def generate_book(root: Path, job: dict[str, Any], number: int, prepared: str | None) -> str:
    """本を生成し、ログ、本の順に同じファイルシステム内で公開する。"""
    generation = job['generation']
    logs = root / 'logs' / f'gen-{generation}'
    teacher = root / 'teacher' / f'gen-{generation}'
    logs.mkdir(parents=True, exist_ok=True)
    teacher.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=root / 'work') as temporary:
        directory = Path(temporary)
        output = directory / f'book-{number}.mnsd'
        log_path = directory / f'book-{number}.log'
        try:
            with log_path.open('w') as log:
                repository = root / 'minase'
                if prepared != job['commit']:
                    repository = prepare_commit(root, job, log)
                command = [str(repository / 'target/release/minase'), 'data', 'selfplay', 'generate',
                           '--output', str(output), '--seed', str(job['generation_seed'] + job['book_step'] * number)]
                for key in ('games', 'nodes', 'random_moves', 'concurrency', 'max_ply', 'hash_mb'):
                    command.extend(['--' + key.replace('_', '-'), str(job[key])])
                subprocess.run(command, cwd=repository, check=True, stdout=log, stderr=subprocess.STDOUT)
            if not output.is_file():
                raise ValueError('生成が成功しましたが、本がありません')
            log_path.replace(logs / log_path.name)
            output.replace(teacher / output.name)
        except Exception as error:
            # 本の公開に失敗した場合には、先に移したログを失敗ログへ移し直す。
            source = log_path if log_path.exists() else logs / log_path.name
            with source.open('a') as log:
                log.write(f'\n生成失敗: {error}\n')
            source.replace(logs / f'book-{number}.failed.log')
            raise
    return job['commit']


def write_exit_status(root: Path, code: int) -> None:
    """終了コードは完成した1行だけを公開する。"""
    temporary = root / 'logs/.exit-status.tmp'
    temporary.write_text(f'{code}\n')
    temporary.replace(root / 'logs/exit-status')


def run_worker(root: Path) -> int:
    """未完成の出力を捨て、指示された本を生成し、仕事が途絶えたら自己停止する。"""
    (root / 'logs').mkdir(parents=True, exist_ok=True)
    try:
        work = root / 'work'
        if work.exists():
            shutil.rmtree(work)
        work.mkdir()
        (root / 'logs/exit-status').unlink(missing_ok=True)
        config = json.loads((root / 'worker.json').read_text())
        idle_limit = config['idle_limit_seconds']
        if idle_limit <= 0:
            raise ValueError('仕事を待つ上限秒数は正数にしてください')
        idle_since = time.monotonic()
        prepared = None
        while True:
            path = root / 'job.json'
            if path.is_file():
                job = json.loads(path.read_text())
                number = next_book(job, root)
                if number is not None:
                    prepared = generate_book(root, job, number, prepared)
                    idle_since = time.monotonic()
                    continue
            remaining = idle_limit - (time.monotonic() - idle_since)
            if remaining <= 0:
                print('次の本が指定されないまま待ち時間の上限に達したため、自己停止します', flush=True)
                code = stop_instance(Path('/proc/1/environ'))
                if code:
                    write_exit_status(root, code)
                return code
            time.sleep(min(2, remaining))
    except Exception as error:
        print(f'生成作業を停止します: {error}', file=sys.stderr)
        code = 1
        if isinstance(error, subprocess.CalledProcessError):
            code = error.returncode if error.returncode > 0 else 128 - error.returncode
        write_exit_status(root, code)
        return code


def main(argv: list[str]) -> int:
    """固定の配置で作業、CPU照会、または停止を実行する。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('run', 'cpu-info', 'stop'))
    args = parser.parse_args(argv)
    try:
        if args.command == 'cpu-info':
            print(json.dumps(cpu_info(Path('/sys/fs/cgroup'))))
            return 0
        if args.command == 'stop':
            return stop_instance(Path('/proc/1/environ'))
        return run_worker(Path('/workspace/job'))
    except Exception as error:
        print(f'生成機の操作に失敗しました: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
