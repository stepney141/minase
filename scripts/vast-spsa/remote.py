#!/usr/bin/env python3
"""調整機でビルド、速度測定、事前確認とSPSAを実行する。

使い方: python3 /workspace/job/remote.py <セッション名>
設定検査と計算の関数は、import remoteで利用できる。
"""

from contextlib import ExitStack
from datetime import datetime, timezone
from decimal import Decimal, ROUND_HALF_UP
import fcntl
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tomllib
from types import FrameType
from typing import Any

Session = dict[str, Any]


def check_name(name: str) -> None:
    """セッション名の文字と予約名を検査する。"""
    if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_-]*', name):
        raise ValueError('セッション名には英数字、ハイフン、アンダースコアだけを使ってください')
    if name in ('minase', 'logs', 'started'):
        raise ValueError(f'ジョブの予約名は使えません: {name}')


def load_session(directory: Path) -> Session:
    """TOMLの型、必須キー、値の範囲とパラメーターファイルを検査する。"""
    path = directory / 'session.toml'
    if not path.is_file():
        raise ValueError(f'session.tomlがありません: {directory}')
    with path.open('rb') as stream:
        config = tomllib.load(stream)
    required = {'commit', 'runner_commit', 'seed', 'iterations', 'pairs_per_iteration',
                'rules', 'precheck', 'local_nps', 'game_rss_bytes'}
    unknown = config.keys() - required - {'precheck_seed'}
    if unknown:
        raise ValueError(f'session.tomlに未知のキーがあります: {", ".join(sorted(unknown))}')
    for key in sorted(required):
        if key not in config:
            raise ValueError(f'session.tomlに{key}がありません')
    for key in ('commit', 'runner_commit', 'rules'):
        if type(config[key]) is not str or not config[key]:
            raise ValueError(f'{key}には空でない文字列が必要です')
    if type(config['precheck']) is not bool:
        raise ValueError('precheckはtrueかfalseにしてください')
    if config['precheck']:
        if 'precheck_seed' not in config:
            raise ValueError('session.tomlにprecheck_seedがありません')
    elif 'precheck_seed' in config:
        raise ValueError('precheck=falseではprecheck_seedを指定できません')
    for key in ('seed', 'iterations', 'pairs_per_iteration', 'game_rss_bytes', 'precheck_seed'):
        if key == 'precheck_seed' and not config['precheck']:
            continue
        value = config[key]
        minimum = 0 if key in ('seed', 'precheck_seed') else 1
        if type(value) is not int or not minimum <= value < 2**64:
            raise ValueError(f'{key}の整数値が不正です: {value}')
    nps = config['local_nps']
    if type(nps) not in (int, float) or nps <= 0 or (type(nps) is float and not math.isfinite(nps)):
        raise ValueError('local_npsは有限の正数にしてください')
    params = directory / 'params'
    if not params.is_file() or params.stat().st_size == 0:
        raise ValueError(f'paramsがないか空です: {directory}')
    safety = directory / 'safety.params'
    if config['precheck'] and (not safety.is_file() or safety.stat().st_size == 0):
        raise ValueError('precheck=trueですがsafety.paramsがないか空です')
    return config


def check_manifest(path: Path, config: Session) -> None:
    """再開用の記録が対象コミットとセッション設定に一致するか検査する。"""
    manifest = json.loads(path.read_text())
    if manifest['engine']['kind'] != 'commit' or manifest['engine']['hash'] != config['commit']:
        raise ValueError('再開用の記録とcommitが一致しません')
    for field in ('seed', 'iterations', 'pairs_per_iteration'):
        if manifest['settings'][field] != config[field]:
            raise ValueError(f'再開用の記録とsession.tomlの{field}が一致しません')
    if manifest['rules_source'] != config['rules']:
        raise ValueError('再開用の記録とrulesが一致しません')


def concurrency(cores: int, memory: int, rss: int) -> int:
    """物理コア数とメモリの両方を満たす同時対局数を求める。"""
    if min(cores, memory, rss) <= 0:
        raise ValueError('コア数とメモリは正数でなければなりません')
    count = min(cores - 1, memory * 4 // (rss * 5))
    if count < 1:
        raise ValueError('1局を実行するCPUまたはメモリが足りません')
    return count


def sizing(local: int | float | Decimal, remote: Decimal, iterations: int,
           pairs: int, count: int) -> tuple[Decimal, str, Decimal]:
    """換算係数、持ち時間と所要時間を十進数で算出する。"""
    local, remote, iterations, pairs, count = map(Decimal, map(str, (local, remote, iterations, pairs, count)))
    if any(not value.is_finite() or value <= 0 for value in (local, remote, iterations, pairs, count)):
        raise ValueError('速度と実行規模は有限の正数にしてください')
    factor = (local / remote).quantize(Decimal('0.01'), rounding=ROUND_HALF_UP)
    if factor <= 0:
        raise ValueError('丸めた換算係数が0です')
    base, increment = ((value * factor).quantize(Decimal(1), rounding=ROUND_HALF_UP)
                       for value in (10000, 100))
    hours = iterations * pairs * Decimal('9.05') * 16 * factor / (count * 3600)
    return factor, f'time={base}+{increment}', Decimal(f'{hours:.8f}')


def resources() -> tuple[int, int]:
    """物理コア数と、ホストとcgroupの小さい方のメモリ上限を読む。"""
    result = subprocess.run(['lscpu', '--json'], env={**os.environ, 'LC_ALL': 'C'},
                            check=True, capture_output=True, text=True)
    rows = {row['field']: row['data'] for row in json.loads(result.stdout)['lscpu']}
    cores = int(rows['Socket(s):']) * int(rows['Core(s) per socket:'])
    memory = next(int(line.split()[1]) * 1024 for line in Path('/proc/meminfo').read_text().splitlines()
                  if line.startswith('MemTotal:'))
    v1 = Path('/sys/fs/cgroup/memory/memory.limit_in_bytes')
    v2 = Path('/sys/fs/cgroup/memory.max')
    if v1.is_file():
        memory = min(memory, int(v1.read_text()))
    elif v2.is_file():
        limit = v2.read_text().strip()
        if limit != 'max':
            memory = min(memory, int(limit))
    else:
        raise ValueError('cgroupのメモリ上限を取得できません')
    return cores, memory


class Children:
    """実行中の子へ終了シグナルを伝え、終了まで回収する。"""

    def __init__(self) -> None:
        """管理対象の子プロセスを初期化する。"""
        self.processes: list[subprocess.Popen[str]] = []

    def start(self, args: list[str], **kwargs: Any) -> subprocess.Popen[str]:
        """子プロセスを生成し、終了時の回収対象へ登録する。"""
        process = subprocess.Popen(args, text=True, **kwargs)
        self.processes.append(process)
        return process

    def run(self, args: list[str], **kwargs: Any) -> str | None:
        """コマンドを実行し、失敗を例外として伝える。"""
        process = self.start(args, **kwargs)
        output, _ = process.communicate()
        self.processes.remove(process)
        if process.returncode:
            raise subprocess.CalledProcessError(process.returncode, args, output)
        return output

    def finish(self, signum: int) -> None:
        """すべての生存中の子へ同じシグナルを送り、終了を回収する。"""
        for process in self.processes:
            process.send_signal(signum)
        for process in self.processes:
            process.wait()
        self.processes.clear()

    def handle_signal(self, signum: int, frame: FrameType | None) -> None:
        """終了処理の再入を防ぎ、シグナルに対応する終了コードで終わる。"""
        signal.signal(signum, signal.SIG_IGN)
        raise SystemExit(128 + signum)


def bench(binary: Path, count: int, logs: Path, children: Children,
          *, cwd: Path | None = None) -> Decimal:
    """同時対局数と同じ本数のbenchを実行し、中央値NPSの平均を返す。"""
    with ExitStack() as stack:
        processes = []
        try:
            for index in range(1, count + 1):
                output = stack.enter_context((logs / f'bench-{index}.log').open('w'))
                processes.append(children.start([str(binary), 'dev', 'bench', '--depth', '9',
                                                 '--repetitions', '3'], cwd=cwd, stdout=output, stderr=subprocess.STDOUT))
            codes = [process.wait() for process in processes]
            for process in processes:
                children.processes.remove(process)
        except BaseException:
            children.finish(signal.SIGTERM)
            raise
    if any(codes):
        raise ValueError('benchが失敗しました')
    values = []
    for index in range(1, count + 1):
        matches = re.findall(r'^median:.*\bnps=([0-9]+)\s*$',
                             (logs / f'bench-{index}.log').read_text(), re.M)
        if len(matches) != 1 or int(matches[0]) <= 0:
            raise ValueError(f'bench-{index}.logの中央値NPSが不正です')
        values.append(Decimal(matches[0]))
    return sum(values) / len(values)


def summary_ok(path: Path, iterations: int, no_failures: bool) -> None:
    """最終要約の完了反復数と事前確認の異常件数を検査する。"""
    text = path.read_text()
    summaries = re.findall(r'^summary: applied=([0-9]+) valid_pairs=[0-9]+ discarded_pairs=[0-9]+$', text, re.M)
    failures = re.findall(r'^engine_failures: illegal_moves=([0-9]+) crashes=([0-9]+) timeouts=([0-9]+) time_forfeits=([0-9]+) rejected_moves=([0-9]+)$', text, re.M)
    if not summaries or not failures or int(summaries[-1]) != iterations:
        raise ValueError('最終要約がないか、完了反復数が一致しません')
    if no_failures and any(int(value) for value in failures[-1]):
        raise ValueError('時間切れの事前確認でエンジン異常が発生しました')


def prepare_run(path: Path) -> None:
    """新規作成のための空ディレクトリだけを取り除く。"""
    if path.is_dir() and not (path / 'manifest.json').is_file():
        try:
            path.rmdir()
        except OSError as error:
            raise ValueError(f'manifest.jsonのない実行ディレクトリが空ではありません: {path}') from error


def run_spsa(children: Children, args: list[str], log: Path, cwd: Path,
             iterations: int, no_failures: bool, failure_code: int) -> int:
    """SPSAの実行と最終要約の検査を行い、用途別の終了コードを返す。"""
    try:
        with log.open('w') as output:
            children.run(['target/release/minase', 'spsa', *args], cwd=cwd,
                         stdout=output, stderr=subprocess.STDOUT)
        summary_ok(log, iterations, no_failures)
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        print(error, file=sys.stderr)
        return failure_code
    return 0


def execute(session: str, job: Path, children: Children) -> int:
    """ビルド、計測、事前確認、本番または再開を順に実行する。"""
    config = load_session(job)
    logs = job / 'logs'
    repository = job / 'minase'
    if not repository.is_dir():
        cloning = job / 'minase-cloning'
        if cloning.is_dir():
            shutil.rmtree(cloning)
        with (logs / 'clone.log').open('w') as output:
            children.run(['git', 'clone', '--branch', 'runner', str(job / 'source.bundle'), str(cloning)],
                         stdout=output, stderr=subprocess.STDOUT)
        cloning.rename(repository)
    with (logs / 'clone.log').open('a') as output:
        children.run(['git', 'checkout', '--detach', config['runner_commit']], cwd=repository,
                     stdout=output, stderr=subprocess.STDOUT)
    config['commit'] = children.run(['git', 'rev-parse', '--verify', '--end-of-options',
                                    config['commit'] + '^{commit}'], cwd=repository, stdout=subprocess.PIPE).strip()
    if not (job / '.build-ready').is_file():
        with (logs / 'build.log').open('w') as output:
            children.run(['cargo', 'build', '--release', '--locked', '--bin', 'minase'],
                         cwd=repository, stdout=output, stderr=subprocess.STDOUT)
        with (logs / 'declared.params').open('w') as output, (logs / 'build.log').open('a') as errors:
            children.run(['target/release/minase', 'spsa', 'params', '--engine', 'commit:' + config['commit'],
                          '--rules', config['rules']], cwd=repository, stdout=output, stderr=errors)
        (job / '.build-ready').touch()
    binary = repository / 'target/match-cache' / (config['commit'] + '-tuning/minase')
    if not os.access(binary, os.X_OK):
        raise ValueError(f'調整用ビルドがありません: {binary}')
    cores, memory = resources()
    count = concurrency(cores, memory, config['game_rss_bytes'])
    remote_nps = bench(binary, count, logs, children, cwd=repository)
    factor, each, hours = sizing(config['local_nps'], remote_nps, config['iterations'],
                                config['pairs_per_iteration'], count)
    with (logs / 'sizing.txt').open('a') as output:
        output.write(f'measured_at={datetime.now(timezone.utc):%Y-%m-%dT%H:%M:%SZ}\n'
                     f'physical_cores={cores}\nmemory_bytes={memory}\nc={count}\n'
                     f'local_nps={config["local_nps"]}\nloaded_nps={remote_nps}\n'
                     f'f={factor}\neach={each}\nestimated_hours={hours:.8f}\n')
    run = job / session
    safety = job / (session + '-safety')
    if (run / 'manifest.json').is_file():
        check_manifest(run / 'manifest.json', config)
        with (logs / 'sizing.txt').open('a') as output:
            output.write(f'resume={run}; measured f and each are not applied\n')
        return run_spsa(children, ['--resume', str(run)], logs / (session + '.log'), repository,
                        config['iterations'], False, 5)
    if hours > 24:
        return 3
    if config['precheck']:
        safety_iterations = (13 * count + 15) // 16
        safety_log = logs / (session + '-safety.log')
        if (safety / 'manifest.json').is_file():
            manifest = json.loads((safety / 'manifest.json').read_text())
            safety_iterations = manifest['settings']['iterations']
            count = manifest['settings']['concurrency']
            each = manifest['each']
            try:
                summary_ok(safety_log, safety_iterations, True)
            except (FileNotFoundError, ValueError) as error:
                print(error, file=sys.stderr)
                if run_spsa(children, ['--resume', str(safety)], safety_log, repository,
                            safety_iterations, True, 4):
                    return 4
        else:
            prepare_run(safety)
            args = ['--run-dir', str(safety), '--seed', str(config['precheck_seed']),
                    '--engine', 'commit:' + config['commit'], '--params', str(job / 'safety.params'),
                    '--rules', config['rules'], '--each', each, '--concurrency', str(count),
                    '--iterations', str(safety_iterations), '--pairs-per-iteration', '8']
            if run_spsa(children, args, safety_log, repository, safety_iterations, True, 4):
                return 4
    with (logs / 'sizing.txt').open('a') as output:
        output.write(f'production_c={count}\nproduction_each={each}\n')
    prepare_run(run)
    args = ['--run-dir', str(run), '--seed', str(config['seed']), '--engine', 'commit:' + config['commit'],
            '--params', str(job / 'params'), '--rules', config['rules'], '--each', each,
            '--concurrency', str(count), '--iterations', str(config['iterations']),
            '--pairs-per-iteration', str(config['pairs_per_iteration'])]
    return run_spsa(children, args, logs / (session + '.log'), repository, config['iterations'], False, 5)


def remote_main(session: str, job: Path) -> int:
    """重複起動を拒み、終了コードを一時ファイルから置換して保存する。"""
    check_name(session)
    logs = job / 'logs'
    logs.mkdir(exist_ok=True)
    with (job / '.remote.lock').open('w') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError('remote.pyはすでに実行中です') from error
        children = Children()
        previous = {sig: signal.signal(sig, children.handle_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
        code = 1
        try:
            (logs / 'exit-status').unlink(missing_ok=True)
            code = execute(session, job, children)
        except SystemExit as error:
            code = int(error.code)
        except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
            print(error, file=sys.stderr)
            if isinstance(error, subprocess.CalledProcessError):
                code = error.returncode if error.returncode > 0 else 128 - error.returncode
        finally:
            children.finish(signal.SIGTERM)
            temporary = logs / '.exit-status.tmp'
            temporary.write_text(f'{code}\n')
            temporary.replace(logs / 'exit-status')
            for sig, handler in previous.items():
                signal.signal(sig, handler)
        return code


def main() -> int:
    """引数を検査し、調整機のジョブを起動する。"""
    if len(sys.argv) != 2:
        print('使い方: remote.py <セッション名>', file=sys.stderr)
        return 1
    try:
        return remote_main(sys.argv[1], Path('/workspace/job'))
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
