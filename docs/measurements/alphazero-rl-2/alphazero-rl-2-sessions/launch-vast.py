#!/usr/bin/env python3
"""vast.aiの機械を借りて導入し、生成前にネットを照合してから学習の実行器を動かす。

作業機で次のように起動し、--の後ろにrun-cyclesの引数を指定する。
python3 launch-vast.py --offer ID --label LABEL --image IMAGE --hours HOURS --commit COMMIT --root ROOT --salvage-dir /ABS/PATH -- --cycles-dir cycles ...
起動側の引数はすべて必須で、commitは40桁の16進数字、hoursは正の有限数とする。
生成と学習のコミットはcommitと一致させ、機械を作る前に実行器の引数を検査する。
外部コマンドを呼ぶ前に、最初の生成に使うネットとPyTorchの基準出力がrootの下にあることを確かめる。
外部コマンドを呼ぶ前に系列の記録を検査し、first-cycleが記録済みなら記録の最後のサイクルの次から起動するよう案内して失敗する。
実行器のmachine、vast-instance、deadline、rootはこのスクリプトが指定し、sessionとcycle-cuは指定できない。
ソースの書庫を用意して自己停止を予約し、貸出期限と自己停止時刻の早いほうを終了期限にする。
導入後にネットと基準出力を送り、SHA-256を確認してからcheck_outputs.pyをbashで実行する。
照合は20秒ごとに状態を調べ、転送を含めて30分を超えたら失敗とする。
終了コードにかかわらず照合の出力とログをrootへ受け取り、trt-cacheを除く全ファイルのSHA-256を照合する。
照合の転送とSSHの時間の上限は、対象のバイト数を毎秒1 MBで割った秒数に120秒を足して求める。
照合が時間切れになった場合も、出力の受け取りを試みてから機械を削除する。
導入中、照合中、および最後の受け取りの前後に終了期限を確認し、実行器の起動中も60秒ごとに確認する。
期限を超えた機械には停止を1回だけ要求し、実行器が自ら終了するまで監視する。
実行器が異常終了した場合は工程の稼働を調べ、動いている工程があるか照会に失敗すれば機械を残す。
正常終了した場合と稼働中の工程がない場合は、NASとの差分をsalvage-dirのインスタンス別ディレクトリへ受け取り、SHA-256を照合する。
SSHの準備、導入、または生成前の照合に失敗した機械と、最後の受け取りと照合に成功した機械を削除する。
生成前の照合は、終了コード0で出力の受け取りとSHA-256の照合に成功した場合だけ実行器へ進む。
最後の受け取りか照合に失敗した機械は残し、自己停止までに受け取り直して削除するよう通知する。
停止要求または削除が失敗した場合は、SSHの準備が済んでいればコンテナ専用の鍵を使って停止を要求する。
それ以外の失敗では機械を削除せず、起動確認に失敗した場合の削除だけはcommon.wait_runningに任せる。
各出来事はroot/vast-instances.jsonlへ追記し、1行ずつfsyncする。
料金の欄は選別せず、作成直後と削除直前のインスタンスのJSON全体を記録する。
生成前の照合のネット、基準出力、出力先、終了コード、およびsummary.jsonの内容はnetwork_check_finished行に記録する。
before_destroyはsalvage_finished行、SSHの準備または導入の失敗時はsetup_failed行、生成前の照合の失敗時はnetwork_check_failed行に入れる。
"""

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time


SCRIPT_DIR = Path(__file__).resolve().parent
COMMON_PATH = SCRIPT_DIR.parents[3] / "scripts/vast/common.py"
spec = importlib.util.spec_from_file_location("launch_vast_common", COMMON_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError(f"vast.aiの操作モジュールを読めません: {COMMON_PATH}")
common = importlib.util.module_from_spec(spec)
spec.loader.exec_module(common)

FAILURES = (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError)
FORBIDDEN = ("--machine", "--vast-instance", "--deadline", "--session", "--cycle-cu", "--root")

VALIDATE_RUNNER = """import sys
from minase_zero.__main__ import _parse
from minase_zero.runner import Options
try:
    arguments = vars(_parse(sys.argv[1:]))
except SystemExit as error:
    raise ValueError('run-cycles arguments must describe a run') from error
arguments.pop('command')
Options(**arguments)
"""

RUNNING_JOBS = """import json, os, pathlib, re, sys
root = pathlib.Path(sys.argv[1])
jobs = []
for marker in sorted(root.rglob('*.pid')):
    if marker.with_suffix('.exit').exists():
        continue
    pid = marker.read_text().strip()
    if re.fullmatch(r'[1-9][0-9]*', pid) is None:
        raise ValueError('invalid PID: ' + str(marker))
    try:
        os.kill(int(pid), 0)
        state = pathlib.Path('/proc/' + pid + '/stat').read_text().rsplit(')', 1)[1].split()[0]
    except (ProcessLookupError, FileNotFoundError):
        continue
    if state not in ('Z', 'X'):
        jobs.append(marker.with_suffix('').relative_to(root).as_posix())
print(json.dumps(jobs))
"""

# 鍵も例外の詳細も作業機へ返さず、PID 1の環境にある資格情報だけで要求する。
CONTAINER_STOP = """import pathlib, sys, urllib.request
try:
    values = dict(item.split(b'=', 1) for item in pathlib.Path('/proc/1/environ').read_bytes().split(b'\\0') if b'=' in item)
    instance = values[b'CONTAINER_ID'].decode('ascii')
    if not instance.isdecimal() or int(instance) != int(sys.argv[1]):
        raise ValueError('container ID mismatch')
    key = values[b'CONTAINER_API_KEY'].decode('ascii')
    if not key:
        raise ValueError('empty container key')
    request = urllib.request.Request(
        'https://console.vast.ai/api/v0/instances/' + instance + '/',
        data=b'{"state":"stopped"}', method='PUT',
        headers={'Authorization': 'Bearer ' + key, 'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=45) as response:
        status = response.status
    if not 200 <= status < 300:
        raise ValueError('stop request rejected')
    print(status)
except Exception:
    print('container stop request failed', file=sys.stderr)
    sys.exit(1)
"""


def positive_hours(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0 or not math.isfinite(number * 3600):
        raise argparse.ArgumentTypeError("--hoursには正の有限数が必要です")
    return number


def arguments(argv):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for name in ("offer", "label", "image", "commit"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--hours", type=positive_hours, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--salvage-dir", type=Path, required=True)
    separator = argv.index("--") if "--" in argv else len(argv)
    options = parser.parse_args(argv[:separator])
    if separator == len(argv):
        parser.error("--の後ろにrun-cyclesの引数が必要です")
    options.runner_args = argv[separator + 1:]
    if re.fullmatch(r"[0-9a-fA-F]{40}", options.commit) is None:
        parser.error("--commitには40桁の16進数字が必要です")
    if not options.salvage_dir.is_absolute():
        parser.error("--salvage-dirには絶対パスが必要です")
    if any(not getattr(options, name).strip() for name in ("offer", "label", "image")):
        parser.error("--offer、--label、--imageは空にできません")
    for argument in options.runner_args:
        option = argument.split("=", 1)[0]
        # 実行器のargparseが受理する省略形でも、予約した値を上書きさせない。
        if option.startswith("--") and any(name.startswith(option) for name in FORBIDDEN):
            parser.error(f"--の後ろには指定できません: {option}")
    cycles_parser = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    cycles_parser.add_argument("--cycles-dir", type=Path, required=True)
    cycles_parser.add_argument("--base-cycles-dir", type=Path, required=True)
    cycles_parser.add_argument("--first-cycle", type=int, required=True)
    cycles_parser.add_argument("--branch-from", type=int, required=True)
    for name in ("generation-commit", "trainer-commit"):
        cycles_parser.add_argument(f"--{name}", required=True)
    cycles, _ = cycles_parser.parse_known_args(options.runner_args)
    for name in ("generation_commit", "trainer_commit"):
        if getattr(cycles, name) != options.commit:
            parser.error(f"--{name.replace('_', '-')}は--commitと同じ40桁の値が必要です")
    directory = cycles.cycles_dir
    if (directory.is_absolute() or directory == Path(".") or ".." in directory.parts
            or any(character.isspace() for character in str(directory))):
        parser.error("--cycles-dirには空白を含まないroot内の相対ディレクトリが必要です")
    base = cycles.base_cycles_dir
    if base.is_absolute() or base == Path(".") or ".." in base.parts:
        parser.error("--base-cycles-dirにはroot内の相対ディレクトリが必要です")
    if not 0 <= cycles.branch_from < cycles.first_cycle:
        parser.error("--branch-fromには0以上かつ--first-cycle未満の値が必要です")
    options.cycles_dir = directory
    options.first_cycle = cycles.first_cycle
    # runner.pyのOptions.cycle_pathとRunner.runのnetworkの規則に合わせる。
    # 外部コマンドを呼ぶ前に入力ファイルを検査するため、ここでパスを求める。
    previous = cycles.first_cycle - 1
    source = base if previous <= cycles.branch_from else directory
    cycle = source / f"cycle-{previous:04d}"
    options.network = cycle / ("model.onnx" if cycles.first_cycle == 1 else "train/model.onnx")
    options.reference = cycle / "check/pytorch.json"
    # 実行器ではcwdが変わるので、相対指定も起動元の同じディレクトリを指すようにする。
    options.root = options.root.absolute()
    return options


def utc(timestamp):
    return datetime.fromtimestamp(timestamp, timezone.utc).isoformat()


def validate_runner(tree, options):
    command = ["uv", "run", "--frozen", "python", "-c", VALIDATE_RUNNER, "run-cycles",
               "--machine", "vast", "--vast-instance", "1", "--deadline", utc(time.time() + 3600),
               "--root", str(options.root), *options.runner_args]
    try:
        subprocess.run(command, cwd=tree / "tools/train/zero", check=True,
                       capture_output=True, text=True, timeout=600)
    except subprocess.CalledProcessError as error:
        print(error.stderr, end="", file=sys.stderr)
        raise ValueError("実行器の引数の検査に失敗しました") from error


class Journal:
    def __init__(self, root, instance):
        self.path = root / "vast-instances.jsonl"
        self.instance = instance

    def write(self, event, **fields):
        row = {"instance": self.instance, "event": event, "at": utc(time.time()), **fields}
        with self.path.open("a", encoding="utf-8") as output:
            output.write(json.dumps(row, allow_nan=False) + "\n")
            output.flush()
            os.fsync(output.fileno())


class Remote:
    def __init__(self, instance, host, port):
        self.host = host
        self.port = port
        self.ssh = ["ssh", *common.ssh_options(port, "-p"), "-o", "ControlMaster=auto",
                    "-o", f"ControlPath=/tmp/vast-cm-{instance}", "-o", "ControlPersist=900"]
        address = f"[{host}]" if ":" in host else host
        self.address = f"root@{address}"

    def shell(self, command, timeout, data=None):
        return subprocess.run([*self.ssh, f"root@{self.host}", command], input=data,
                              capture_output=True, check=True, timeout=timeout).stdout

    def rsync(self, sources, destination, timeout, *options):
        subprocess.run(["rsync", "-a", *options, "-e", shlex.join(self.ssh), "--",
                        *map(str, sources), str(destination)], check=True, timeout=timeout,
                       capture_output=True)


def instance_snapshot(instance, timeout):
    state = common.instance(instance, timeout)
    if not isinstance(state, dict):
        raise ValueError(f"インスタンス {instance} の情報がありません")
    return state


def container_stop(remote, journal, reason):
    try:
        response = remote.shell(shlex.join(["python3", "-c", CONTAINER_STOP, str(journal.instance)]), 60)
        if re.fullmatch(rb"2[0-9]{2}", response.strip()) is None:
            raise ValueError("コンテナの停止要求の応答が不正です")
    except FAILURES as error:
        journal.write("container_stop_requested", reason=reason, success=False, error=str(error))
        print(f"インスタンス {journal.instance}: コンテナ専用の鍵による停止に失敗しました", file=sys.stderr)
    else:
        journal.write("container_stop_requested", reason=reason, success=True,
                      response=response.decode("ascii").strip())


def destroy(remote, journal, preceding_event, **fields):
    # 列挙されたevent名を保ち、削除要求の前に課金情報をfsyncする。
    try:
        fields["before_destroy"] = instance_snapshot(journal.instance, 600)
    except FAILURES as error:
        fields["before_destroy_error"] = str(error)
    journal.write(preceding_event, **fields)
    try:
        common.destroy(journal.instance)
    except FAILURES as error:
        journal.write("destroy_failed", error=str(error))
        if remote is not None:
            container_stop(remote, journal, "destroy_failed")
        print(f"インスタンス {journal.instance}: 削除に失敗した。"
              "`vastai tfa login` で認証し直してから削除すること", file=sys.stderr)
        return False
    journal.write("destroyed")
    return True


def setup(remote, archive, check_deadline):
    # 最小構成のイメージにはrsyncがまだないため、導入前の転送はSCPを使う。
    check_deadline()
    common.run_timed(["scp", "-r", *common.ssh_options(remote.port, "-P"),
                      str(archive), str(SCRIPT_DIR / "vast-setup.sh"),
                      f"root@[{remote.host}]:/workspace/"], 600)
    check_deadline()
    end = time.monotonic() + 3600
    remaining = end - time.monotonic()
    if remaining <= 0:
        raise RuntimeError("導入が60分を超えました")
    command = (f"setsid nohup bash /workspace/vast-setup.sh /workspace/{archive.name} "
               "> /workspace/setup.log 2>&1 < /dev/null &\nprintf '%s\\n' \"$!\"")
    pid = remote.shell(command, min(60, remaining)).strip()
    if re.fullmatch(rb"[1-9][0-9]*", pid) is None:
        raise ValueError("導入プロセスの番号を取得できません")
    poll = ("if grep -Fxq setup-done /workspace/setup.log; then echo done; "
            f"elif kill -0 {pid.decode('ascii')} 2>/dev/null; then echo running; "
            "else echo failed; fi")
    while (remaining := end - time.monotonic()) > 0:
        check_deadline()
        status = remote.shell(poll, min(60, remaining)).strip()
        if status == b"done":
            return
        if status == b"failed":
            raise RuntimeError("setup-doneを出さずに導入プロセスが終了しました")
        if status != b"running":
            raise ValueError(f"導入の状態が不正です: {status!r}")
        remaining = end - time.monotonic()
        if remaining > 0:
            time.sleep(min(20, remaining))
    raise RuntimeError("導入が60分を超えました")


# 回収対象だけを列挙し、機械ごとに作るTensorRTのキャッシュには立ち入らない。
CHECK_FILES = """import json, os, pathlib, sys
root = pathlib.Path('/workspace/az')
out = pathlib.Path(sys.argv[1])
files = {}
def add(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('not a regular file: ' + str(path))
    files[path.relative_to(root).as_posix()] = path.stat().st_size
if (root / out).is_symlink():
    raise ValueError('output directory is a symlink')
def walk_error(error):
    raise error
for directory, children, names in os.walk(root / out, onerror=walk_error):
    directory = pathlib.Path(directory)
    if directory == root / out and 'trt-cache' in children:
        children.remove('trt-cache')
    for name in children:
        if (directory / name).is_symlink():
            raise ValueError('output directory is a symlink')
    for name in names:
        add(directory / name)
log = root / (str(out) + '.log')
if log.exists() or log.is_symlink():
    add(log)
print(json.dumps(files))
"""


def transfer_timeout(size):
    return 120 + size / 1_000_000


def verify_files(remote, root, names, timeout):
    output = remote.shell("cd /workspace/az && xargs -0 -r sha256sum --zero --", timeout,
                          b"".join(os.fsencode(name) + b"\0" for name in names))
    records = output.split(b"\0")
    if records.pop() != b"":
        raise ValueError("遠隔のSHA-256の出力が途中で終わっています")
    hashes = {}
    for record in records:
        match = re.fullmatch(rb"([0-9a-f]{64}) [ *](.+)", record, re.DOTALL)
        if match is None or os.fsdecode(match[2]) in hashes:
            raise ValueError("遠隔のSHA-256の出力が不正です")
        hashes[os.fsdecode(match[2])] = match[1].decode("ascii")
    if set(hashes) != set(names):
        raise ValueError("遠隔のSHA-256のファイル名が要求と一致しません")
    for name in names:
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"通常のファイルではありません: {path}")
        digest = hashlib.sha256()
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
        if digest.hexdigest() != hashes[name]:
            raise ValueError(f"SHA-256が一致しません: {name}")


def receive_check(remote, root, out, check_deadline):
    check_deadline()
    try:
        inventory = json.loads(remote.shell(shlex.join(["python3", "-c", CHECK_FILES, out]),
                                            transfer_timeout(0)))
        if not isinstance(inventory, dict):
            raise ValueError("照合の出力の一覧が不正です")
        for name, size in inventory.items():
            path = Path(name)
            if (path.is_absolute() or ".." in path.parts
                    or not (name == out + ".log" or path.is_relative_to(out))
                    or path.is_relative_to(out + "/trt-cache")
                    or type(size) is not int or size < 0):
                raise ValueError("照合の出力のパスか大きさが不正です")
        timeout = transfer_timeout(sum(inventory.values()))
        check_deadline()
        remote.rsync([f"{remote.address}:/workspace/az/./{out}",
                      f"{remote.address}:/workspace/az/./{out}.log"], str(root) + "/", timeout,
                     "--relative", "--protect-args", f"--exclude=/{out}/trt-cache/")
        check_deadline()
        paths = [path for path in (root / out).rglob("*") if path.is_symlink() or not path.is_dir()]
        paths.append(root / (out + ".log"))
        if {path.relative_to(root).as_posix() for path in paths} != set(inventory):
            raise ValueError("受け取った照合の出力が機械の一覧と一致しません")
        verify_files(remote, root, sorted(inventory), timeout)
        summary = json.loads((root / out / "summary.json").read_text(encoding="utf-8"))
        if not isinstance(summary, dict):
            raise ValueError("照合のsummary.jsonがJSONオブジェクトではありません")
        json.dumps(summary, allow_nan=False)
        return summary
    finally:
        check_deadline()


def check_output(options, instance):
    return (options.cycles_dir / f"cycle-{options.first_cycle:04d}"
            / f"pre-generation-check-{instance}").as_posix()


def network_check_start(script, out):
    # mkdirまで背景化すると、子シェルがSSHの標準出力を開いたまま残る。
    return (shlex.join(["mkdir", "-p", "--", out]) + " || exit 1\n"
            + shlex.join(["setsid", "nohup", "bash", "-c", script])
            + f" > {shlex.quote(out + '.log')} 2>&1 < /dev/null &\n"
              "printf '%s\\n' \"$!\"")


def network_check(remote, options, journal, deadline, check_deadline):
    out = check_output(options, journal.instance)
    fields = {"network": options.network.as_posix(), "reference": options.reference.as_posix(),
              "out": out, "exit_code": None, "summary": None}
    end = time.monotonic() + 1800

    def check():
        check_deadline()
        if time.time() >= deadline:
            raise RuntimeError("生成前の照合中に機械の終了期限を超えました")
        if time.monotonic() >= end:
            raise RuntimeError("生成前の照合が30分を超えました")

    def timeout(size):
        check()
        return min(transfer_timeout(size), end - time.monotonic(), deadline - time.time())

    try:
        for name in (fields["network"], fields["reference"]):
            path = options.root / name
            size = path.stat().st_size
            parent = str(Path("/workspace/az") / Path(name).parent)
            remote.shell(shlex.join(["mkdir", "-p", "--", parent]), timeout(0))
            remote.rsync([path], f"{remote.address}:/workspace/az/{name}", timeout(size), "--protect-args")
            verify_files(remote, options.root, [name], timeout(size))
        absolute_out = "/workspace/az/" + out
        marker = f"/workspace/pre-generation-check-{journal.instance}.exit"
        command = ("cd /workspace/src && source tools/train/zero/env.sh && "
                   "export XDG_CACHE_HOME=/workspace/cache && " + shlex.join([
                       "python3", "-I",
                       "docs/measurements/alphazero-rl-2/alphazero-rl-2-routine-scripts/check_outputs.py",
                       "--net", "/workspace/az/" + fields["network"],
                       "--reference", "/workspace/az/" + fields["reference"], "--out", absolute_out]))
        # 終了印はOUTの外に置き、check_outputs.pyが要求する空の出力先を保つ。
        script = (f"( {command} ); code=$?; "
                  f"printf '%s\\n' \"$code\" > {shlex.quote(marker + '.tmp')} && "
                  + shlex.join(["mv", "--", marker + ".tmp", marker]))
        start = network_check_start(script, absolute_out)
        pid = remote.shell(start, timeout(0)).strip()
        if re.fullmatch(rb"[1-9][0-9]*", pid) is None:
            raise ValueError("照合プロセスの番号を取得できません")
        poll = (f"if test -f {shlex.quote(marker)}; then cat {shlex.quote(marker)}; "
                f"elif kill -0 {pid.decode('ascii')} 2>/dev/null; then echo running; "
                "else echo failed; fi")
        while True:
            status = remote.shell(poll, min(60, timeout(0))).strip()
            if re.fullmatch(rb"[0-9]+", status):
                fields["exit_code"] = int(status)
                check()
                break
            check()
            if status != b"running":
                raise RuntimeError(f"生成前の照合の状態が不正です: {status!r}")
            time.sleep(min(20, end - time.monotonic(), deadline - time.time()))
    except FAILURES as error:
        fields["error"] = str(error)
    # 異常終了や時間切れでも回収を試みる。回収できなくても機械は削除する。
    try:
        fields["summary"] = receive_check(remote, options.root, out, check_deadline)
        check()
    except FAILURES as error:
        fields["receive_error"] = str(error)
    journal.write("network_check_finished", **fields)
    if fields["exit_code"] != 0 or "error" in fields or "receive_error" in fields:
        destroy(remote, journal, "network_check_failed", **fields)
        return False
    return True


def stop_at_deadline(remote, journal):
    try:
        state = instance_snapshot(journal.instance, 60)
        status = state["actual_status"]
        if not isinstance(status, str) or not status:
            raise ValueError("機械の状態がありません")
    except FAILURES as error:
        container_stop(remote, journal, f"deadline: 状態の照会に失敗: {error}")
        return
    if status in ("stopped", "exited"):
        return
    try:
        common.stop(journal.instance)
    except FAILURES as error:
        journal.write("stop_requested", success=False, error=str(error))
        container_stop(remote, journal, "deadline: stop_failed")
    else:
        journal.write("stop_requested", success=True)


def run_cycles(tree, options, journal, deadline, check_deadline):
    command = ["uv", "run", "--frozen", "python", "-m", "minase_zero", "run-cycles",
               "--machine", "vast", "--vast-instance", str(journal.instance),
               "--deadline", utc(deadline), "--root", str(options.root), *options.runner_args]
    process = subprocess.Popen(command, cwd=tree / "tools/train/zero")
    while process.poll() is None:
        check_deadline()
        try:
            return process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            pass
    return process.returncode


def cycle_records(options):
    path = options.root / f"{options.cycles_dir}.jsonl"
    if not path.exists():
        # 完了したサイクルが0件なら、実行器は台帳を作成しない。
        return []
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if not isinstance(row, dict) or "machine" not in row or not isinstance(row["machine"], dict):
            raise ValueError(f"サイクルの記録が不正です: {path}")
        if "cycle" not in row or type(row["cycle"]) is not int or row["cycle"] <= 0:
            raise ValueError(f"サイクルの番号が不正です: {path}")
        rows.append(row)
    return rows


def completed_cycles(options, instance):
    return [row["cycle"] for row in cycle_records(options)
            if "vast_instance" in row["machine"] and row["machine"]["vast_instance"] == instance]


def running_jobs(remote):
    output = remote.shell(shlex.join(["python3", "-c", RUNNING_JOBS, "/workspace/az"]), 60)
    jobs = json.loads(output)
    if not isinstance(jobs, list) or any(not isinstance(name, str) or not name for name in jobs):
        raise ValueError("稼働中の工程の応答が不正です")
    return jobs


def salvage(remote, root, destination, journal, check_deadline, check_out):
    check_deadline()
    operation = "rsync"
    pending = []
    files_known = False
    try:
        destination.mkdir(parents=True, exist_ok=True)
        remote.rsync([f"{remote.address}:/workspace/az/"], str(destination) + "/", 3600,
                     f"--compare-dest={root}/", f"--exclude=/{check_out}/trt-cache/")
        operation = "sha256"
        paths = sorted(path for path in destination.rglob("*") if path.is_symlink() or not path.is_dir())
        pending = [path.relative_to(destination).as_posix() for path in paths]
        files_known = True
        if any(path.is_symlink() or not path.is_file() for path in paths):
            raise ValueError("回収した成果物に通常のファイル以外が含まれます")
        if paths:
            output = remote.shell("cd /workspace/az && xargs -0 -r sha256sum --zero --", 3600,
                                  b"".join(os.fsencode(name) + b"\0" for name in pending))
            records = output.split(b"\0")
            if records.pop() != b"":
                raise ValueError("遠隔のSHA-256の出力が途中で終わっています")
            hashes = {}
            for record in records:
                match = re.fullmatch(rb"([0-9a-f]{64}) [ *](.+)", record, re.DOTALL)
                if match is None or os.fsdecode(match[2]) in hashes:
                    raise ValueError("遠隔のSHA-256の出力が不正です")
                hashes[os.fsdecode(match[2])] = match[1].decode("ascii")
            if set(hashes) != set(pending):
                raise ValueError("遠隔のSHA-256のファイル名が要求と一致しません")
            for path, name in zip(paths, pending.copy()):
                digest = hashlib.sha256()
                with path.open("rb") as source:
                    for chunk in iter(lambda: source.read(1024 * 1024), b""):
                        digest.update(chunk)
                if digest.hexdigest() == hashes[name]:
                    pending.remove(name)
            if pending:
                raise ValueError("SHA-256が一致しません")
    except FAILURES as error:
        journal.write("salvage_failed", operation=operation, error=str(error),
                      unreceived_files=pending, files_complete=files_known)
        print(f"インスタンス {journal.instance}: 受け取りに失敗したので機械を残した。"
              "自己停止の時刻まで受け取り直してから削除すること", file=sys.stderr)
        return False
    finally:
        check_deadline()
    return destroy(remote, journal, "salvage_finished", files=[path.relative_to(destination).as_posix() for path in paths])


def launch(options):
    cycles = [row["cycle"] for row in cycle_records(options)]
    if options.first_cycle in cycles:
        raise ValueError(f"--first-cycle {options.first_cycle}は記録済みです: "
                         f"{options.root / f'{options.cycles_dir}.jsonl'}。"
                         f"記録の最後のサイクルの次に当たる--first-cycle {max(cycles) + 1}から起動してください")
    for relative in (options.network, options.reference):
        path = options.root / relative
        if not path.resolve().is_relative_to(options.root.resolve()) or not path.is_file():
            raise ValueError(f"rootの下に照合の入力ファイルがありません: {path}")
    key = Path.home() / ".ssh/id_vastai"
    if not key.is_file():
        raise ValueError(f"vast.ai専用のSSH鍵がありません: {key}")
    output = subprocess.run(["git", "-C", str(SCRIPT_DIR), "rev-parse", "--show-toplevel"],
                            check=True, capture_output=True, text=True, timeout=60).stdout.strip()
    if not output:
        raise ValueError("作業ツリーの根を取得できません")
    tree = Path(output)
    validate_runner(tree, options)
    archive = options.root / "src" / f"minase-{options.commit}.tar"
    archive.parent.mkdir(parents=True, exist_ok=True)
    if not archive.is_file():
        subprocess.run(["git", "-C", str(tree), "archive", "--format=tar", "-o", str(archive), options.commit],
                       check=True, timeout=600)
    self_stop = int(time.time() + options.hours * 3600)
    utc(self_stop)
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", suffix=".sh") as onstart:
        onstart.write(common.onstart_prefix(self_stop))
        onstart.flush()
        instance = common.create(options.offer, options.label, Path(onstart.name), options.image, 100)
    journal = Journal(options.root, instance)
    created = {"offer": options.offer, "label": options.label, "image": options.image,
               "disk_gb": 100, "self_stop": self_stop}
    try:
        created["instance_info"] = instance_snapshot(instance, 60)
    except FAILURES as error:
        # 照会に失敗しても、作成済みの番号と停止予約は必ず残す。
        journal.write("created", **created, instance_error=str(error))
        raise RuntimeError(f"インスタンス {instance}: 作成直後の照会に失敗しました。機械を残します") from error
    journal.write("created", **created)
    try:
        common.wait_running(instance, 900)
        journal.write("running")
        operation = "ssh_address"
        try:
            host, port = common.ssh_address(instance, 600)
            operation = "wait_ssh"
            common.wait_ssh(instance, host, port)
        except FAILURES as error:
            destroy(None, journal, "setup_failed", operation=operation, error=str(error))
            return 1
        remote = Remote(instance, host, port)
        try:
            state = instance_snapshot(instance, 600)
            end_date = state["end_date"]
            if type(end_date) not in (float, int) or not math.isfinite(end_date) or end_date <= 0:
                raise ValueError("end_dateには正の有限数が必要です")
            deadline = min(self_stop, end_date)
            journal.write("deadline", end_date=end_date, deadline=deadline)
        except FAILURES as error:
            destroy(remote, journal, "setup_failed", error=str(error))
            return 1
        checked_deadline = False

        def check_deadline():
            nonlocal checked_deadline
            if not checked_deadline and time.time() >= deadline:
                checked_deadline = True
                stop_at_deadline(remote, journal)

        setup_fields = {}
        setup_error = None
        try:
            setup(remote, archive, check_deadline)
        except FAILURES as error:
            setup_error = str(error)
        finally:
            check_deadline()
            try:
                log = options.salvage_dir / str(instance) / "setup.log"
                log.parent.mkdir(parents=True, exist_ok=True)
                subprocess.run(["scp", *common.ssh_options(port, "-P"), "--",
                                f"{remote.address}:/workspace/setup.log", str(log)],
                               check=True, capture_output=True, timeout=60)
            except FAILURES as error:
                setup_fields["setup_log_error"] = str(error)
                print(f"インスタンス {instance}: setup.logの受け取りに失敗しました: {error}", file=sys.stderr)
            finally:
                check_deadline()
        if setup_error is not None:
            destroy(remote, journal, "setup_failed", error=setup_error, **setup_fields)
            return 1
        journal.write("setup_finished", **setup_fields)
        if not network_check(remote, options, journal, deadline, check_deadline):
            return 1
        runner_fields = {}
        try:
            code = run_cycles(tree, options, journal, deadline, check_deadline)
        except FAILURES as error:
            code = 1
            runner_fields["error"] = str(error)
        try:
            runner_fields["cycles"] = completed_cycles(options, instance)
        except FAILURES as error:
            runner_fields["cycles_error"] = str(error)
            print(f"インスタンス {instance}: サイクルの記録を読めません: {error}", file=sys.stderr)
        journal.write("runner_exited", exit_code=code, **runner_fields)
        check_deadline()
        if code != 0:
            try:
                jobs = running_jobs(remote)
            except FAILURES as error:
                journal.write("kept_running_jobs", jobs=None, error=str(error))
                print(f"インスタンス {instance}: 工程の稼働確認に失敗したので機械を残した: {error}", file=sys.stderr)
                return 1
            if jobs:
                journal.write("kept_running_jobs", jobs=jobs)
                print(f"インスタンス {instance}: 機械の上で工程が動いているので機械を残した", file=sys.stderr)
                return 1
        if not salvage(remote, options.root, options.salvage_dir / str(instance) / "az", journal,
                       check_deadline, check_output(options, instance)):
            return 1
        return 1 if "cycles_error" in runner_fields else code
    except FAILURES as error:
        raise RuntimeError(f"インスタンス {instance}: {error}") from error


def main(argv=None):
    options = arguments(sys.argv[1:] if argv is None else argv)
    try:
        return launch(options)
    except FAILURES as error:
        print(error, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
