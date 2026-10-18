#!/usr/bin/env python3
"""vast.aiの作成、照会、転送、停止と削除を行う。

使い方: scripts/vastをsys.pathへ追加し、import commonで読み込む。
"""

from collections.abc import Iterator
from contextlib import contextmanager
import fcntl
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import time
from typing import Any

POLL_SECONDS = 5
OPERATION_SECONDS = 600
SSH_SECONDS = 300


class VastError(RuntimeError):
    """操作または状態の確認に失敗したことを表す。"""


@contextmanager
def session_lock(path: Path) -> Iterator[None]:
    """同じセッションの起動と取得が重ならないようにする。"""
    with path.open('w') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise VastError('同じセッションの起動または取得が実行中です') from error
        yield



def run_timed(args: list[str], seconds: float, **kwargs: Any) -> subprocess.CompletedProcess[str]:
    """上限でプロセス群を終了し、5秒で終わらなければ強制終了する。"""
    with subprocess.Popen(args, text=True, start_new_session=True, **kwargs) as process:
        try:
            output, errors = process.communicate(timeout=seconds)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
            raise
        result = subprocess.CompletedProcess(args, process.returncode, output, errors)
        result.check_returncode()
        return result


def call(seconds: float, *args: str, **kwargs: Any) -> str:
    """上限時間を設けてvastaiを実行し、標準出力を返す。"""
    if isinstance(seconds, bool) or seconds <= 0:
        raise VastError(f'vastaiの上限秒数には正の整数が必要です: {seconds}')
    return run_timed(['vastai', *map(str, args)], seconds, stdout=subprocess.PIPE, **kwargs).stdout


def ids(label: str) -> list[int]:
    """指定ラベルを持つインスタンスのIDを返す。"""
    return [int(i['id']) for i in json.loads(call(600, 'show', 'instances', '--raw'))
            if i['label'] == label]


def instance(instance_id: int, seconds: float) -> dict[str, Any] | None:
    """指定IDを照会し、削除済みならNoneを返す。"""
    matches = [i for i in json.loads(call(seconds, 'show', 'instances', '--raw'))
               if str(i['id']) == str(instance_id)]
    if len(matches) > 1:
        raise VastError('同じIDのインスタンスが複数あります')
    return matches[0] if matches else None


def create(offer: str, label: str, onstart: Path, image: str, disk: int) -> int:
    """出品からインスタンスを作り、応答のIDを検査する。"""
    value = json.loads(call(600, 'create', 'instance', offer, '--image', image,
                            '--disk', str(disk), '--ssh', '--direct', '--cancel-unavail',
                            '--label', label, '--onstart', str(onstart), '--raw'))['new_contract']
    if type(value) is not int or value <= 0:
        raise VastError('作成応答に正しいnew_contractがありません')
    return value


def change_state(instance_id: int, action: str) -> None:
    """停止または削除を要求し、10分以内に状態を確認する。"""
    word = {'stop': '停止', 'destroy': '削除'}[action]
    end = time.monotonic() + OPERATION_SECONDS
    args = [action, 'instance', str(instance_id)]
    if action == 'destroy':
        args.append('-y')
    try:
        print(call(OPERATION_SECONDS, *args), end='', file=sys.stderr)
    except (OSError, subprocess.SubprocessError) as error:
        raise VastError(f'インスタンス {instance_id}: {word}要求が失敗し、{word}を確認できません') from error
    while (remaining := end - time.monotonic()) > 0:
        try:
            state = instance(instance_id, remaining)
            confirmed = (state is None if action == 'destroy' else
                         state is not None and state['intended_status'] == 'stopped'
                         and state['actual_status'] in ('stopped', 'exited'))
        except (OSError, ValueError, KeyError, TypeError, VastError, subprocess.SubprocessError) as error:
            raise VastError(f'インスタンス {instance_id}: {word}の状態照会に失敗しました') from error
        if confirmed:
            return
        time.sleep(min(POLL_SECONDS, max(0, end - time.monotonic())))
    raise VastError(f'インスタンス {instance_id}: 10分以内に{word}を確認できません')


def stop(instance_id: int) -> None:
    """停止要求と停止状態の確認を行う。"""
    change_state(instance_id, 'stop')


def destroy(instance_id: int) -> None:
    """削除要求と一覧からの消失の確認を行う。"""
    change_state(instance_id, 'destroy')


def wait_running(instance_id: int, seconds: float) -> None:
    """起動を確認し、確認できないインスタンスは削除する。"""
    end = time.monotonic() + seconds
    while (remaining := end - time.monotonic()) > 0:
        try:
            state = instance(instance_id, remaining)
            if state is None:
                break
            status = state['actual_status']
        except (OSError, ValueError, KeyError, TypeError, VastError, subprocess.SubprocessError):
            break
        if status == 'running':
            return
        if status in ('stopped', 'exited', 'offline', 'unknown'):
            break
        time.sleep(min(POLL_SECONDS, max(0, end - time.monotonic())))
    message = f'インスタンス {instance_id}: 起動を確認できないため削除します'
    print(message, file=sys.stderr)
    destroy(instance_id)
    raise VastError(message)


def ssh_address(instance_id: int, seconds: float) -> tuple[str, int]:
    """公開IPと割り当て済みSSHポートを取得する。"""
    end = time.monotonic() + seconds
    while (remaining := end - time.monotonic()) > 0:
        state = instance(instance_id, remaining)
        if state is None:
            raise VastError('インスタンスがありません')
        if (state['public_ipaddr'] and state['ports'] and '22/tcp' in state['ports']
                and state['ports']['22/tcp']):
            host = str(ipaddress.ip_address(state['public_ipaddr']))
            port = int(state['ports']['22/tcp'][0]['HostPort'])
            if not 1 <= port <= 65535:
                raise VastError('SSHポートが不正です')
            return host, port
        time.sleep(min(POLL_SECONDS, max(0, end - time.monotonic())))
    raise VastError(f'インスタンス {instance_id}: SSH接続先の割り当てを確認できません')


def ssh_options(port: int, port_flag: str) -> list[str]:
    """SSHとSCPに共通する接続オプションを作る。"""
    return ['-i', str(Path.home() / '.ssh/id_vastai'), port_flag, str(port),
            '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=accept-new',
            '-o', 'ConnectTimeout=20', '-o', 'ServerAliveInterval=30']


def ssh(host: str, port: int, command: str, *, timeout: float | None = None) -> None:
    """SSHでリモートコマンドを実行する。"""
    args = ['ssh', *ssh_options(port, '-p'), f'root@{host}', command]
    if timeout is None:
        subprocess.run(args, check=True)
    else:
        run_timed(args, timeout)


def wait_ssh(instance_id: int, host: str, port: int) -> None:
    """SSHの準備を5秒おきに確認し、5分で打ち切る。"""
    end = time.monotonic() + SSH_SECONDS
    while (remaining := end - time.monotonic()) > 0:
        try:
            ssh(host, port, 'true', timeout=remaining)
            return
        except (OSError, subprocess.SubprocessError):
            print(f'インスタンス {instance_id}: SSHの準備を確認できないため再試行します', file=sys.stderr)
        time.sleep(min(POLL_SECONDS, max(0, end - time.monotonic())))
    raise VastError(f'インスタンス {instance_id}: 5分以内にSSHの準備を確認できません')


def scp(host: str, port: int, destination: str, *sources: Path | str) -> None:
    """指定したファイルとディレクトリをSSH経由で送る。"""
    subprocess.run(['scp', '-r', *ssh_options(port, '-P'), *map(str, sources),
                    f'root@[{host}]:{destination}'], check=True)


def rsync(instance_id: int, source: str, destination: Path) -> None:
    """接続先を照会し直し、コンテナのSSH経由で取得する。"""
    if not destination.is_absolute():
        raise VastError(f'取得先は絶対パスで指定してください: {destination}')
    host, port = ssh_address(instance_id, 600)
    if ':' in host:
        host = f'[{host}]'
    command = shlex.join(['ssh', *ssh_options(port, '-p')])
    run_timed(['rsync', '-a', '-e', command, f'root@{host}:{source}',
               str(destination) + '/'], 3600)


def copy_complete(output: str) -> bool:
    """rsyncの完了行とエラーの有無を確認する。"""
    complete = re.search(r'^total size is [0-9,]+\s+speedup is [0-9,.]+\s*$', output, re.M)
    failed = re.search(r'rsync (?:error|warning):', output)
    return bool(complete and not failed)


def copy(instance_id: int, source: str, destination: Path) -> None:
    """停止中も使えるvastai copyで取得し、転送結果を検査する。"""
    if not destination.is_absolute():
        raise VastError(f'取得先は絶対パスで指定してください: {destination}')
    try:
        output = call(3600, 'copy', '-i', str(Path.home() / '.ssh/id_vastai'),
                      f'C.{instance_id}:{source}', f'local:{destination}/',
                      stderr=subprocess.STDOUT, env={**os.environ, 'LC_ALL': 'C'})
    except subprocess.CalledProcessError as error:
        print(error.stdout, end='')
        print(f'インスタンス {instance_id}: 取得が失敗しました（終了コード {error.returncode}）', file=sys.stderr)
        raise
    print(output, end='')
    if not copy_complete(output):
        raise VastError(f'インスタンス {instance_id}: rsyncの取得完了を確認できません')


def onstart_prefix(deadline: int) -> str:
    """共通テンプレートに停止期限を埋め込む。"""
    if type(deadline) is not int or deadline <= 0:
        raise VastError('停止期限が不正です')
    return Path(__file__).with_name('onstart.sh').read_text().replace('@DEADLINE@', str(deadline))


def cli_error(error: Exception) -> int:
    """エラーを標準エラー出力へ表示し、終了コードに変換する。"""
    print(error, file=sys.stderr)
    if isinstance(error, subprocess.TimeoutExpired):
        return 124
    if isinstance(error, subprocess.CalledProcessError):
        return error.returncode if error.returncode > 0 else 128 - error.returncode
    return 1
