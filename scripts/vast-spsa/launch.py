#!/usr/bin/env python3
"""設定と出品を検査し、必要なファイルをすべて転送してSPSAを開始する。

使い方:
  python3 scripts/vast-spsa/launch.py <セッション名> <期限までの分数> --offer <出品ID>
  python3 scripts/vast-spsa/launch.py <セッション名> <期限までの分数> --max-price <1時間あたりの米ドル>
      --min-cpu-cores <スレッド数> --min-cpu-ram <MB> --cpu <CPU名の一部>
      [--cpu <CPU名の一部> ...] --wait <分>
"""

import argparse
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

OFFER_RETRY_SECONDS = 30
OFFER_QUERY = ('cpu_cores>=192 verified=true reliability>=0.95 inet_up>=100 disk_space>=50 '
               'rentable=true gpu_frac=1 duration>=2 direct_port_count>=1')
FAILURES = (OSError, ValueError, KeyError, TypeError, common.VastError, subprocess.SubprocessError)


def offer_errors(offer: dict[str, Any]) -> list[str]:
    """出品のCPU、貸出期限、通信とディスクの条件ごとに誤りを返す。"""
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
    return errors


def check_offer(offers: list[dict[str, Any]], offer_id: str) -> None:
    """指定出品を1件に定め、出品条件を検査する。"""
    matches = [offer for offer in offers if str(offer['id']) == str(offer_id)]
    if not matches:
        raise ValueError('選定条件を満たす出品の中に、指定した出品が見つかりません')
    if len(matches) != 1:
        raise ValueError('指定した出品のIDが照会結果に重複しています')
    errors = offer_errors(matches[0])
    if errors:
        raise ValueError('\n'.join(errors))


def search_offers() -> tuple[str, list[dict[str, Any]]]:
    """起動条件に合う出品を照会し、生の応答と解析結果を返す。"""
    output = common.call(600, 'search', 'offers', OFFER_QUERY,
                         '--type', 'on-demand', '--raw', '--limit', '1000')
    offers = json.loads(output)
    if type(offers) is not list or any(type(offer) is not dict for offer in offers):
        raise ValueError('出品の照会結果はオブジェクトの配列でなければなりません')
    return output, offers


def select_offers(offers: list[dict[str, Any]], max_price: float,
                  min_cpu_cores: int, min_cpu_ram: int,
                  cpu_names: list[str]) -> list[dict[str, Any]]:
    """追加条件に合う出品を価格とIDの昇順で返す。"""
    selected = []
    for offer in offers:
        offer_id = offer.get('id')
        cpu_name = offer.get('cpu_name')
        cpu_ram = offer.get('cpu_ram')
        price = offer.get('dph_total')
        if (offer_errors(offer)
                or type(offer_id) is not int or offer_id <= 0
                or type(cpu_name) is not str or not any(name in cpu_name for name in cpu_names)
                or offer['cpu_cores'] < min_cpu_cores
                or type(cpu_ram) not in (int, float) or not math.isfinite(cpu_ram)
                or cpu_ram < min_cpu_ram
                or type(price) not in (int, float) or not math.isfinite(price)
                or price > max_price):
            continue
        selected.append(offer)
    return sorted(selected, key=lambda offer: (offer['dph_total'], offer['id']))


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


def start_offer(session: str, minutes: int, offer_id: int, offers_output: str,
                root: Path, directory: Path, label: str, config: remote.Session,
                temporary: Path, retry_unavailable: bool, wait_end: float | None) -> int | None:
    """1件の出品で作成と転送を行い、借りられない出品だけ再試行可能とする。"""
    instance_id = None
    cleanup = False
    try:
        deadline = int(time.time()) + minutes * 60
        if deadline >= 2**63:
            raise ValueError('停止期限が大きすぎます')
        (directory / 'deadline').write_text(f'{deadline}\n')
        script = onstart(deadline, session)
        (temporary / 'onstart.sh').write_text(script)
        version_output = subprocess.run(['rustc', '--version'], check=True, text=True,
                                        stdout=subprocess.PIPE).stdout.split()
        if len(version_output) < 2:
            raise ValueError(f'公式イメージを指定できないrustcの版です: {" ".join(version_output)}')
        version = version_output[1]
        if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', version):
            raise ValueError(f'公式イメージを指定できないrustcの版です: {version}')
        (directory / 'offer.json').write_text(offers_output)
        (directory / 'onstart.sh').write_text(script)
        if wait_end is not None and time.monotonic() >= wait_end:
            raise ValueError('出品の作成待機時間を過ぎました')
        try:
            instance_id = common.create(str(offer_id), label, temporary / 'onstart.sh',
                                        f'rust:{version}-bookworm', 50)
        except FAILURES:
            try:
                ids = common.ids(label)
            except FAILURES as error:
                print(error, file=sys.stderr)
                ids = None
            if ids == [] and retry_unavailable:
                print(f'出品 {offer_id}: 借りられなかったため次の候補を試します', file=sys.stderr)
                return None
            if ids is not None and len(ids) == 1:
                instance_id = ids[0]
                cleanup = True
                print(f'インスタンス {instance_id}: 作成応答を確認できないため削除します', file=sys.stderr)
            else:
                confirmed = [] if ids is None else ids
                print(f'ラベル {label}: 作成操作は未確認です。照会できたID: '
                      + '\n'.join(map(str, confirmed)), file=sys.stderr)
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
        subprocess.run(['git', 'init', '--bare', str(bundle_git)], check=True,
                       stdout=subprocess.DEVNULL)
        subprocess.run(['git', f'--git-dir={bundle_git}', 'fetch', '--no-tags', str(root),
                        config['commit'] + ':refs/heads/session',
                        config['runner_commit'] + ':refs/heads/runner'], check=True)
        subprocess.run(['git', f'--git-dir={bundle_git}', 'bundle', 'create',
                        str(temporary / 'source.bundle'), 'refs/heads/session',
                        'refs/heads/runner'], check=True)
        write_session(temporary / 'session.toml', config)
        common.wait_ssh(instance_id, host, port)
        common.ssh(host, port, f'mkdir -p /workspace/job/logs /workspace/job/{session}')
        if config['precheck']:
            common.ssh(host, port, f'mkdir -p /workspace/job/{session}-safety')
        common.scp(host, port, '/workspace/job/', temporary / 'source.bundle',
                   temporary / 'session.toml', directory / 'params', SCRIPT_DIR / 'remote.py')
        if (directory / 'safety.params').is_file():
            common.scp(host, port, '/workspace/job/', directory / 'safety.params')
        if (directory / 'run').is_dir():
            common.scp(host, port, f'/workspace/job/{session}/', str(directory / 'run') + '/.')
        common.ssh(host, port, 'touch /workspace/job/started')
        cleanup = False
        return instance_id
    finally:
        if cleanup:
            common.destroy(instance_id)


def launch(session: str, minutes: int, offer_id: int | None, max_price: float | None,
           min_cpu_cores: int | None, min_cpu_ram: int | None, cpu_names: list[str] | None,
           wait_minutes: int | None) -> None:
    """出品を指定または自動選択し、作成から転送完了までを管理する。"""
    automatic = offer_id is None
    if automatic:
        if (max_price is None or min_cpu_cores is None or min_cpu_ram is None
                or not cpu_names or wait_minutes is None):
            raise ValueError('自動選択には--max-price、--min-cpu-cores、--min-cpu-ram、'
                             '--cpu、--waitが必要です')
    elif any(value is not None
             for value in (max_price, min_cpu_cores, min_cpu_ram, cpu_names, wait_minutes)):
        raise ValueError('--offerと自動選択の引数は同時に指定できません')
    remote.check_name(session)
    root = Path(git_output(SCRIPT_DIR.parent.parent, 'rev-parse', '--show-toplevel'))
    directory = root / 'data/vast-spsa' / session
    label = 'minase-spsa-' + session
    config = remote.load_session(directory)
    for key in ('commit', 'runner_commit'):
        config[key] = git_output(root, 'rev-parse', '--verify', '--end-of-options',
                                 config[key] + '^{commit}')
    if (directory / 'run').is_dir():
        remote.check_manifest(directory / 'run/manifest.json', config)
    for command in ('vastai', 'ssh', 'scp', 'rustc'):
        if shutil.which(command) is None:
            raise ValueError(f'必要なコマンドがありません: {command}')
    key = Path.home() / '.ssh/id_vastai'
    if not os.access(key, os.R_OK):
        raise ValueError(f'{key}がありません')
    with common.session_lock(directory / '.operation.lock'), \
            tempfile.TemporaryDirectory(prefix='.launch.', dir=directory) as name:
        temporary = Path(name)
        ids = common.ids(label)
        if ids:
            raise ValueError('同じラベルのインスタンスがあります: ' + '\n'.join(map(str, ids)))
        if not automatic:
            offers_output, offers = search_offers()
            check_offer(offers, str(offer_id))
            instance_id = start_offer(session, minutes, offer_id, offers_output, root,
                                      directory, label, config, temporary, False, None)
            assert instance_id is not None
            print(f'セッション {session} を開始しました。インスタンスID: {instance_id}')
            return

        assert max_price is not None and min_cpu_cores is not None and min_cpu_ram is not None
        assert cpu_names is not None and wait_minutes is not None
        wait_end = time.monotonic() + wait_minutes * 60
        while time.monotonic() < wait_end:
            offers_output, offers = search_offers()
            candidates = select_offers(offers, max_price, min_cpu_cores, min_cpu_ram, cpu_names)
            for offer in candidates:
                if time.monotonic() >= wait_end:
                    raise ValueError(f'{wait_minutes}分の待機時間内に出品を作成できませんでした')
                instance_id = start_offer(session, minutes, offer['id'], offers_output, root,
                                          directory, label, config, temporary, True, wait_end)
                if instance_id is not None:
                    print(f'セッション {session} を開始しました。インスタンスID: {instance_id}、'
                          f'出品ID: {offer["id"]}、cpu_name: {offer["cpu_name"]}、'
                          f'cpu_ram: {offer["cpu_ram"]}、'
                          f'dph_total: {offer["dph_total"]}')
                    return
            remaining = wait_end - time.monotonic()
            if remaining <= 0:
                break
            time.sleep(min(OFFER_RETRY_SECONDS, remaining))
    raise ValueError(f'{wait_minutes}分の待機時間内に出品を作成できませんでした')


def positive_integer(value: str) -> int:
    """正の整数の引数を解析する。"""
    if not re.fullmatch(r'[1-9][0-9]*', value):
        raise argparse.ArgumentTypeError('正の整数が必要です')
    return int(value)


def positive_number(value: str) -> float:
    """正の有限数の引数を解析する。"""
    try:
        number = float(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError('正の有限数が必要です') from error
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError('正の有限数が必要です')
    return number


def cpu_name_part(value: str) -> str:
    """空でないCPU名の一部を解析する。"""
    if not value:
        raise argparse.ArgumentTypeError('空でないCPU名の一部が必要です')
    return value


def main() -> int:
    """起動引数と実行時エラーを処理する。"""
    parser = argparse.ArgumentParser(
        usage=('launch.py <セッション名> <期限までの分数> --offer <出品ID>\n'
               '       launch.py <セッション名> <期限までの分数> '
               '--max-price <1時間あたりの米ドル> '
               '--min-cpu-cores <スレッド数> --min-cpu-ram <MB> --cpu <CPU名の一部> '
               '[--cpu <CPU名の一部> ...] --wait <分>'),
        add_help=False, allow_abbrev=False)
    parser.add_argument('session', metavar='セッション名')
    parser.add_argument('minutes', metavar='期限までの分数', type=positive_integer)
    choice = parser.add_mutually_exclusive_group(required=True)
    choice.add_argument('--offer', type=positive_integer)
    choice.add_argument('--max-price', type=positive_number)
    parser.add_argument('--min-cpu-cores', type=positive_integer)
    parser.add_argument('--min-cpu-ram', type=positive_integer)
    parser.add_argument('--cpu', action='append', type=cpu_name_part)
    parser.add_argument('--wait', type=positive_integer)
    try:
        options = parser.parse_args()
        automatic_values = (options.min_cpu_cores, options.min_cpu_ram,
                            options.cpu, options.wait)
        if options.offer is not None and any(value is not None for value in automatic_values):
            parser.error('--offerと自動選択の引数は同時に指定できません')
        if options.max_price is not None and any(value is None for value in automatic_values):
            parser.error('自動選択には--min-cpu-cores、--min-cpu-ram、--cpu、--waitが必要です')
    except SystemExit as error:
        return 0 if error.code == 0 else 1
    try:
        launch(options.session, options.minutes, options.offer, options.max_price,
               options.min_cpu_cores, options.min_cpu_ram, options.cpu, options.wait)
    except FAILURES as error:
        return common.cli_error(error)
    return 0


if __name__ == '__main__':
    sys.exit(main())
