#!/usr/bin/env python3
"""10分ごとに実行記録を取得し、終了時に検査して調整機を削除または停止する。

使い方: python3 scripts/vast-spsa/fetch.py <セッション名> [--keep]

起動直後と以後10分ごとに取得する。
動いている調整機にはSSHを通したrsyncを使い、停止した調整機を含むその他の状態では`vastai copy`を使う。
取得はセッション内の一時ディレクトリへ行い、本番とログに加えて、有効なら事前確認の記録も転送する。
すべての転送を終えてから`incoming/`を置き換え、事前確認が有効なら`safety/`も置き換える。
`logs/`には取得したログを統合する。
`incoming/`の適用番号が1から連続するときだけ、再開用の`run/`を更新する。

調整機が動いたまま`deadline`を過ぎた場合と、調整機が`frozen`の場合は停止を要求する。
全反復の取得と`minase spsa apply`による検査に成功すると、終了コード0で終わる。
このとき`--keep`がなければ調整機を削除し、あれば調整機を残してIDを表示する。
それ以外の終わり方は`--keep`の有無で変わらない。
remote.pyの終了コード3を取得した場合は調整機を削除し、fetch.pyも終了コード3で終わる。
remote.pyのその他の失敗、終了コードの欠落または記録の未完了では、調整機を停止して終了コード1で終わる。
引数、記録、通信または検査の通常の失敗は終了コード1、外部コマンドの時間切れは124である。
外部コマンドのその他の失敗では正の終了コードを引き継ぎ、シグナルでは128にシグナル番号を加えた値を返す。
"""

import argparse
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / 'vast'))
import common
import remote
from sync_run import IncompleteRun, check_run, sync_run

FETCH_SECONDS = 600
FAILURES = (OSError, ValueError, KeyError, TypeError, common.VastError, subprocess.SubprocessError)


def fetch_once(instance_id: int, session: str, directory: Path, precheck: bool) -> None:
    """新しい場所へ取得し、完全な記録だけで再開用の写しを更新する。"""
    with tempfile.TemporaryDirectory(prefix='.fetch.', dir=directory) as name:
        temporary = Path(name)
        logs = directory / 'logs'
        logs.mkdir(exist_ok=True)
        (temporary / 'incoming').mkdir()
        (temporary / 'logs').mkdir()
        (logs / 'exit-status').unlink(missing_ok=True)
        state = common.instance(instance_id, 600)
        if state is None:
            raise ValueError('インスタンスがありません')
        status = state['actual_status']
        if status == 'running':
            transfer, method = common.rsync, 'ssh-rsync'
        else:
            transfer, method = common.copy, 'vastai-copy'
        with (logs / 'fetch.log').open('a') as output:
            output.write(f'{datetime.now(timezone.utc):%Y-%m-%dT%H:%M:%SZ} '
                         f'instance={instance_id} status={status} method={method}\n')
        transfer(instance_id, f'/workspace/job/{session}', temporary / 'incoming')
        if precheck:
            (temporary / 'safety').mkdir()
            transfer(instance_id, f'/workspace/job/{session}-safety', temporary / 'safety')
        transfer(instance_id, '/workspace/job/logs/', temporary / 'logs')
        folders = ['incoming', 'safety'] if precheck else ['incoming']
        for folder in folders:
            if (directory / folder).is_dir():
                (directory / folder).rename(temporary / ('old-' + folder))
            (temporary / folder).rename(directory / folder)
        shutil.copytree(temporary / 'logs', logs, dirs_exist_ok=True, symlinks=True)
        try:
            count = sync_run(directory / 'incoming' / session, directory / 'run')
            print(count)
        except (IncompleteRun, FileNotFoundError) as error:
            print(f'再開用の写しは更新しません: {error}', file=sys.stderr)
        except (OSError, ValueError, AttributeError) as error:
            raise ValueError(f'再開用の写しの処理が失敗しました: {error}') from error


def fetch_retry(failures: int, message: str) -> None:
    """通信障害では調整機を操作せず、6回の連続失敗で取得を打ち切る。"""
    print(f'{message}。連続失敗 {failures}/6回。取得済みの記録は残しています', file=sys.stderr)
    if failures >= 6:
        raise ValueError('6回連続で失敗したため取得を終了します。調整機の状態を確認してください')
    time.sleep(FETCH_SECONDS)


def validate_result(root: Path, directory: Path, config: remote.Session, instance_id: int) -> None:
    """記録と対象ソースの一時的な写しをapplyで検査し、失敗時は停止する。"""
    try:
        remote.check_manifest(directory / 'run/manifest.json', config)
        with tempfile.NamedTemporaryFile(prefix='.params.', suffix='.rs', dir=directory) as source:
            subprocess.run(['git', 'show', config['commit'] + ':crates/minase/src/search/alphabeta/params.rs'],
                           cwd=root, stdout=source, check=True)
            binary = root / 'target/release/minase'
            if not os.access(binary, os.X_OK):
                subprocess.run(['cargo', 'build', '--release', '--bin', 'minase'], cwd=root, check=True)
            try:
                subprocess.run([str(binary), 'spsa', 'apply', '--run-dir', str(directory / 'run'),
                                '--source', source.name], cwd=root, check=True)
            except subprocess.CalledProcessError as error:
                raise ValueError(f'インスタンス {instance_id}: applyによる検査に失敗しました') from error
    except FAILURES:
        common.stop(instance_id)
        raise


def fetch_loop(session: str, root: Path, directory: Path, config: remote.Session,
               deadline: int, keep: bool) -> int:
    """取得を繰り返し、終了状態と記録の整合性に応じて調整機を片付ける。"""
    failures = 0
    while True:
        try:
            ids = common.ids('minase-spsa-' + session)
        except FAILURES as error:
            print(error, file=sys.stderr)
            failures += 1
            fetch_retry(failures, f'セッション {session}: 調整機の照会に失敗しました')
            continue
        if not ids:
            raise ValueError('調整機がありません。取得済みの記録は残しています')
        if len(ids) != 1:
            raise ValueError('調整機が複数あります。操作しません: ' + '\n'.join(map(str, ids)))
        instance_id = ids[0]
        try:
            fetch_once(instance_id, session, directory, config['precheck'])
        except FAILURES as error:
            print(error, file=sys.stderr)
            failures += 1
            fetch_retry(failures, f'インスタンス {instance_id}: 取得に失敗しました')
            continue
        try:
            state = common.instance(instance_id, 600)
            if state is None:
                raise ValueError('インスタンスがありません')
            status = state['actual_status']
        except FAILURES as error:
            print(error, file=sys.stderr)
            failures += 1
            fetch_retry(failures, f'インスタンス {instance_id}: 状態の照会に失敗しました')
            continue
        failures = 0
        ended = False
        if status in ('stopped', 'exited', 'offline', 'unknown'):
            ended = True
        elif status == 'frozen' or (status == 'running' and time.time() >= deadline):
            common.stop(instance_id)
            ended = True
        elif status not in ('running', 'loading', 'rebooting'):
            raise ValueError(f'インスタンス {instance_id}: 未対応の状態です: {status}')
        if (directory / 'logs/exit-status').is_file():
            ended = True
        if not ended:
            time.sleep(FETCH_SECONDS)
            continue
        try:
            fetch_once(instance_id, session, directory, config['precheck'])
        except FAILURES as error:
            raise ValueError(f'インスタンス {instance_id}: 最終取得に失敗しました。調整機は操作せず、取得済みの記録を残します') from error
        exit_file = directory / 'logs/exit-status'
        exit_status = exit_file.read_text().rstrip('\n') if exit_file.is_file() else 'missing'
        complete = False
        if exit_status == '0':
            try:
                check_run(directory / 'run', config['iterations'])
                complete = True
            except (IncompleteRun, FileNotFoundError) as error:
                print(f'再開用の写しは更新しません: {error}', file=sys.stderr)
            except (OSError, ValueError, AttributeError) as error:
                print(f'再開用の写しの処理が失敗しました: {error}', file=sys.stderr)
        if complete:
            validate_result(root, directory, config, instance_id)
            if keep:
                print(f'セッション {session} の全反復を取得して検査し、'
                      f'調整機を残しました。インスタンスID: {instance_id}')
            else:
                common.destroy(instance_id)
                print(f'セッション {session} の全反復を取得し、検査と調整機の削除を完了しました')
            return 0
        if exit_status == '3':
            sizing = directory / 'logs/sizing.txt'
            if sizing.is_file():
                print(sizing.read_text(), end='')
            common.destroy(instance_id)
            print('所要時間の見込みが24時間を超えました', file=sys.stderr)
            return 3
        common.stop(instance_id)
        raise ValueError(f'セッション {session} は未完了です。終了コード: {exit_status}。インスタンス {instance_id} は停止しました')


def fetch(session: str, keep: bool) -> int:
    """ローカル設定と期限を検査し、セッションのロック中に取得する。"""
    remote.check_name(session)
    root = Path(subprocess.run(['git', '-C', str(SCRIPT_DIR.parent.parent), 'rev-parse', '--show-toplevel'],
                               check=True, text=True, stdout=subprocess.PIPE).stdout.strip())
    directory = root / 'data/vast-spsa' / session
    config = remote.load_session(directory)
    config['commit'] = subprocess.run(['git', '-C', str(root), 'rev-parse', '--verify', '--end-of-options',
                                       config['commit'] + '^{commit}'], check=True, text=True,
                                      stdout=subprocess.PIPE).stdout.strip()
    path = directory / 'deadline'
    if not path.is_file():
        raise ValueError('deadlineがありません')
    deadline = path.read_text().rstrip('\n')
    if not re.fullmatch(r'[1-9][0-9]*', deadline):
        raise ValueError('deadlineが不正です')
    with common.session_lock(directory / '.operation.lock'):
        return fetch_loop(session, root, directory, config, int(deadline), keep)


def main() -> int:
    """取得引数と実行時エラーを処理する。"""
    parser = argparse.ArgumentParser(usage='fetch.py <セッション名> [--keep]',
                                     add_help=False, allow_abbrev=False)
    parser.add_argument('session', metavar='セッション名')
    parser.add_argument('--keep', action='store_true')
    try:
        options = parser.parse_args()
    except SystemExit as error:
        return 0 if error.code == 0 else 1
    try:
        return fetch(options.session, options.keep)
    except FAILURES as error:
        return common.cli_error(error)


if __name__ == '__main__':
    sys.exit(main())
