"""別セッションの子孫を、強制終了・時間切れ・中断のいずれでも残さない。"""

import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import unittest
from unittest.mock import patch

from tests.cli import kill_training, run_command


WORKER = """
import signal, subprocess, sys
child = subprocess.Popen([sys.executable, '-c', 'import signal; signal.pause()'], start_new_session=True)
print(child.pid, flush=True)
signal.pause()
"""

LAUNCHER = """
import json, signal, subprocess, sys
workers = [subprocess.Popen([sys.executable, '-c', sys.argv[1]], stdout=subprocess.PIPE,
                            text=True, start_new_session=True) for _ in range(2)]
print(json.dumps([worker.pid for worker in workers] +
                 [int(worker.stdout.readline()) for worker in workers]), flush=True)
signal.pause()
"""


class CliProcessTests(unittest.TestCase):
    def ready_pids(self, process):
        ready, _, _ = select.select([process.stdout], [], [], 10)
        self.assertTrue(ready, "worker processes did not start")
        return [process.pid, *json.loads(process.stdout.readline())]

    @staticmethod
    def stop_groups(pids):
        for pid in pids:
            try:
                os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def assert_stopped(self, pids):
        for pid in pids:
            with self.subTest(pid=pid):
                try:
                    status = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
                except (FileNotFoundError, ProcessLookupError):
                    continue
                self.assertIn(status, ("Z", "X"), f"process {pid} survived")

    def test_sigkill_stops_workers_and_grandchildren_in_separate_sessions(self):
        with subprocess.Popen([sys.executable, "-c", LAUNCHER, WORKER], stdout=subprocess.PIPE,
                              text=True, start_new_session=True) as process:
            pids = [process.pid]
            try:
                pids = self.ready_pids(process)
                self.assertEqual(len(pids), 5)
                self.assertEqual(kill_training(process), -signal.SIGKILL)
                self.assert_stopped(pids)
            finally:
                self.stop_groups(pids)
                process.wait(timeout=10)

    def test_run_command_cleans_up_workers_on_timeout_and_interruption(self):
        for error in (subprocess.TimeoutExpired("training", 180), KeyboardInterrupt()):
            pids = []

            def interrupted(process, timeout):
                pids.extend(self.ready_pids(process))
                raise error

            with self.subTest(error=type(error).__name__):
                try:
                    with patch.object(subprocess.Popen, "communicate", new=interrupted):
                        with self.assertRaises(type(error)) as caught:
                            run_command([sys.executable, "-c", LAUNCHER, WORKER])
                    self.assertIs(caught.exception, error)
                    self.assertEqual(len(pids), 5)
                    self.assert_stopped(pids)
                finally:
                    self.stop_groups(pids)
