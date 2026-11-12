#!/usr/bin/env python3
"""イメージと認証の検査、トークンの秘匿、および成果物を保全する削除条件を確かめる。

作業ツリーの根でpython3 docs/measurements/docker-environment/test_launch_vast.pyを実行する。
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
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import urllib.request


sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("launch-vast.py").resolve()
spec = importlib.util.spec_from_file_location("launch_vast", SCRIPT)
launch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launch)
VastError = launch.common.VastError
RegistryLogin = launch.common.RegistryLogin


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
        self.tree = SCRIPT.parents[3]
        self.commit = "abcdef0123456789" * 2 + "abcdef01"
        self.image = "ghcr.io/stepney141/minase-dev@sha256:" + "0123456789abcdef" * 4
        self.token = "ghp_test_registry_token_8392"
        self.token_file = self.directory / "registry-token"
        self.token_file.write_text("\n" + self.token + "\n", encoding="utf-8")
        self.token_file.chmod(0o600)
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
        self.api.RegistryLogin = RegistryLogin
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
        self.args = ["--offer", "456", "--label", "multi gpu test", "--image", self.image,
                     "--hours", "1", "--commit", self.commit, "--root", str(self.root),
                     "--salvage-dir", str(self.salvage_dir), "--registry-token-file", str(self.token_file),
                     "--", *self.runner_args]

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

    def create(self, offer, label, onstart, image, disk, login):
        self.assertEqual((offer, label, image, disk), ("456", "multi gpu test", self.image, 100))
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
        self.assertIn(last["event"], ("salvage_finished", "setup_failed"))
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
        elif argv[0] == "rsync":
            self.events_at_rsync = self.events()
            self.clock.advance(self.rsync_seconds)
            self.assertIn(f"--compare-dest={self.root}/", argv)
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
        self.runner = Runner(self.clock, self.runner_code, self.runner_timeouts, self.runner_exit_seconds)
        return self.runner

    def test_normal_lifecycle_preserves_metadata_arguments_and_cycles(self):
        """0で終わり、全情報を耐久化してから削除する。"""
        ledger = self.root / "experiments/run-a.jsonl"
        ledger.parent.mkdir()
        ledger.write_text("\n".join(json.dumps(row) for row in [
            {"cycle": 7, "machine": {"kind": "colab"}},
            {"cycle": 8, "machine": {"vast_instance": 731}},
            {"cycle": 9, "machine": {"vast_instance": 732}},
            {"cycle": 10, "machine": {"vast_instance": 731}},
        ]) + "\n")
        self.assertEqual(launch.main(self.args), 0)
        self.assertEqual(self.events(), ["created", "running", "deadline", "setup_finished", "runner_exited",
                                         "salvage_finished", "destroyed"])
        self.assertEqual(self.row("created")["instance_info"], self.created)
        self.assertEqual(self.row("created")["image"], self.image)
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

    def test_registry_login_uses_image_registry_owner_and_trimmed_token(self):
        """ログイン先と利用者名はイメージから取り、トークンの前後の空白だけを除く。"""
        self.image = "registry.example:5443/research/minase@sha256:" + "fedcba9876543210" * 4
        self.args[self.args.index("--image") + 1] = self.image
        self.token_file.write_text("\n\t " + self.token + " \n", encoding="utf-8")
        self.assertEqual(launch.main(self.args), 0)
        self.api.create.assert_called_once()
        login = self.api.create.call_args.args[5]
        self.assertIsInstance(login, RegistryLogin)
        self.assertEqual((login.registry, login.user, login.token),
                         ("registry.example:5443", "research", self.token))
        self.assertEqual(self.row("created")["image"], self.image)

    def assert_snapshot_tokens_redacted(self):
        self.created["image_login"] = {"arguments": f"-p {self.token}", "copies": [self.token, self.token]}
        self.before_destroy["image_login"] = {"arguments": f"-p {self.token}", "copies": [self.token, self.token]}
        self.api.destroy.side_effect = None
        self.assertEqual(launch.main(self.args), 0)
        for line in (self.root / "vast-instances.jsonl").read_text(encoding="utf-8").splitlines():
            self.assertNotIn(self.token, line)
        for snapshot in (self.row("created")["instance_info"],
                         self.row("salvage_finished")["before_destroy"]):
            self.assertEqual(snapshot["image_login"],
                             {"arguments": "-p <token>", "copies": ["<token>", "<token>"]})
        self.assertEqual(self.row("created")["image"], self.image)
        self.api.destroy.assert_called_once_with(731)

    def test_journal_redacts_token_in_created_and_before_destroy(self):
        """作成直後と削除直前の情報に含まれるトークンを、記録の全行から除く。"""
        self.assert_snapshot_tokens_redacted()

    def test_journal_redacts_json_escaped_token(self):
        """空白のないトークンは、JSONが引用符などをエスケープしても秘匿する。"""
        self.token = 'ghp_test_"quoted"\\秘密'
        self.token_file.write_text(self.token + "\n", encoding="utf-8")
        self.assert_snapshot_tokens_redacted()

    def test_main_redacts_token_when_launch_fails(self):
        """起動失敗の表示からトークンを除き、引数の名前空間にもトークンを入れない。"""
        message = f"launch failed: {self.token}, again: {self.token}"
        with mock.patch.object(launch, "launch", side_effect=RuntimeError(message)) as start:
            self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.stderr.getvalue(), "launch failed: <token>, again: <token>\n")
        self.assertNotIn(self.token, self.stderr.getvalue())
        options, login = start.call_args.args
        self.assertEqual(options.registry_token_file, self.token_file)
        self.assertNotIn(self.token, repr(vars(options)))
        self.assertEqual(login.token, self.token)
        self.api.create.assert_not_called()

    def test_failed_setup_is_deleted_without_starting_runner(self):
        """導入プロセスが消えたらログを受け取り、削除する。"""
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
        """転送に失敗したら機械とその上のファイルを残す。"""
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
        """接続断だけを理由に成果物を破棄しない。"""
        self.runner_code = 1
        self.rsync_error = 255
        self.assertEqual(launch.main(self.args), 1)
        self.assertEqual(self.events()[-2:], ["runner_exited", "salvage_failed"])
        self.api.destroy.assert_not_called()

    def test_deadline_requests_stop_only_once_and_does_not_kill_runner(self):
        """期限を越えた機械には一度だけ停止を要求する。"""
        self.running["end_date"] = self.clock.time() + 30
        self.runner_timeouts = 3
        self.assertEqual(launch.main(self.args), 0)
        self.api.stop.assert_called_once_with(731)
        self.assertEqual(self.events(), ["created", "running", "deadline", "setup_finished", "stop_requested",
                                         "runner_exited", "salvage_finished", "destroyed"])
        self.assertTrue(self.row("stop_requested")["success"])
        self.assertEqual(self.clock.elapsed, 180)

    def test_deadline_during_setup_and_later_phases_is_checked_only_once(self):
        self.running["end_date"] = self.clock.time() + 20
        self.setup_statuses = [b"running\n", b"running\n", b"done\n"]
        self.runner_timeouts = 2
        self.rsync_seconds = 60
        with mock.patch.object(launch, "stop_at_deadline", wraps=launch.stop_at_deadline) as stop:
            self.assertEqual(launch.main(self.args), 0)
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
        """削除失敗を隠さず、停止を要求して再認証を案内する。"""
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
        """=形式と省略形も含め、機械の管理引数を上書きさせない。"""
        for option in ("--machine", "--vast-instance", "--deadline", "--session", "--cycle-cu", "--root", "--mach", "--roo"):
            with self.subTest(option=option):
                self.assert_rejected_before_commands([*self.args, option, "value"])
                self.assert_rejected_before_commands([*self.args, option + "=value"])

    def test_relative_salvage_directory_is_rejected_before_commands(self):
        args = self.args.copy()
        args[args.index("--salvage-dir") + 1] = "relative"
        self.assert_rejected_before_commands(args)

    def test_required_arguments_are_never_defaulted(self):
        for option in ("--offer", "--label", "--image", "--hours", "--commit", "--root", "--salvage-dir",
                       "--registry-token-file"):
            with self.subTest(option=option):
                index = self.args.index(option)
                self.assert_rejected_before_commands(self.args[:index] + self.args[index + 2:])

    def test_image_requires_registry_owner_name_and_lowercase_sha256(self):
        """タグと不完全な名前を拒み、内容を固定したイメージだけを使う。"""
        digest = "0123456789abcdef" * 4
        for value in ("ghcr.io/owner/name:tag", "ghcr.io/owner/name",
                      f"ubuntu@sha256:{digest}", f"ghcr.io/name@sha256:{digest}",
                      f"ghcr.io/owner/name@sha256:{digest[:-1]}",
                      f"ghcr.io/owner/name@sha256:{digest}0",
                      f"ghcr.io/owner/name@sha256:{digest.upper()}",
                      f"ghcr.io/owner/name@sha256:{'g' * 64}",
                      f"ghcr.io/owner/name:tag@sha256:{digest}",
                      f"/owner/name@sha256:{digest}", f"ghcr.io//name@sha256:{digest}",
                      f"ghcr.io/owner/@sha256:{digest}", f"ghcr.io/owner/extra/name@sha256:{digest}",
                      f"ghcr.io/ow ner/name@sha256:{digest}",
                      f"ghcr.io/owner/name@sha256:{digest}\n"):
            with self.subTest(image=value):
                args = self.args.copy()
                args[args.index("--image") + 1] = value
                self.assert_rejected_before_commands(args)

    def assert_token_rejected_before_commands(self):
        self.assertEqual(launch.main(self.args), 1)
        self.run.assert_not_called()
        self.popen.assert_not_called()
        self.assertEqual(self.api.mock_calls, [])
        self.assertFalse((self.root / "src").exists())
        self.assertFalse((self.root / "vast-instances.jsonl").exists())
        self.assertNotIn(self.token, self.stderr.getvalue())

    def test_missing_token_file_is_rejected_before_commands(self):
        """トークンを読めなければ、実行器の検査も書庫の作成も始めない。"""
        self.token_file.unlink()
        self.assert_token_rejected_before_commands()

    def test_token_file_must_be_regular(self):
        self.token_file.unlink()
        self.token_file.mkdir(mode=0o700)
        self.assert_token_rejected_before_commands()

    def test_token_file_permissions_exclude_group_and_other(self):
        """グループとその他の読み取り、書き込み、および実行権限を拒む。"""
        for mode in (0o644, 0o640, 0o620, 0o610, 0o602, 0o601):
            with self.subTest(mode=oct(mode)):
                self.token_file.chmod(mode)
                self.assert_token_rejected_before_commands()

    def test_owner_read_only_token_file_is_accepted(self):
        """所有者の書き込み権限は、トークンを読むための必須条件ではない。"""
        self.token_file.chmod(0o400)
        self.assertEqual(launch.main(self.args), 0)

    def test_empty_token_is_rejected_before_commands(self):
        for contents in ("", " \n\t "):
            with self.subTest(contents=contents):
                self.token_file.write_text(contents, encoding="utf-8")
                self.assert_token_rejected_before_commands()

    def test_token_with_internal_whitespace_is_rejected_before_commands(self):
        for separator in (" ", "\t", "\n", "\u2003"):
            with self.subTest(separator=separator):
                self.token_file.write_text(self.token + separator + "suffix", encoding="utf-8")
                self.assert_token_rejected_before_commands()

    def test_invalid_token_encoding_is_rejected_without_disclosing_contents(self):
        self.token_file.write_bytes(self.token.encode() + b"\xff")
        self.assert_token_rejected_before_commands()

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
        """貸出期限が早ければ、その時刻を実行器にも渡す。"""
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
        self.assertFalse(any("sha256sum" in call.args[0][-1] for call in self.run.call_args_list))
        self.api.destroy.assert_called_once_with(731)

    def test_invalid_cycle_ledger_does_not_skip_final_salvage(self):
        ledger = self.root / "experiments/run-a.jsonl"
        ledger.parent.mkdir()
        ledger.write_text("null\n")
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
    """コンテナの停止要求の内容を、通信せずに確認する。"""

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
