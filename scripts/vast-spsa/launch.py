#!/usr/bin/env python3
"""設定と出品を検査し、必要なファイルをすべて転送してSPSAを開始する。

使い方: python3 scripts/vast-spsa/launch.py <セッション名> <出品ID> <期限までの分数>
"""

import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / 'vast'))
import common
import remote


def check_offer(offers: list[dict[str, Any]], offer_id: str) -> None:
    """指定出品のCPU、貸出期限、通信とディスクの条件を検査する。"""
    matches = [offer for offer in offers if str(offer['id']) == offer_id]
    if not matches:
        raise ValueError('選定条件を満たす出品の中に、指定した出品が見つかりません')
    if len(matches) != 1:
        raise ValueError('指定した出品のIDが照会結果に重複しています')
    offer = matches[0]
    errors = []
    for key, expected in (('cpu_arch', 'amd64'), ('verification', 'verified')):
        if key not in offer or offer[key] != expected:
            errors.append(f'{key}は{expected}でなければなりません')
    for key, minimum in (('cpu_cores', 192), ('duration', 172800), ('reliability2', 0.95),
                         ('direct_port_count', 1), ('inet_up', 100), ('disk_space', 50)):
        if (key not in offer or type(offer[key]) not in (int, float)
                or not math.isfinite(offer[key]) or offer[key] < minimum):
            errors.append(f'{key}は{minimum}以上でなければなりません')
    if 'gpu_frac' not in offer or offer['gpu_frac'] != 1:
        errors.append('gpu_fracは1でなければなりません')
    if ('cpu_cores_effective' not in offer or 'cpu_cores' not in offer
            or offer['cpu_cores_effective'] != offer['cpu_cores']):
        errors.append('cpu_cores_effectiveとcpu_coresが一致しません')
    if errors:
        raise ValueError('\n'.join(errors))


def write_session(path: Path, config: remote.Session) -> None:
    """検査済みの平らな設定をTOMLで保存する。"""
    lines = []
    for key, value in config.items():
        if type(value) is str:
            # JSONとTOMLで共通する文字列エスケープを使う。
            encoded = json.dumps(value, ensure_ascii=False).replace('\x7f', '\\u007f')
        elif type(value) is bool:
            encoded = 'true' if value else 'false'
        elif type(value) is int:
            encoded = str(value)
        elif type(value) is float and math.isfinite(value):
            encoded = repr(value)
        else:
            raise ValueError(f'TOMLに書き出せない値です: {key}')
        lines.append(f'{key} = {encoded}\n')
    path.write_text(''.join(lines))


def onstart(deadline: int, session: str) -> str:
    """共通の自己停止処理へSPSAの起動行を加え、長さを検査する。"""
    remote.check_name(session)
    script = common.onstart_prefix(deadline) + (
        "nohup setsid bash -c 'set -euo pipefail; until [[ -f /workspace/job/started ]]; "
        f"do sleep 2; done; exec python3 /workspace/job/remote.py {session}' "
        '>> /workspace/job/logs/remote.log 2>&1 < /dev/null &\n')
    if len(script) > 4048:
        raise ValueError(f'起動時スクリプトが4048文字を超えています: {len(script)}')
    return script


def git_output(root: Path, *args: str) -> str:
    """gitの標準出力を読み、コマンドの失敗を伝える。"""
    return subprocess.run(['git', '-C', str(root), *args], check=True,
                          text=True, stdout=subprocess.PIPE).stdout.strip()


def launch(session: str, offer: str, minutes: str) -> None:
    """作成から転送完了までを管理し、途中の失敗では調整機を削除する。"""
    remote.check_name(session)
    if not re.fullmatch(r'[1-9][0-9]*', offer) or not re.fullmatch(r'[1-9][0-9]*', minutes):
        raise ValueError('出品IDと期限までの分数には正の整数が必要です')
    root = Path(git_output(SCRIPT_DIR.parent.parent, 'rev-parse', '--show-toplevel'))
    directory = root / 'data/vast-spsa' / session
    label = 'minase-spsa-' + session
    config = remote.load_session(directory)
    for key in ('commit', 'runner_commit'):
        config[key] = git_output(root, 'rev-parse', '--verify', '--end-of-options', config[key] + '^{commit}')
    if (directory / 'run').is_dir():
        remote.check_manifest(directory / 'run/manifest.json', config)
    for command in ('vastai', 'ssh', 'scp', 'rustc'):
        if shutil.which(command) is None:
            raise ValueError(f'必要なコマンドがありません: {command}')
    key = Path.home() / '.ssh/id_vastai'
    if not os.access(key, os.R_OK):
        raise ValueError(f'{key}がありません')
    with common.session_lock(directory / '.operation.lock'), tempfile.TemporaryDirectory(prefix='.launch.', dir=directory) as name:
        temporary = Path(name)
        instance_id = None
        cleanup = False
        try:
            offers = common.call(600, 'search', 'offers',
                                 'cpu_cores>=192 verified=true reliability>=0.95 inet_up>=100 disk_space>=50 rentable=true gpu_frac=1 duration>=2 direct_port_count>=1',
                                 '--type', 'on-demand', '--raw', '--limit', '1000')
            check_offer(json.loads(offers), offer)
            ids = common.ids(label)
            if ids:
                raise ValueError('同じラベルのインスタンスがあります: ' + '\n'.join(map(str, ids)))
            deadline = int(time.time()) + int(minutes) * 60
            if deadline >= 2**63:
                raise ValueError('停止期限が大きすぎます')
            (directory / 'deadline').write_text(f'{deadline}\n')
            script = onstart(deadline, session)
            (temporary / 'onstart.sh').write_text(script)
            version_output = subprocess.run(['rustc', '--version'], check=True, text=True, stdout=subprocess.PIPE).stdout.split()
            if len(version_output) < 2:
                raise ValueError(f'公式イメージを指定できないrustcの版です: {" ".join(version_output)}')
            version = version_output[1]
            if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', version):
                raise ValueError(f'公式イメージを指定できないrustcの版です: {version}')
            (directory / 'offer.json').write_text(offers)
            (directory / 'onstart.sh').write_text(script)
            try:
                instance_id = common.create(offer, label, temporary / 'onstart.sh', f'rust:{version}-bookworm', 50)
            except (OSError, ValueError, KeyError, TypeError, common.VastError, subprocess.SubprocessError):
                try:
                    ids = common.ids(label)
                except (OSError, ValueError, KeyError, TypeError, common.VastError, subprocess.SubprocessError) as error:
                    print(error, file=sys.stderr)
                    ids = []
                if len(ids) == 1:
                    instance_id = ids[0]
                    cleanup = True
                    print(f'インスタンス {instance_id}: 作成応答を確認できないため削除します', file=sys.stderr)
                else:
                    print(f'ラベル {label}: 作成操作は未確認です。照会できたID: ' + '\n'.join(map(str, ids)), file=sys.stderr)
                raise
            cleanup = True
            (directory / 'instance-id').write_text(f'{instance_id}\n')
            # 起動待ちの関数が失敗時の削除を担当する。
            cleanup = False
            common.wait_running(instance_id, 600)
            cleanup = True
            host, port = common.ssh_address(instance_id, 600)
            (directory / 'ssh-address').write_text(f'{host} {port}\n')
            bundle_git = temporary / 'bundle.git'
            subprocess.run(['git', 'init', '--bare', str(bundle_git)], check=True, stdout=subprocess.DEVNULL)
            subprocess.run(['git', f'--git-dir={bundle_git}', 'fetch', '--no-tags', str(root),
                            config['commit'] + ':refs/heads/session', config['runner_commit'] + ':refs/heads/runner'], check=True)
            subprocess.run(['git', f'--git-dir={bundle_git}', 'bundle', 'create', str(temporary / 'source.bundle'),
                            'refs/heads/session', 'refs/heads/runner'], check=True)
            write_session(temporary / 'session.toml', config)
            common.wait_ssh(instance_id, host, port)
            common.ssh(host, port, f'mkdir -p /workspace/job/logs /workspace/job/{session}')
            if config['precheck']:
                common.ssh(host, port, f'mkdir -p /workspace/job/{session}-safety')
            common.scp(host, port, '/workspace/job/', temporary / 'source.bundle', temporary / 'session.toml',
                       directory / 'params', SCRIPT_DIR / 'remote.py')
            if (directory / 'safety.params').is_file():
                common.scp(host, port, '/workspace/job/', directory / 'safety.params')
            if (directory / 'run').is_dir():
                common.scp(host, port, f'/workspace/job/{session}/', str(directory / 'run') + '/.')
            common.ssh(host, port, 'touch /workspace/job/started')
            cleanup = False
        finally:
            if cleanup:
                common.destroy(instance_id)
    print(f'セッション {session} を開始しました。インスタンスID: {instance_id}')


def main() -> int:
    """起動引数と実行時エラーを処理する。"""
    if len(sys.argv) != 4:
        print('使い方: launch.py <セッション名> <出品ID> <期限までの分数>', file=sys.stderr)
        return 1
    try:
        launch(*sys.argv[1:])
    except (OSError, ValueError, KeyError, TypeError, common.VastError, subprocess.SubprocessError) as error:
        return common.cli_error(error)
    return 0


if __name__ == '__main__':
    sys.exit(main())
