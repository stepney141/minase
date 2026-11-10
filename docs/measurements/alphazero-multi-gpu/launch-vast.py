#!/usr/bin/env python3
"""vast.aiの機械を借りて導入し、学習の実行器を動かして成果物を回収する。

作業機で次のように起動し、--の後ろにrun-cyclesの引数を指定する。
python3 launch-vast.py --offer ID --label LABEL --image IMAGE --hours HOURS --commit COMMIT --root ROOT --salvage-dir /ABS/PATH -- --cycles-dir cycles ...
起動側の引数はすべて必須で、commitは40桁の16進数字、hoursは正の有限数とする。
実行器のmachine、vast-instance、deadline、rootはこのスクリプトが指定し、sessionとcycle-cuは指定できない。
ソースの書庫を用意して自己停止を予約し、貸出期限と自己停止時刻の早いほうを終了期限にする。
導入のログを回収した後、この作業ツリーの実行器を起動し、60秒ごとに終了期限を確認する。
期限を超えた機械には停止を1回だけ要求し、実行器が自ら終了するまで監視する。
実行器の終了コードや接続断にかかわらず、NASとの差分をsalvage-dirのインスタンス別ディレクトリへ受け取り、SHA-256を照合する。
SSHの準備または導入に失敗した機械、または最後の受け取りと照合に成功した機械を削除する。
最後の受け取りか照合に失敗した機械は残し、自己停止までに受け取り直して削除するよう通知する。
停止要求または削除が失敗した場合は、SSHの準備が済んでいればコンテナ専用の鍵を使って停止を要求する。
それ以外の失敗では機械を削除せず、起動確認に失敗した場合の削除だけはcommon.wait_runningに任せる。
各出来事はroot/vast-instances.jsonlへ追記し、1行ずつfsyncする。
料金の欄は選別せず、作成直後と削除直前のインスタンスのJSON全体を記録する。
before_destroyは削除前に確定するsalvage_finished行に入れ、SSHの準備または導入の失敗時はsetup_failed行に入れる。
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
COMMON_PATH = SCRIPT_DIR.parents[2] / "scripts/vast/common.py"
spec = importlib.util.spec_from_file_location("launch_vast_common", COMMON_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError(f"vast.aiの操作モジュールを読めません: {COMMON_PATH}")
common = importlib.util.module_from_spec(spec)
spec.loader.exec_module(common)

FAILURES = (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError)
FORBIDDEN = ("--machine", "--vast-instance", "--deadline", "--session", "--cycle-cu", "--root")

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
    cycles, _ = cycles_parser.parse_known_args(options.runner_args)
    directory = cycles.cycles_dir
    if (directory.is_absolute() or directory == Path(".") or ".." in directory.parts
            or any(character.isspace() for character in str(directory))):
        parser.error("--cycles-dirには空白を含まないroot内の相対ディレクトリが必要です")
    options.cycles_dir = directory
    # 実行器ではcwdが変わるので、相対指定も起動元の同じディレクトリを指すようにする。
    options.root = options.root.absolute()
    return options


def utc(timestamp):
    return datetime.fromtimestamp(timestamp, timezone.utc).isoformat()


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


def setup(remote, archive):
    # 最小構成のイメージにはrsyncがまだないため、導入前の転送はSCPを使う。
    common.scp(remote.host, remote.port, "/workspace/", archive, SCRIPT_DIR / "vast-setup.sh")
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


def run_cycles(tree, options, remote, journal, deadline):
    command = ["uv", "run", "--frozen", "python", "-m", "minase_zero", "run-cycles",
               "--machine", "vast", "--vast-instance", str(journal.instance),
               "--deadline", utc(deadline), "--root", str(options.root), *options.runner_args]
    process = subprocess.Popen(command, cwd=tree / "tools/train/zero")
    checked_deadline = False
    while process.poll() is None:
        if not checked_deadline and time.time() >= deadline:
            checked_deadline = True
            stop_at_deadline(remote, journal)
        try:
            return process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            pass
    return process.returncode


def completed_cycles(options, instance):
    path = options.root / f"{options.cycles_dir}.jsonl"
    if not path.exists():
        # 完了したサイクルが0件なら、実行器は台帳を作成しない。
        return []
    cycles = []
    for line in path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if not isinstance(row, dict) or not isinstance(row["machine"], dict):
            raise ValueError(f"サイクルの記録が不正です: {path}")
        machine = row["machine"]
        if "vast_instance" in machine and machine["vast_instance"] == instance:
            if type(row["cycle"]) is not int or row["cycle"] <= 0:
                raise ValueError(f"サイクルの番号が不正です: {path}")
            cycles.append(row["cycle"])
    return cycles


def salvage(remote, root, destination, journal):
    operation = "rsync"
    pending = []
    files_known = False
    try:
        destination.mkdir(parents=True, exist_ok=True)
        remote.rsync([f"{remote.address}:/workspace/az/"], str(destination) + "/", 3600,
                     f"--compare-dest={root}/")
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
    return destroy(remote, journal, "salvage_finished", files=[path.relative_to(destination).as_posix() for path in paths])


def launch(options):
    key = Path.home() / ".ssh/id_vastai"
    if not key.is_file():
        raise ValueError(f"vast.ai専用のSSH鍵がありません: {key}")
    output = subprocess.run(["git", "-C", str(SCRIPT_DIR), "rev-parse", "--show-toplevel"],
                            check=True, capture_output=True, text=True, timeout=60).stdout.strip()
    if not output:
        raise ValueError("作業ツリーの根を取得できません")
    tree = Path(output)
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
        setup_fields = {}
        setup_error = None
        try:
            setup(remote, archive)
        except FAILURES as error:
            setup_error = str(error)
        finally:
            try:
                log = options.salvage_dir / str(instance) / "setup.log"
                log.parent.mkdir(parents=True, exist_ok=True)
                subprocess.run(["scp", *common.ssh_options(port, "-P"), "--",
                                f"{remote.address}:/workspace/setup.log", str(log)],
                               check=True, capture_output=True, timeout=60)
            except FAILURES as error:
                setup_fields["setup_log_error"] = str(error)
                print(f"インスタンス {instance}: setup.logの受け取りに失敗しました: {error}", file=sys.stderr)
        if setup_error is not None:
            destroy(remote, journal, "setup_failed", error=setup_error, **setup_fields)
            return 1
        journal.write("setup_finished", **setup_fields)
        runner_fields = {}
        try:
            code = run_cycles(tree, options, remote, journal, deadline)
        except FAILURES as error:
            code = 1
            runner_fields["error"] = str(error)
        try:
            runner_fields["cycles"] = completed_cycles(options, instance)
        except FAILURES as error:
            runner_fields["cycles_error"] = str(error)
            print(f"インスタンス {instance}: サイクルの記録を読めません: {error}", file=sys.stderr)
        journal.write("runner_exited", exit_code=code, **runner_fields)
        if not salvage(remote, options.root, options.salvage_dir / str(instance) / "az", journal):
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
