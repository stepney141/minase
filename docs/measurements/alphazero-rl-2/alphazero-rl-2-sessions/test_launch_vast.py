#!/usr/bin/env python3
"""2回目の強化学習の指示書に従い、生成前の照合、費用の記録、および削除条件を確かめる。

作業ツリーの根でpython3 docs/measurements/alphazero-rl-2/alphazero-rl-2-sessions/test_launch_vast.pyを実行する。
外部コマンド、vast.aiの操作、および時刻を置き換え、実際の機械は操作しない。
"""

import copy
from datetime import datetime, timezone
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock
import urllib.request


sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("launch-vast.py").resolve()
spec = importlib.util.spec_from_file_location("launch_vast", SCRIPT)
launch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launch)
VastError = launch.common.VastError


class Clock:
    def __init__(self):
        self.now = 1_800_000_000.25
        self.elapsed = 0
        self.sleeps = []

    def time(self):
        return self.now

    def monotonic(self):
        return self.elapsed

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.advance(seconds)

    def advance(self, seconds):
        self.now += seconds
        self.elapsed += seconds


class Runner:
    def __init__(self, clock, code, timeouts, exit_seconds):
        self.clock = clock
        self.code = code
        self.timeouts = timeouts
        self.exit_seconds = exit_seconds
        self.returncode = None

    def poll(self):
        return self.returncode

    def wait(self, timeout):
        if self.timeouts:
            self.timeouts -= 1
            self.clock.advance(timeout)
            raise subprocess.TimeoutExpired("uv", timeout)
        self.clock.advance(self.exit_seconds)
        self.returncode = self.code
        return self.returncode


class LaunchTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.root = self.directory / "NAS data"
        self.root.mkdir()
        self.salvage_dir = self.directory / "salvage data"
        self.home = self.directory / "home"
        (self.home / ".ssh").mkdir(parents=True)
        (self.home / ".ssh/id_vastai").write_text("test key")
        self.tree = SCRIPT.parents[4]
        self.commit = "abcdef0123456789" * 2 + "abcdef01"
        self.clock = Clock()
        self.created = {"id": 731, "end_date": 1_800_007_200, "actual_status": "loading",
                        "unfamiliar_pricing": {"storage": 0.17, "traffic": [4, 40]}, "extra": ["retain", {"all": True}]}
        self.running = {**self.created, "actual_status": "running"}
        self.before_destroy = {**self.running, "unfamiliar_usage": {"bytes": 123456}}
        self.instance_calls = 0
        self.instance_errors = {}
        self.setup_statuses = [b"done\n"]
        self.log_error = False
        self.rsync_error = None
        self.hash_mode = "valid"
        self.container_error = False
        self.runner_code = 0
        self.runner_timeouts = 0
        self.runner_exit_seconds = 0
        self.runner = None
        self.runner_ledger = None
        self.validation_error = None
        self.jobs = []
        self.jobs_error = False
        self.rsync_seconds = 0
        self.hash_seconds = 0
        self.events_at_rsync = None
        self.files = {"cycles/cycle-0001/train/checkpoint.pt": b"checkpoint bytes",
                      "cycles/cycle-0001/log space\nquote'\\.txt": b"log bytes"}
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        self.api = mock.create_autospec(launch.common)
        self.api.create.side_effect = self.create
        self.api.onstart_prefix.side_effect = lambda deadline: f"self-stop {deadline}\n"
        self.api.instance.side_effect = self.instance_info
        self.api.ssh_address.return_value = ("192.0.2.10", 2222)
        self.api.ssh_options.side_effect = lambda port, flag: ["-i", str(self.home / ".ssh/id_vastai"), flag, str(port)]
        self.api.wait_running.side_effect = self.running_after_record
        self.api.destroy.side_effect = self.destroy_after_record
        for target, name, value in ((launch, "common", self.api), (launch, "time", self.clock),
                                    (sys, "stdout", self.stdout), (sys, "stderr", self.stderr)):
            patch = mock.patch.object(target, name, value)
            patch.start()
            self.addCleanup(patch.stop)
        self.patch("home", launch.Path, return_value=self.home)
        self.run = self.patch("run", launch.subprocess, side_effect=self.command)
        self.popen = self.patch("Popen", launch.subprocess, side_effect=self.start_runner)
        self.fsync = self.patch("fsync", launch.os, wraps=os.fsync)
        self.runner_args = ["--cycles-dir", "experiments/run-a", "--first-cycle", "8", "--last-cycle", "10",
                            "--cycle-minutes", "80", "--threads-per-gpu", "4", "--precision", "bf16",
                            "--generation-commit", self.commit, "--trainer-commit", self.commit]
        self.runner_args.extend(["--base-cycles-dir", "cycles-base", "--branch-from", "0"])
        self.network = "experiments/run-a/cycle-0007/train/model.onnx"
        self.reference = "experiments/run-a/cycle-0007/check/pytorch.json"
        self.out = "experiments/run-a/cycle-0008/pre-generation-check-731"
        self.inputs = {self.network: b"onnx bytes", self.reference: b"pytorch bytes"}
        for name, data in self.inputs.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        self.check_summary = {"results": [{"backend": backend, "batch": batch, "exit_code": 0}
                                          for batch in (1, 7, 256) for backend in ("cpu", "tensorrt")],
                              "elapsed_seconds": 400}
        self.check_files = {self.out + "/summary.json": json.dumps(self.check_summary).encode(),
                            self.out + "/cpu-b1.json": b"inference output",
                            self.out + "/log space\nquote'\\.txt": b"diagnostics",
                            self.out + ".log": b"check log\n"}
        self.check_statuses = [b"0\n"]
        self.check_receive_error = False
        self.check_hash_mode = "valid"
        self.check_upload_error = False
        self.check_upload_hash_error = False
        self.check_start_error = False
        self.check_query_error = False
        self.check_transfer_seconds = 0
        self.uploaded = []
        self.downloads = []
        self.check_starts = []
        self.args = ["--offer", "456", "--label", "multi gpu test", "--image", "ubuntu-cuda:version",
                     "--hours", "1", "--commit", self.commit, "--root", str(self.root),
                     "--salvage-dir", str(self.salvage_dir), "--", *self.runner_args]

    def patch(self, name, target, **kwargs):
        patch = mock.patch.object(target, name, **kwargs)
        value = patch.start()
        self.addCleanup(patch.stop)
        return value

    def records(self):
        path = self.root / "vast-instances.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def events(self):
        return [row["event"] for row in self.records()]

    def row(self, event):
        return next(row for row in self.records() if row["event"] == event)

    def create(self, offer, label, onstart, image, disk):
        self.assertEqual((offer, label, image, disk), ("456", "multi gpu test", "ubuntu-cuda:version", 100))
        self.assertEqual(onstart.read_text(), "self-stop 1800003600\n")
        return 731

    def instance_info(self, instance, seconds):
        self.assertEqual(instance, 731)
        self.assertGreater(seconds, 0)
        index = self.instance_calls
        self.instance_calls += 1
        if index in self.instance_errors:
            raise self.instance_errors[index]
        if index == 0:
            return copy.deepcopy(self.created)
        if index == 1 or (self.runner is not None and self.runner.returncode is None):
            return copy.deepcopy(self.running)
        return copy.deepcopy(self.before_destroy)

    def running_after_record(self, instance, seconds):
        self.assertEqual((instance, seconds), (731, 900))
        self.assertEqual(self.events(), ["created"])
        self.assertEqual(self.fsync.call_count, 1)

    def destroy_after_record(self, instance):
        self.assertEqual(instance, 731)
        last = self.records()[-1]
        self.assertIn(last["event"], ("salvage_finished", "setup_failed", "network_check_failed"))
        self.assertEqual(last["before_destroy"], self.before_destroy)
        self.assertEqual(self.fsync.call_count, len(self.records()))

    def command(self, argv, **kwargs):
        if argv[0] == "git":
            if "rev-parse" in argv:
                return subprocess.CompletedProcess(argv, 0, str(self.tree) + "\n", "")
            self.assertIn("archive", argv)
            Path(argv[argv.index("-o") + 1]).write_bytes(b"source archive")
        elif argv[0] == "uv":
            self.assertEqual(argv[:5], ["uv", "run", "--frozen", "python", "-c"])
            self.assertEqual(kwargs["cwd"], self.tree / "tools/train/zero")
            self.assertFalse(self.api.create.called)
            if self.validation_error is not None:
                raise subprocess.CalledProcessError(1, argv, stderr=self.validation_error)
        elif argv[0] == "scp":
            self.assertEqual(argv[-2], "root@192.0.2.10:/workspace/setup.log")
            if self.log_error:
                raise subprocess.CalledProcessError(255, argv, stderr=b"connection lost")
            Path(argv[-1]).write_bytes(b"setup log\n")
        elif argv[0] == "rsync" and "--protect-args" in argv:
            self.assertIn("setup_finished", self.events())
            self.assertIsNone(self.runner)
            if "--relative" in argv:
                self.assertEqual(argv[-3:-1], [f"root@192.0.2.10:/workspace/az/./{self.out}",
                                             f"root@192.0.2.10:/workspace/az/./{self.out}.log"])
                self.assertEqual(argv[-1], str(self.root) + "/")
                self.assertIn(f"--exclude=/{self.out}/trt-cache/", argv)
                self.assertEqual(kwargs["timeout"], 120 + sum(map(len, self.check_files.values())) / 1_000_000)
                self.downloads.append(argv)
                self.clock.advance(self.check_transfer_seconds)
                if self.check_receive_error:
                    raise subprocess.CalledProcessError(23, argv)
                for name, data in self.check_files.items():
                    path = self.root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(data)
            else:
                name = Path(argv[-2]).relative_to(self.root).as_posix()
                self.assertEqual(argv[-1], f"root@192.0.2.10:/workspace/az/{name}")
                self.assertEqual(kwargs["timeout"], min(120 + len(self.inputs[name]) / 1_000_000,
                                                       self.row("deadline")["deadline"] - self.clock.time(), 1800))
                if self.check_upload_error:
                    raise subprocess.CalledProcessError(23, argv)
                self.uploaded.append(name)
        elif argv[0] == "rsync":
            self.events_at_rsync = self.events()
            self.clock.advance(self.rsync_seconds)
            self.assertIn(f"--compare-dest={self.root}/", argv)
            self.assertIn(f"--exclude=/{self.out}/trt-cache/", argv)
            self.assertEqual(kwargs["timeout"], 3600)
            if self.rsync_error is not None:
                raise subprocess.CalledProcessError(self.rsync_error, argv, stderr=b"transfer failed")
            destination = Path(argv[-1])
            for name, data in self.files.items():
                path = destination / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
        elif argv[0] == "ssh":
            command = argv[-1]
            if "pre-generation-check" in command:
                if "setsid nohup bash" in command:
                    self.assertEqual(self.events()[-1], "setup_finished")
                    self.check_starts.append(command)
                    if self.check_start_error:
                        raise subprocess.CalledProcessError(255, argv)
                    return subprocess.CompletedProcess(argv, 0, b"986\n", b"")
                if "elif kill -0" in command:
                    status = self.check_statuses[0]
                    if len(self.check_statuses) > 1:
                        self.check_statuses.pop(0)
                    return subprocess.CompletedProcess(argv, 0, status, b"")
                if command.startswith("python3 -c"):
                    self.assertEqual(shlex.split(command)[-1], self.out)
                    if self.check_query_error:
                        raise subprocess.CalledProcessError(255, argv)
                    sizes = {name: len(data) for name, data in self.check_files.items()}
                    return subprocess.CompletedProcess(argv, 0, json.dumps(sizes).encode(), b"")
            if command.startswith("mkdir -p --"):
                return subprocess.CompletedProcess(argv, 0, b"", b"")
            if "sha256sum" in command and self.runner is None:
                names = [os.fsdecode(name) for name in kwargs["input"].split(b"\0")[:-1]]
                sending = set(names).issubset(self.inputs)
                files = self.inputs if sending else self.check_files
                size = sum(len(files[name]) for name in names)
                self.assertEqual(kwargs["timeout"], min(120 + size / 1_000_000,
                                                       self.row("deadline")["deadline"] - self.clock.time(), 1800)
                                 if sending else 120 + size / 1_000_000)
                records = [hashlib.sha256(files[name]).hexdigest().encode() + b"  " + os.fsencode(name) + b"\0"
                           for name in names]
                mode = ("mismatch" if self.check_upload_hash_error else "valid") if sending else self.check_hash_mode
                if mode == "mismatch":
                    records[0] = b"0" * 64 + records[0][64:]
                elif mode == "missing":
                    records.pop()
                elif mode == "duplicate":
                    records.append(records[0])
                elif mode == "truncated":
                    records[-1] = records[-1][:-1]
                elif mode == "invalid":
                    records[0] = b"invalid\0"
                return subprocess.CompletedProcess(argv, 0, b"".join(records), b"")
            if command.startswith("setsid nohup bash"):
                self.assertLessEqual(kwargs["timeout"], 60)
                return subprocess.CompletedProcess(argv, 0, b"987\n", b"")
            if "grep -Fxq setup-done" in command:
                self.assertIn("kill -0 987", command)
                self.assertLessEqual(kwargs["timeout"], 60)
                status = self.setup_statuses[0]
                if len(self.setup_statuses) > 1:
                    self.setup_statuses.pop(0)
                return subprocess.CompletedProcess(argv, 0, status, b"")
            if "sha256sum" in command:
                self.clock.advance(self.hash_seconds)
                names = kwargs["input"].split(b"\0")
                self.assertEqual(names.pop(), b"")
                self.assertEqual(set(map(os.fsdecode, names)), set(self.files))
                self.assertIn("xargs -0", command)
                self.assertIn("sha256sum --zero --", command)
                records = [hashlib.sha256(self.files[os.fsdecode(name)]).hexdigest().encode() + b"  " + name + b"\0"
                           for name in names]
                if self.hash_mode == "mismatch":
                    records[0] = b"0" * 64 + records[0][64:]
                elif self.hash_mode == "missing":
                    records.pop()
                elif self.hash_mode == "duplicate":
                    records.append(records[0])
                elif self.hash_mode == "truncated":
                    records[-1] = records[-1][:-1]
                elif self.hash_mode == "invalid":
                    records[0] = b"invalid\0"
                return subprocess.CompletedProcess(argv, 0, b"".join(records), b"")
            if command.startswith("python3 -c"):
                if shlex.split(command)[-1] == "/workspace/az":
                    if self.jobs_error:
                        raise subprocess.CalledProcessError(255, argv, stderr=b"connection lost")
                    return subprocess.CompletedProcess(argv, 0, json.dumps(self.jobs).encode(), b"")
                self.assertIn("/proc/1/environ", command)
                self.assertIn("CONTAINER_API_KEY", command)
                self.assertEqual(shlex.split(command)[-1], "731")
                if self.container_error:
                    raise subprocess.CalledProcessError(255, argv, stderr=b"ssh failed")
                return subprocess.CompletedProcess(argv, 0, b"200\n", b"")
            self.fail(f"予期しないSSH: {command}")
        else:
            self.fail(f"予期しない外部コマンド: {argv}")
        return subprocess.CompletedProcess(argv, 0, b"", b"")

    def start_runner(self, argv, **kwargs):
        self.assertEqual(self.events()[-1], "network_check_finished")
        self.assertEqual(self.row("network_check_finished")["exit_code"], 0)
        self.assertEqual(self.row("network_check_finished")["summary"], self.check_summary)
        if self.runner_ledger is not None:
            ledger = self.root / "experiments/run-a.jsonl"
            ledger.parent.mkdir(parents=True, exist_ok=True)
            ledger.write_text(self.runner_ledger)
        self.runner = Runner(self.clock, self.runner_code, self.runner_timeouts, self.runner_exit_seconds)
        return self.runner

    def test_normal_lifecycle_preserves_metadata_arguments_and_cycles(self):
        """指示書1: 0で終わり、全情報を耐久化してから削除する。"""
        self.runner_ledger = "\n".join(json.dumps(row) for row in [
            {"cycle": 7, "machine": {"kind": "colab"}},
            {"cycle": 8, "machine": {"vast_instance": 731}},
            {"cycle": 9, "machine": {"vast_instance": 732}},
            {"cycle": 10, "machine": {"vast_instance": 731}},
        ]) + "\n"
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.events(), ["created", "running", "deadline", "setup_finished", "network_check_finished", "runner_exited",
                                         "salvage_finished", "destroyed"])
        self.assertEqual(self.row("created")["instance_info"], self.created)
        self.assertEqual(self.row("created")["self_stop"], 1_800_003_600)
        self.assertEqual(self.row("salvage_finished")["before_destroy"], self.before_destroy)
        self.assertEqual(self.row("runner_exited")["cycles"], [8, 10])
        for row in self.records():
            self.assertEqual(row["instance"], 731)
            self.assertEqual(datetime.fromisoformat(row["at"]).utcoffset().total_seconds(), 0)
        self.assertEqual(self.fsync.call_count, len(self.records()))
        command = self.popen.call_args.args[0]
        deadline = datetime.fromtimestamp(1_800_003_600, timezone.utc).isoformat()
        self.assertEqual(command, ["uv", "run", "--frozen", "python", "-m", "minase_zero", "run-cycles",
                                   "--machine", "vast", "--vast-instance", "731", "--deadline", deadline,
                                   "--root", str(self.root), *self.runner_args])
        self.assertEqual(self.popen.call_args.kwargs["cwd"], self.tree / "tools/train/zero")
        self.api.wait_ssh.assert_called_once_with(731, "192.0.2.10", 2222)
        self.api.run_timed.assert_called_once_with(
            ["scp", "-r", "-i", str(self.home / ".ssh/id_vastai"), "-P", "2222",
             str(self.root / f"src/minase-{self.commit}.tar"), str(SCRIPT.with_name("vast-setup.sh")),
             "root@[192.0.2.10]:/workspace/"], 600)
        self.assertEqual((self.salvage_dir / "731/setup.log").read_bytes(), b"setup log\n")
        self.api.destroy.assert_called_once_with(731)
        self.api.stop.assert_not_called()

    def test_failed_setup_is_deleted_without_starting_runner(self):
        """指示書2: 導入プロセスが消えたらログを受け取り、削除する。"""
        self.setup_statuses = [b"failed\n"]
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events(), ["created", "running", "deadline", "setup_failed", "destroyed"])
        self.assertTrue((self.salvage_dir / "731/setup.log").is_file())
        self.popen.assert_not_called()
        self.api.destroy.assert_called_once_with(731)

    def test_failed_setup_log_download_does_not_prevent_deletion(self):
        self.setup_statuses = [b"failed\n"]
        self.log_error = True
        self.assertEqual(launch.main(self.args), 1)
        self.assertIn("setup_log_error", self.row("setup_failed"))
        self.api.destroy.assert_called_once_with(731)
        self.popen.assert_not_called()

    def test_setup_polls_every_twenty_seconds(self):
        self.setup_statuses = [b"running\n", b"running\n", b"done\n"]
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.clock.sleeps, [20, 20])

    def test_setup_is_limited_to_sixty_minutes(self):
        self.setup_statuses = [b"running\n"]
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.clock.elapsed, 3600)
        self.api.destroy.assert_called_once_with(731)
        self.popen.assert_not_called()

    def test_runner_failure_without_live_jobs_salvages_and_deletes(self):
        """工程が動いていなければ、異常終了後も回収と照合を終えてから削除する。"""
        self.runner_code = 1
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-3:], ["runner_exited", "salvage_finished", "destroyed"])
        self.assertEqual(self.row("runner_exited")["exit_code"], 1)
        self.api.destroy.assert_called_once_with(731)

    def test_runner_failure_with_live_jobs_keeps_instance_without_salvage(self):
        """接続断後も動いている工程と、その途中の成果物を保全する。"""
        self.runner_code = 1
        self.jobs = ["cycles/cycle-0008/generate", "cycles/cycle-0008/train"]
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-2:], ["runner_exited", "kept_running_jobs"])
        self.assertEqual(self.row("kept_running_jobs")["jobs"], self.jobs)
        self.assertIn("機械の上で工程が動いているので機械を残した", self.stderr.getvalue())
        self.assertIsNone(self.events_at_rsync)
        self.api.destroy.assert_not_called()
        self.api.stop.assert_not_called()

    def test_failed_job_check_keeps_instance_without_salvage(self):
        self.runner_code = 1
        self.jobs_error = True
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-2:], ["runner_exited", "kept_running_jobs"])
        self.assertIsNone(self.row("kept_running_jobs")["jobs"])
        self.assertIn("error", self.row("kept_running_jobs"))
        self.assertIn("機械を残した", self.stderr.getvalue())
        self.assertIsNone(self.events_at_rsync)
        self.api.destroy.assert_not_called()

    def test_successful_runner_does_not_query_jobs(self):
        self.jobs_error = True
        self.assertEqual(launch.main(self.args), 0)
        self.api.destroy.assert_called_once_with(731)

    def test_malformed_job_response_keeps_instance(self):
        self.runner_code = 1
        self.jobs = {"unexpected": "response"}
        self.assertEqual(launch.main(self.args), 1)
        self.assertIn("error", self.row("kept_running_jobs"))
        self.assertIsNone(self.events_at_rsync)
        self.api.destroy.assert_not_called()

    def test_rsync_failure_keeps_instance(self):
        """指示書4: 転送に失敗したら機械とその上のファイルを残す。"""
        self.rsync_error = 23
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-2:], ["runner_exited", "salvage_failed"])
        self.assertEqual(self.row("salvage_failed")["operation"], "rsync")
        self.assertFalse(self.row("salvage_failed")["files_complete"])
        self.assertIn("受け取りに失敗したので機械を残した", self.stderr.getvalue())
        self.api.destroy.assert_not_called()
        self.api.stop.assert_not_called()

    def test_checksum_mismatch_keeps_instance_and_names_file(self):
        self.hash_mode = "mismatch"
        self.assertEqual(launch.main(self.args), 1)
        row = self.row("salvage_failed")
        self.assertEqual(row["operation"], "sha256")
        self.assertEqual(row["unreceived_files"], [sorted(self.files)[0]])
        self.api.destroy.assert_not_called()

    def test_incomplete_checksum_response_keeps_instance(self):
        self.hash_mode = "missing"
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(set(self.row("salvage_failed")["unreceived_files"]), set(self.files))
        self.api.destroy.assert_not_called()

    def test_duplicate_checksum_response_keeps_instance(self):
        self.hash_mode = "duplicate"
        self.assertEqual(launch.main(self.args), 1)
        self.api.destroy.assert_not_called()

    def test_truncated_checksum_response_keeps_instance(self):
        self.hash_mode = "truncated"
        self.assertEqual(launch.main(self.args), 1)
        self.api.destroy.assert_not_called()

    def test_malformed_checksum_response_keeps_instance(self):
        self.hash_mode = "invalid"
        self.assertEqual(launch.main(self.args), 1)
        self.api.destroy.assert_not_called()

    def test_salvage_connection_loss_without_live_jobs_keeps_instance(self):
        """指示書5: 接続断だけを理由に成果物を破棄しない。"""
        self.runner_code = 1
        self.rsync_error = 255
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-2:], ["runner_exited", "salvage_failed"])
        self.api.destroy.assert_not_called()

    def test_deadline_requests_stop_only_once_and_does_not_kill_runner(self):
        """指示書6: 期限を越えた機械には一度だけ停止を要求する。"""
        self.running["end_date"] = self.clock.time() + 30
        self.runner_timeouts = 3
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_called_once_with(731)
        self.assertEqual(self.events(), ["created", "running", "deadline", "setup_finished", "network_check_finished", "stop_requested",
                                         "runner_exited", "salvage_finished", "destroyed"])
        self.assertTrue(self.row("stop_requested")["success"])
        self.assertEqual(self.clock.elapsed, 180)

    def test_deadline_during_setup_and_later_phases_is_checked_only_once(self):
        self.running["end_date"] = self.clock.time() + 20
        self.setup_statuses = [b"running\n", b"running\n", b"done\n"]
        self.runner_timeouts = 2
        self.rsync_seconds = 60
        with mock.patch.object(launch, "stop_at_deadline", wraps=launch.stop_at_deadline) as stop:
            self.assertEqual(launch.main(self.args), 1)
        self.popen.assert_not_called()
        self.api.destroy.assert_called_once_with(731)
        stop.assert_called_once()
        self.api.stop.assert_called_once_with(731)
        self.assertLess(self.events().index("stop_requested"), self.events().index("setup_finished"))

    def test_setup_timeout_checks_deadline_before_deletion(self):
        self.setup_statuses = [b"running\n"]
        self.assertEqual(launch.main(self.args), 1)
        self.api.stop.assert_called_once_with(731)
        self.assertLess(self.events().index("stop_requested"), self.events().index("setup_failed"))

    def test_deadline_before_salvage_requests_stop(self):
        self.running["end_date"] = self.clock.time() + 30
        self.runner_exit_seconds = 30
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_called_once_with(731)
        self.assertIn("stop_requested", self.events_at_rsync)

    def test_deadline_during_salvage_requests_stop_before_deletion(self):
        self.running["end_date"] = self.clock.time() + 30
        self.hash_seconds = 30
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_called_once_with(731)
        self.assertNotIn("stop_requested", self.events_at_rsync)
        self.assertLess(self.events().index("stop_requested"), self.events().index("salvage_finished"))

    def test_deadline_during_failed_salvage_still_requests_stop(self):
        self.running["end_date"] = self.clock.time() + 30
        self.rsync_seconds = 30
        self.rsync_error = 255
        self.assertEqual(launch.main(self.args), 1)
        self.api.stop.assert_called_once_with(731)
        self.api.destroy.assert_not_called()

    def test_stop_failure_uses_container_credentials_once(self):
        self.running["end_date"] = self.clock.time() + 30
        self.runner_timeouts = 3
        self.api.stop.side_effect = VastError("authentication expired")
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_called_once_with(731)
        self.assertEqual(self.events().count("container_stop_requested"), 1)
        self.assertFalse(self.row("stop_requested")["success"])
        self.assertTrue(self.row("container_stop_requested")["success"])

    def test_deadline_query_failure_uses_container_credentials(self):
        self.running["end_date"] = self.clock.time() + 30
        self.runner_timeouts = 2
        self.instance_errors[2] = VastError("authentication expired")
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_not_called()
        self.assertEqual(self.events().count("container_stop_requested"), 1)

    def test_already_stopped_instance_receives_no_deadline_request(self):
        self.running["end_date"] = self.clock.time() + 30
        self.running["actual_status"] = "stopped"
        self.runner_timeouts = 3
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_not_called()
        self.assertNotIn("stop_requested", self.events())
        self.assertNotIn("container_stop_requested", self.events())

    def test_destroy_authentication_failure_stops_container(self):
        """指示書7: 削除失敗を隠さず、停止を要求して再認証を案内する。"""
        self.api.destroy.side_effect = VastError("authentication expired")
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-3:], ["salvage_finished", "destroy_failed", "container_stop_requested"])
        self.assertTrue(self.row("container_stop_requested")["success"])
        self.assertIn("vastai tfa login", self.stderr.getvalue())

    def test_container_stop_failure_is_recorded(self):
        self.api.destroy.side_effect = VastError("authentication expired")
        self.container_error = True
        self.assertEqual(launch.main(self.args), 1)
        self.assertFalse(self.row("container_stop_requested")["success"])
        self.assertNotIn("destroyed", self.events())
        self.assertIn("vastai tfa login", self.stderr.getvalue())

    def assert_rejected_before_commands(self, args):
        with self.assertRaises(SystemExit) as raised:
            launch.main(args)
        self.assertEqual(raised.exception.code, 2)
        self.run.assert_not_called()
        self.popen.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])

    def test_reserved_runner_options_are_rejected_before_commands(self):
        """指示書8: =形式と省略形も含め、機械の管理引数を上書きさせない。"""
        for option in ("--machine", "--vast-instance", "--deadline", "--session", "--cycle-cu", "--root", "--mach", "--roo"):
            with self.subTest(option=option):
                self.assert_rejected_before_commands([*self.args, option, "value"])
                self.assert_rejected_before_commands([*self.args, option + "=value"])

    def test_relative_salvage_directory_is_rejected_before_commands(self):
        args = self.args.copy()
        args[args.index("--salvage-dir") + 1] = "relative"
        self.assert_rejected_before_commands(args)

    def test_required_arguments_are_never_defaulted(self):
        for option in ("--offer", "--label", "--image", "--hours", "--commit", "--root", "--salvage-dir"):
            with self.subTest(option=option):
                index = self.args.index(option)
                self.assert_rejected_before_commands(self.args[:index] + self.args[index + 2:])

    def test_hours_and_commit_are_validated_before_commands(self):
        for option, values in (("--hours", ("0", "-1", "nan", "inf", "1e309", "invalid")),
                               ("--commit", ("abc", "z" * 40, "a" * 41))):
            for value in values:
                with self.subTest(option=option, value=value):
                    args = self.args.copy()
                    args[args.index(option) + 1] = value
                    self.assert_rejected_before_commands(args)

    def test_runner_commits_must_match_before_commands(self):
        for option in ("--generation-commit", "--trainer-commit"):
            for value in ("b" * 40, "abcdef0"):
                with self.subTest(option=option, value=value):
                    args = self.args.copy()
                    args[args.index(option) + 1] = value
                    self.assert_rejected_before_commands(args)
        self.api.create.assert_not_called()

    def test_both_runner_commits_are_required_before_commands(self):
        for option in ("--generation-commit", "--trainer-commit"):
            with self.subTest(option=option):
                index = self.args.index(option)
                self.assert_rejected_before_commands(self.args[:index] + self.args[index + 2:])
        self.api.create.assert_not_called()

    def test_invalid_runner_options_report_stderr_without_creating_instance(self):
        self.validation_error = "ValueError: branch-from must be nonnegative and less than first-cycle\n"
        self.assertEqual(launch.main(self.args), 1)
        self.assertIn(self.validation_error, self.stderr.getvalue())
        self.api.create.assert_not_called()
        self.popen.assert_not_called()
        self.assertFalse((self.root / "src").exists())

    def test_runner_validation_uses_temporary_machine_and_deadline(self):
        self.assertEqual(launch.main(self.args), 0)
        validation = next(call for call in self.run.call_args_list if call.args[0][0] == "uv")
        self.assertEqual(validation.args[0][6:], [
            "run-cycles", "--machine", "vast", "--vast-instance", "1",
            "--deadline", datetime.fromtimestamp(1_800_003_600.25, timezone.utc).isoformat(),
            "--root", str(self.root), *self.runner_args])

    def test_missing_key_is_rejected_before_commands(self):
        (self.home / ".ssh/id_vastai").unlink()
        self.assertEqual(launch.main(self.args), 1)
        self.run.assert_not_called()
        self.popen.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])

    def test_earlier_rental_end_sets_runner_deadline(self):
        """指示書9: 貸出期限が早ければ、その時刻を実行器にも渡す。"""
        self.running["end_date"] = 1_800_001_234
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.row("deadline")["deadline"], 1_800_001_234)
        command = self.popen.call_args.args[0]
        self.assertEqual(datetime.fromisoformat(command[command.index("--deadline") + 1]).timestamp(), 1_800_001_234)

    def test_missing_rental_end_is_setup_failure(self):
        del self.running["end_date"]
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events(), ["created", "running", "setup_failed", "destroyed"])
        self.popen.assert_not_called()
        self.api.run_timed.assert_not_called()
        self.api.destroy.assert_called_once_with(731)

    def test_invalid_rental_end_is_setup_failure(self):
        self.running["end_date"] = 0
        self.assertEqual(launch.main(self.args), 1)
        self.popen.assert_not_called()
        self.api.destroy.assert_called_once_with(731)

    def test_snapshot_failure_does_not_prevent_deletion_after_salvage(self):
        self.instance_errors[2] = VastError("cannot query final usage")
        self.api.destroy.side_effect = None
        self.assertEqual(launch.main(self.args), 0)
        self.assertIn("before_destroy_error", self.row("salvage_finished"))
        self.api.destroy.assert_called_once_with(731)

    def test_creation_is_recorded_even_when_initial_query_fails(self):
        self.instance_errors[0] = VastError("cannot query new instance")
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events(), ["created"])
        self.assertEqual(self.row("created")["instance"], 731)
        self.assertIn("instance_error", self.row("created"))
        self.api.destroy.assert_not_called()

    def assert_ssh_preparation_failure(self, operation, destroyed):
        """SSHの準備に失敗した機械は、失敗の詳細を記録して削除する。"""
        getattr(self.api, operation).side_effect = VastError("ssh unavailable")
        self.api.instance.side_effect = [self.created, self.before_destroy]
        if not destroyed:
            def fail_destroy(instance):
                self.destroy_after_record(instance)
                raise VastError("authentication expired")
            self.api.destroy.side_effect = fail_destroy
        self.assertEqual(launch.main(self.args), 1)
        self.api.destroy.assert_called_once_with(731)
        event = "destroyed" if destroyed else "destroy_failed"
        self.assertEqual(self.events(), ["created", "running", "setup_failed", event])
        failure = self.row("setup_failed")
        self.assertEqual(failure["operation"], operation)
        self.assertEqual(failure["error"], "ssh unavailable")
        self.assertEqual(failure["before_destroy"], self.before_destroy)
        self.popen.assert_not_called()
        self.api.run_timed.assert_not_called()
        self.api.ssh.assert_not_called()
        self.api.stop.assert_not_called()
        self.assertFalse(any(call.args[0][0] in ("ssh", "scp", "rsync") for call in self.run.call_args_list))
        if not destroyed:
            self.assertEqual(self.row("destroy_failed")["error"], "authentication expired")
            self.assertIn("削除に失敗した。`vastai tfa login` で認証し直してから削除すること",
                          self.stderr.getvalue())

    def test_wait_ssh_failure_deletes_instance(self):
        self.assert_ssh_preparation_failure("wait_ssh", True)

    def test_ssh_address_failure_deletes_instance(self):
        self.assert_ssh_preparation_failure("ssh_address", True)

    def test_wait_ssh_failure_with_destroy_failure_does_not_use_ssh(self):
        self.assert_ssh_preparation_failure("wait_ssh", False)

    def test_ssh_address_failure_with_destroy_failure_does_not_use_ssh(self):
        self.assert_ssh_preparation_failure("ssh_address", False)

    def test_existing_archive_is_reused(self):
        archive = self.root / f"src/minase-{self.commit}.tar"
        archive.parent.mkdir()
        archive.write_bytes(b"existing archive")
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(archive.read_bytes(), b"existing archive")
        self.assertFalse(any("archive" in call.args[0] for call in self.run.call_args_list))

    def test_empty_difference_needs_no_remote_checksums(self):
        self.files = {}
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.row("salvage_finished")["files"], [])
        hashes = [call for call in self.run.call_args_list if "sha256sum" in call.args[0][-1]]
        self.assertEqual(len(hashes), 3)  # 入力2件と照合の出力だけを検査し、salvageには要求しない。
        self.assertTrue(all(not set(self.files).intersection(
            os.fsdecode(name) for name in call.kwargs["input"].split(b"\0")[:-1]) for call in hashes))
        self.api.destroy.assert_called_once_with(731)

    def test_invalid_cycle_ledger_does_not_skip_final_salvage(self):
        self.runner_ledger = "null\n"
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-3:], ["runner_exited", "salvage_finished", "destroyed"])
        self.assertIn("cycles_error", self.row("runner_exited"))
        self.assertEqual(self.row("runner_exited")["exit_code"], 0)
        self.api.destroy.assert_called_once_with(731)

    def test_setup_failure_also_handles_destroy_authentication_failure(self):
        self.setup_statuses = [b"failed\n"]
        self.api.destroy.side_effect = VastError("authentication expired")
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-3:], ["setup_failed", "destroy_failed", "container_stop_requested"])
        self.popen.assert_not_called()

    def test_runner_arguments_with_equals_are_forwarded_unchanged(self):
        for option in ("--cycles-dir", "--generation-commit", "--trainer-commit"):
            index = self.args.index(option)
            self.args[index:index + 2] = [f"{option}={self.args[index + 1]}"]
        self.assertEqual(launch.main(self.args), 0)
        suffix = self.args[self.args.index("--") + 1:]
        self.assertEqual(self.popen.call_args.args[0][-len(suffix):], suffix)


    def test_check_runs_once_between_setup_and_runner_and_records_all_outputs(self):
        """指示書の正常系: 送信、照合、キャッシュを除く回収と記録を済ませて生成へ進む。"""
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.uploaded, [self.network, self.reference])
        self.assertEqual(len(self.check_starts), 1)
        self.assertEqual(len(self.downloads), 1)
        self.assertEqual(self.events().count("network_check_finished"), 1)
        self.assertLess(self.events().index("setup_finished"), self.events().index("network_check_finished"))
        self.assertLess(self.events().index("network_check_finished"), self.events().index("runner_exited"))
        row = self.row("network_check_finished")
        for key, value in (("network", self.network), ("reference", self.reference),
                           ("out", self.out), ("exit_code", 0), ("summary", self.check_summary)):
            self.assertEqual(row[key], value)
        for name, data in self.check_files.items():
            self.assertEqual((self.root / name).read_bytes(), data)
        self.assertFalse((self.root / self.out / "trt-cache").exists())
        command = self.check_starts[0]
        words = shlex.split(command)
        index = words.index("setsid")
        self.assertEqual(words[index:index + 4], ["setsid", "nohup", "bash", "-c"])
        script = words[index + 4]
        self.assertIn("cd /workspace/src && source tools/train/zero/env.sh && export XDG_CACHE_HOME=/workspace/cache", script)
        self.assertIn("python3 -I docs/measurements/alphazero-rl-2/alphazero-rl-2-routine-scripts/check_outputs.py", script)
        self.assertIn(f"--net /workspace/az/{self.network}", script)
        self.assertIn(f"--reference /workspace/az/{self.reference}", script)
        self.assertIn(f"--out /workspace/az/{self.out}", script)
        self.assertIn(f"> /workspace/az/{self.out}.log 2>&1", command)
        self.popen.assert_called_once()

    def assert_network_failure_deletes(self):
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-3:], ["network_check_finished", "network_check_failed", "destroyed"])
        self.assertEqual(self.row("network_check_failed")["before_destroy"], self.before_destroy)
        self.api.destroy.assert_called_once_with(731)
        self.popen.assert_not_called()
        self.assertIsNone(self.events_at_rsync)

    def test_failed_check_receives_outputs_before_deletion(self):
        self.check_statuses = [b"1\n"]
        self.check_summary["results"][0]["exit_code"] = 1
        self.check_files[self.out + "/summary.json"] = json.dumps(self.check_summary).encode()
        self.assert_network_failure_deletes()
        self.assertEqual(self.row("network_check_finished")["exit_code"], 1)
        self.assertEqual(self.row("network_check_finished")["summary"], self.check_summary)
        for name, data in self.check_files.items():
            self.assertEqual((self.root / name).read_bytes(), data)
        self.assertEqual(len(self.downloads), 1)

    def test_failed_check_download_deletes_instance(self):
        self.check_receive_error = True
        self.assert_network_failure_deletes()
        self.assertIn("receive_error", self.row("network_check_failed"))

    def test_check_checksum_mismatch_deletes_instance(self):
        self.check_hash_mode = "mismatch"
        self.assert_network_failure_deletes()
        self.assertIn("SHA-256", self.row("network_check_failed")["receive_error"])

    def test_check_incomplete_checksums_delete_instance(self):
        self.check_hash_mode = "missing"
        self.assert_network_failure_deletes()

    def test_check_duplicate_checksums_delete_instance(self):
        self.check_hash_mode = "duplicate"
        self.assert_network_failure_deletes()

    def test_check_truncated_checksums_delete_instance(self):
        self.check_hash_mode = "truncated"
        self.assert_network_failure_deletes()

    def test_check_malformed_checksums_delete_instance(self):
        self.check_hash_mode = "invalid"
        self.assert_network_failure_deletes()

    def test_check_upload_failure_deletes_without_starting_check(self):
        self.check_upload_error = True
        self.assert_network_failure_deletes()
        self.assertEqual(self.check_starts, [])
        self.assertEqual(len(self.downloads), 1)

    def test_check_upload_checksum_mismatch_deletes_without_starting_check(self):
        self.check_upload_hash_error = True
        self.assert_network_failure_deletes()
        self.assertEqual(self.check_starts, [])

    def test_check_start_failure_still_attempts_recovery_and_deletion(self):
        self.check_start_error = True
        self.assert_network_failure_deletes()
        self.assertEqual(len(self.downloads), 1)

    def test_check_size_query_failure_deletes_instance(self):
        self.check_query_error = True
        self.assert_network_failure_deletes()
        self.assertEqual(self.downloads, [])

    def test_check_missing_summary_deletes_instance_after_receiving_logs(self):
        del self.check_files[self.out + "/summary.json"]
        self.assert_network_failure_deletes()
        self.assertTrue((self.root / (self.out + ".log")).is_file())

    def test_check_nonfinite_summary_deletes_instance(self):
        self.check_files[self.out + "/summary.json"] = b'{"elapsed_seconds": NaN}'
        self.assert_network_failure_deletes()

    def test_check_missing_exit_marker_deletes_instance(self):
        self.check_statuses = [b"failed\n"]
        self.assert_network_failure_deletes()
        self.assertIsNone(self.row("network_check_finished")["exit_code"])

    def test_check_polls_every_twenty_seconds(self):
        self.check_statuses = [b"running\n", b"running\n", b"0\n"]
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.clock.sleeps, [20, 20])

    def test_check_is_limited_to_thirty_minutes(self):
        self.check_statuses = [b"running\n"]
        self.assert_network_failure_deletes()
        self.assertEqual(self.clock.elapsed, 1800)
        self.assertEqual(len(self.downloads), 1)
        self.assertIn("30分", self.row("network_check_finished")["error"])
        self.api.stop.assert_not_called()

    def test_deadline_during_check_requests_stop_exactly_once(self):
        self.running["end_date"] = self.clock.time() + 30
        self.check_statuses = [b"running\n"]
        self.assert_network_failure_deletes()
        self.api.stop.assert_called_once_with(731)
        self.assertEqual(self.events().count("stop_requested"), 1)
        self.assertLess(self.events().index("stop_requested"), self.events().index("network_check_failed"))
        self.assertEqual(self.clock.elapsed, 30)
        self.assertEqual(len(self.downloads), 1)

    def test_deadline_during_check_download_prevents_runner(self):
        self.running["end_date"] = self.clock.time() + 30
        self.check_transfer_seconds = 30
        self.assert_network_failure_deletes()
        self.api.stop.assert_called_once_with(731)

    def test_check_download_crossing_thirty_minutes_prevents_runner(self):
        self.check_statuses = [b"running\n"] * 89 + [b"0\n"]
        self.check_transfer_seconds = 21
        self.assert_network_failure_deletes()
        self.assertEqual(self.clock.elapsed, 1801)
        self.assertIn("30分", self.row("network_check_finished")["receive_error"])

    def test_missing_network_fails_before_any_external_command(self):
        (self.root / self.network).unlink()
        self.assertEqual(launch.main(self.args), 1)
        self.run.assert_not_called()
        self.popen.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])

    def assert_ledger_rejected_before_commands(self):
        self.assertEqual(launch.main(self.args), 1)
        self.run.assert_not_called()
        self.popen.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])

    def test_recorded_first_cycle_is_rejected_before_commands(self):
        ledger = self.root / "experiments/run-a.jsonl"
        ledger.write_text('\n'.join(json.dumps(row) for row in [
            {"cycle": 8, "machine": {"kind": "colab"}},
            {"cycle": 9, "machine": {"vast_instance": 732}},
        ]) + '\n')
        self.assert_ledger_rejected_before_commands()
        self.assertIn("--first-cycle 8", self.stderr.getvalue())
        self.assertIn("--first-cycle 10", self.stderr.getvalue())
        self.assertIn(str(ledger), self.stderr.getvalue())

    def test_recorded_first_cycle_in_mainline_cycles_jsonl_is_rejected(self):
        self.args[self.args.index("--cycles-dir") + 1] = "cycles"
        for name in ("cycles/cycle-0007/train/model.onnx", "cycles/cycle-0007/check/pytorch.json"):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"input")
        ledger = self.root / "cycles.jsonl"
        ledger.write_text(json.dumps({"cycle": 8, "machine": {"vast_instance": 732}}) + '\n')
        self.assert_ledger_rejected_before_commands()
        self.assertIn(str(ledger), self.stderr.getvalue())
        self.assertIn("--first-cycle 9", self.stderr.getvalue())

    def test_ledger_through_previous_cycle_allows_launch(self):
        ledger = self.root / "experiments/run-a.jsonl"
        ledger.write_text('\n'.join(json.dumps({"cycle": cycle, "machine": {"kind": "colab"}})
                                    for cycle in range(1, 8)) + '\n')
        self.assertEqual(launch.main(self.args), 0)
        self.api.create.assert_called_once()
        self.popen.assert_called_once()

    def test_absent_cycle_ledger_allows_launch(self):
        self.assertFalse((self.root / "experiments/run-a.jsonl").exists())
        self.assertEqual(launch.main(self.args), 0)
        self.api.create.assert_called_once()
        self.popen.assert_called_once()

    def test_invalid_cycle_ledger_rows_fail_before_commands(self):
        ledger = self.root / "experiments/run-a.jsonl"
        for line in ('not json', '', 'null', '[]', '{}', '{"cycle": 7}',
                     '{"cycle": 7, "machine": null}', '{"cycle": 7, "machine": []}',
                     '{"machine": {"kind": "colab"}}'):
            with self.subTest(line=line):
                ledger.write_text(line + '\n')
                self.assert_ledger_rejected_before_commands()

    def test_invalid_cycle_numbers_fail_before_commands(self):
        ledger = self.root / "experiments/run-a.jsonl"
        for cycle in (0, -1, True, False, 7.0, "7", None):
            with self.subTest(cycle=cycle):
                ledger.write_text(json.dumps({"cycle": cycle, "machine": {"kind": "colab"}}) + '\n')
                self.assert_ledger_rejected_before_commands()

    def test_missing_reference_fails_before_any_external_command(self):
        (self.root / self.reference).unlink()
        self.assertEqual(launch.main(self.args), 1)
        self.run.assert_not_called()
        self.popen.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])

    def test_input_outside_root_is_rejected_before_commands(self):
        outside = self.directory / "outside.onnx"
        outside.write_bytes(b"outside network")
        (self.root / self.network).unlink()
        (self.root / self.network).symlink_to(outside)
        self.assertEqual(launch.main(self.args), 1)
        self.run.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])

    def assert_cycle_paths(self, first, branch, base, network, reference, out):
        for option, value in (("--first-cycle", str(first)), ("--branch-from", str(branch)),
                              ("--cycles-dir", "cycles-rl2-az"), ("--base-cycles-dir", base)):
            self.args[self.args.index(option) + 1] = value
        self.check_files = {out + name[len(self.out):]: data for name, data in self.check_files.items()}
        self.network, self.reference, self.out = network, reference, out
        self.inputs = {network: b"onnx bytes", reference: b"pytorch bytes"}
        for name, data in self.inputs.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.uploaded, [network, reference])
        row = self.row("network_check_finished")
        self.assertEqual((row["network"], row["reference"], row["out"]), (network, reference, out))

    def test_first_cycle_one_reads_initial_model_and_reference(self):
        self.assert_cycle_paths(1, 0, "cycles-rl2-az", "cycles-rl2-az/cycle-0000/model.onnx",
                                "cycles-rl2-az/cycle-0000/check/pytorch.json",
                                "cycles-rl2-az/cycle-0001/pre-generation-check-731")

    def test_first_cycle_four_reads_previous_trained_model(self):
        self.assert_cycle_paths(4, 0, "cycles-base", "cycles-rl2-az/cycle-0003/train/model.onnx",
                                "cycles-rl2-az/cycle-0003/check/pytorch.json",
                                "cycles-rl2-az/cycle-0004/pre-generation-check-731")

    def test_branch_reads_origin_but_writes_check_to_new_series(self):
        self.assert_cycle_paths(4, 3, "cycles-origin", "cycles-origin/cycle-0003/train/model.onnx",
                                "cycles-origin/cycle-0003/check/pytorch.json",
                                "cycles-rl2-az/cycle-0004/pre-generation-check-731")

    def test_check_options_with_equals_select_the_same_inputs(self):
        for option in ("--first-cycle", "--branch-from", "--base-cycles-dir"):
            index = self.args.index(option)
            self.args[index:index + 2] = [f"{option}={self.args[index + 1]}"]
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.uploaded, [self.network, self.reference])

    def test_large_input_and_output_use_size_based_time_limits(self):
        self.inputs[self.network] = b"x" * 2_000_000
        (self.root / self.network).write_bytes(self.inputs[self.network])
        self.check_files[self.out + "/cpu-b1.json"] = b"y" * 3_000_000
        self.assertEqual(launch.main(self.args), 0)
        transfers = [call for call in self.run.call_args_list
                     if call.args[0][0] == "rsync" and "--protect-args" in call.args[0]]
        self.assertEqual(transfers[0].kwargs["timeout"], 122)
        self.assertGreater(transfers[2].kwargs["timeout"], 123)


class LocalCheckTests(unittest.TestCase):
    '''通信の境界だけをローカルに置き換え、bash、rsync、およびsha256sumを実行する。'''

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        self.root = self.directory / "NAS data"
        self.root.mkdir()
        self.workspace = self.directory / "workspace"
        (self.workspace / "az").mkdir(parents=True)
        self.out = "cycles-rl2-az/cycle-0001/pre-generation-check-731"
        self.options = SimpleNamespace(
            root=self.root, cycles_dir=Path("cycles-rl2-az"), first_cycle=1,
            network=Path("cycles-rl2-az/cycle-0000/model.onnx"),
            reference=Path("cycles-rl2-az/cycle-0000/check/pytorch.json"))
        for relative in (self.options.network, self.options.reference):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"input bytes")
        env = self.workspace / "src/tools/train/zero/env.sh"
        env.parent.mkdir(parents=True)
        env.write_text('[[ -n "${BASH_SOURCE[0]}" ]] || return 1\nexport CHECK_ENV_LOADED=yes\n')
        self.script = (self.workspace / "src/docs/measurements/alphazero-rl-2"
                       / "alphazero-rl-2-routine-scripts/check_outputs.py")
        self.script.parent.mkdir(parents=True)
        self.remote = mock.Mock(address="local")
        self.remote.shell.side_effect = self.shell
        self.remote.rsync.side_effect = self.rsync
        self.journal = launch.Journal(self.root, 731)

    def shell(self, command, timeout, data=None):
        command = command.replace("/workspace/", str(self.workspace) + "/")
        return subprocess.run(["bash", "-c", command], input=data, timeout=timeout,
                              check=True, capture_output=True).stdout

    def rsync(self, sources, destination, timeout, *options):
        def local(path):
            return str(path).replace("local:/workspace/", str(self.workspace) + "/")
        subprocess.run(["rsync", "-a", *options, "--", *map(local, sources), local(destination)],
                       timeout=timeout, check=True, capture_output=True)

    def run_check(self, code):
        # GPUの出力計算だけを小さいプログラムに置き換える。
        self.script.write_text('''import argparse, json, os, pathlib, sys
p = argparse.ArgumentParser()
for name in ('net', 'reference', 'out'):
    p.add_argument('--' + name, type=pathlib.Path, required=True)
a = p.parse_args()
assert a.net.read_bytes() == a.reference.read_bytes() == b'input bytes'
assert os.environ['CHECK_ENV_LOADED'] == 'yes'
assert os.environ['XDG_CACHE_HOME'].endswith('/workspace/cache')
assert not any(a.out.iterdir())
(a.out / 'summary.json').write_text(json.dumps({'results': [], 'elapsed_seconds': 1}))
(a.out / 'trt-cache').mkdir()
(a.out / 'trt-cache/engine').write_bytes(b'cache must stay remote')
(a.out / "name with space\\nquote'\\\\.json").write_bytes(b'inference')
print('stdout from check', flush=True)
print('stderr from check', file=sys.stderr, flush=True)
sys.exit(''' + str(code) + ')\n')
        sleep = time.sleep
        with (mock.patch.object(launch.time, "sleep", side_effect=lambda seconds: sleep(0.01)),
              mock.patch.object(launch.common, "instance", return_value={"id": 731}),
              mock.patch.object(launch.common, "destroy") as destroy):
            success = launch.network_check(self.remote, self.options, self.journal,
                                           time.time() + 60, mock.Mock())
        self.assertEqual(success, code == 0)
        if code:
            destroy.assert_called_once_with(731)
        else:
            destroy.assert_not_called()
        rows = [json.loads(line) for line in self.journal.path.read_text().splitlines()]
        self.assertEqual(rows[0]["event"], "network_check_finished")
        self.assertEqual(rows[0]["exit_code"], code)
        self.assertEqual(rows[0]["summary"], {"results": [], "elapsed_seconds": 1})
        log = (self.root / (self.out + ".log")).read_text()
        self.assertIn("stdout from check", log)
        self.assertIn("stderr from check", log)
        self.assertEqual((self.root / self.out / "name with space\nquote'\\.json").read_bytes(), b"inference")
        self.assertFalse((self.root / self.out / "trt-cache").exists())
        self.assertTrue((self.workspace / "az" / self.out / "trt-cache/engine").is_file())
        # 最後の回収でも今回のキャッシュを転送しない。
        salvage = self.directory / "salvage"
        with (mock.patch.object(launch.common, "instance", return_value={"id": 731}),
              mock.patch.object(launch.common, "destroy")):
            self.assertTrue(launch.salvage(self.remote, self.root, salvage, self.journal, mock.Mock(), self.out))
        self.assertFalse((salvage / self.out / "trt-cache").exists())

    def test_real_shell_and_transfers_preserve_outputs_without_cache(self):
        self.run_check(0)

    def test_real_nonzero_exit_recovers_outputs_and_deletes(self):
        self.run_check(1)

    def test_check_start_returns_before_background_job_finishes(self):
        """追加指示: 4秒の背景処理を残したまま、起動の応答は1秒未満で返る。"""
        out = str(self.directory / "check output")
        command = launch.network_check_start("exec sleep 4", out)
        pid = None
        try:
            started = time.monotonic()
            result = subprocess.run(["bash", "-c", command], capture_output=True,
                                    check=True, timeout=6)
            elapsed = time.monotonic() - started
            pid = int(result.stdout.strip())
            self.assertGreater(pid, 0)
            self.assertLess(elapsed, 1)
            os.kill(pid, 0)
            state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
            self.assertNotIn(state, ("Z", "X"))
        finally:
            if pid is not None:
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass

    def test_check_start_mkdir_failure_does_not_launch_background_job(self):
        """追加指示: 出力先を作れなければ、背景処理を起動せずに失敗する。"""
        parent = self.directory / "not a directory"
        parent.write_text("occupied")
        marker = self.directory / "unexpected start"
        command = launch.network_check_start(shlex.join(["touch", "--", str(marker)]),
                                             str(parent / "out"))
        result = subprocess.run(["bash", "-c", command], capture_output=True, timeout=6)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertFalse(marker.exists())

    def test_inventory_rejects_symlinks_in_output(self):
        out = self.workspace / "az" / self.out
        out.mkdir(parents=True)
        (out / "linked").symlink_to(self.root)
        command = shlex.join(["python3", "-c", launch.CHECK_FILES, self.out])
        with self.assertRaises(subprocess.CalledProcessError):
            self.shell(command, 120)


class RunningJobsTests(unittest.TestCase):
    """遠隔で実行する探索をローカルのPID記録で検証し、ゾンビと終了済みの工程を除く。"""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.pid = os.getpid()
        self.remote = mock.Mock()
        self.remote.shell.side_effect = self.shell

    def shell(self, command, timeout):
        python, flag, script, root = shlex.split(command)
        self.assertEqual((python, flag, root), ("python3", "-c", "/workspace/az"))
        self.assertGreater(timeout, 0)
        with (mock.patch.object(sys, "argv", ["-c", str(self.root)]),
              mock.patch.object(sys, "stdout", io.StringIO()) as output):
            exec(compile(script, "<running-jobs>", "exec"), {})
        return output.getvalue().encode()

    def marker(self, name):
        marker = self.root / name
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text(str(self.pid))
        return marker

    def test_only_unfinished_live_jobs_are_reported_with_relative_names(self):
        self.marker("cycles/cycle-0008/train.pid")
        self.marker("cycles/cycle-0009/generate.pid")
        self.marker("cycles/cycle-0008/generate.pid").with_suffix(".exit").write_text("0\n")
        self.assertEqual(launch.running_jobs(self.remote), [
            "cycles/cycle-0008/train", "cycles/cycle-0009/generate"])

    def test_zombie_is_not_running_even_when_kill_zero_succeeds(self):
        self.marker("cycles/train.pid")
        read_text = Path.read_text

        def read(path, *args, **kwargs):
            if path == Path(f"/proc/{self.pid}/stat"):
                return f"{self.pid} (name with ) parentheses) Z 1 1 1\n"
            return read_text(path, *args, **kwargs)

        with mock.patch.object(Path, "read_text", read):
            self.assertEqual(launch.running_jobs(self.remote), [])

    def test_disappeared_process_is_not_running(self):
        self.marker("cycles/train.pid")
        with mock.patch.object(os, "kill", side_effect=ProcessLookupError):
            self.assertEqual(launch.running_jobs(self.remote), [])

    def test_process_disappearing_between_kill_and_stat_is_not_running(self):
        self.marker("cycles/train.pid")
        read_text = Path.read_text

        def read(path, *args, **kwargs):
            if path == Path(f"/proc/{self.pid}/stat"):
                raise FileNotFoundError(path)
            return read_text(path, *args, **kwargs)

        with mock.patch.object(Path, "read_text", read):
            self.assertEqual(launch.running_jobs(self.remote), [])

    def test_invalid_pid_is_an_error(self):
        marker = self.marker("cycles/train.pid")
        for value in ("", "0", "-1", "not-a-pid"):
            with self.subTest(value=value):
                marker.write_text(value)
                with self.assertRaises(ValueError):
                    launch.running_jobs(self.remote)


class ContainerStopTests(unittest.TestCase):
    """指示書9の停止要求の内容を、通信せずに確認する。"""

    def test_request_uses_only_pid_one_container_credentials(self):
        environ = b"CONTAINER_ID=731\0CONTAINER_API_KEY=test-secret\0OTHER=unused\0"
        with (mock.patch.object(Path, "read_bytes", autospec=True, return_value=environ) as read,
              mock.patch.object(urllib.request, "urlopen") as urlopen,
              mock.patch.object(sys, "argv", ["-c", "731"]),
              mock.patch.object(sys, "stdout", io.StringIO()) as output):
            urlopen.return_value.__enter__.return_value.status = 200
            exec(compile(launch.CONTAINER_STOP, "<container-stop>", "exec"), {})
        read.assert_called_once_with(Path("/proc/1/environ"))
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, "https://console.vast.ai/api/v0/instances/731/")
        self.assertEqual(request.get_method(), "PUT")
        self.assertEqual(json.loads(request.data), {"state": "stopped"})
        self.assertEqual(request.get_header("Authorization"), "Bearer test-secret")
        self.assertLessEqual(urlopen.call_args.kwargs["timeout"], 60)
        self.assertEqual(output.getvalue(), "200\n")

    def test_missing_or_different_credentials_never_issue_request(self):
        for environ in (b"CONTAINER_ID=731\0", b"CONTAINER_ID=732\0CONTAINER_API_KEY=test-secret\0"):
            with (self.subTest(environ=environ),
                  mock.patch.object(Path, "read_bytes", return_value=environ),
                  mock.patch.object(urllib.request, "urlopen") as urlopen,
                  mock.patch.object(sys, "argv", ["-c", "731"]),
                  mock.patch.object(sys, "stderr", io.StringIO()) as errors):
                with self.assertRaises(SystemExit) as raised:
                    exec(compile(launch.CONTAINER_STOP, "<container-stop>", "exec"), {})
                self.assertEqual(raised.exception.code, 1)
                self.assertNotIn("test-secret", errors.getvalue())
                urlopen.assert_not_called()

    def test_network_error_does_not_disclose_container_key(self):
        environ = b"CONTAINER_ID=731\0CONTAINER_API_KEY=test-secret\0"
        with (mock.patch.object(Path, "read_bytes", return_value=environ),
              mock.patch.object(urllib.request, "urlopen", side_effect=OSError("test-secret")),
              mock.patch.object(sys, "argv", ["-c", "731"]),
              mock.patch.object(sys, "stderr", io.StringIO()) as errors):
            with self.assertRaises(SystemExit) as raised:
                exec(compile(launch.CONTAINER_STOP, "<container-stop>", "exec"), {})
            self.assertEqual(raised.exception.code, 1)
            self.assertNotIn("test-secret", errors.getvalue())


if __name__ == "__main__":
    unittest.main()
