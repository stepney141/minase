"""Fabricのプロセス数に依存しない勾配と、起動・バッチの境界を仕様から検証する。"""

from contextlib import nullcontext
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import torch

from minase_zero.__main__ import _parse
from minase_zero import train
from minase_zero.train import training_fabric

from tests.cli import ZERO, environment, init_arguments, require_success, run_cli, run_command, train_arguments


class FabricTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        directory = tempfile.TemporaryDirectory(prefix="minase-fabric-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.initial = cls.root / "initial"
        require_success(run_cli(init_arguments(cls.initial, 23)))

    def test_microbatch_and_replay_threads_are_required_in_their_respective_commands(self):
        args = train_arguments(self.initial / "checkpoint.pt", self.root / "arguments")
        for flag in ("--micro-batch-size", "--replay-threads"):
            missing = args.copy()
            index = missing.index(flag)
            del missing[index:index + 2]
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, flag):
                _parse(missing)
        with self.assertRaisesRegex(ValueError, "--replay-threads"):
            _parse(["train", "--resume", str(self.root / "resume"), "--device", "cpu"])
        resumed = _parse(["train", "--resume", str(self.root / "resume"), "--device", "cpu",
                          "--replay-threads", "2"])
        self.assertNotIn("micro_batch_size", vars(resumed))

    def test_only_rank_zero_computes_provenance(self):
        fixture = ZERO / "fixtures/zero-selfplay-small.mnsd"
        initial = self.initial / "checkpoint.pt"
        for rank in (0, 1):
            with self.subTest(rank=rank):
                args = vars(_parse(train_arguments(initial, self.root / f"provenance-{rank}")))
                del args["command"]
                out = args.pop("out_dir")
                fabric = SimpleNamespace(global_rank=rank, world_size=2, device=torch.device("cpu"),
                                         all_reduce=lambda tensor, reduce_op: tensor)
                # 学習と通信を起動せず、実際の教師読込と来歴の作成を検証する。
                with (patch.object(train, "training_fabric", return_value=nullcontext(fabric)),
                      patch.object(train, "_run") as run,
                      patch.object(train, "_finish") as finish,
                      patch.object(train, "_environment", wraps=train._environment) as environment,
                      patch.object(train, "_sha256", wraps=train._sha256) as hashes):
                    train.train_new(args, out)
                state = run.call_args.args[0]
                if rank == 0:
                    environment.assert_called_once()
                    self.assertCountEqual([call.args[0].resolve() for call in hashes.call_args_list],
                                          [initial.resolve(), fixture.resolve()])
                    self.assertEqual(state["provenance"]["world_size"], 2)
                    self.assertEqual(state["provenance"]["files"][0]["sha256"],
                                     hashlib.sha256(fixture.read_bytes()).hexdigest())
                    self.assertEqual(state["provenance"]["init_sha256"],
                                     hashlib.sha256(initial.read_bytes()).hexdigest())
                    finish.assert_called_once()
                else:
                    environment.assert_not_called()
                    hashes.assert_not_called()
                    self.assertNotIn("provenance", state)
                    finish.assert_not_called()

    def test_preclip_gradients_and_first_microbatch_statistics(self):
        states = {}
        for n in (1, 2):
            out = self.root / f"gradients-{n}"
            out.mkdir()
            result = run_command(
                [sys.executable, "-m", "torch.distributed.run", "--standalone", "--nproc-per-node", str(n),
                 "-m", "tests.gradient_worker", str(out)],
            )
            require_success(result)
            states[n] = torch.load(out / "rank-0.pt", weights_only=True)
            for rank in range(n):
                state = torch.load(out / f"rank-{rank}.pt", weights_only=True)
                for name in state["buffers"]:
                    with self.subTest(n=n, rank=rank, buffer=name):
                        torch.testing.assert_close(state["buffers"][name], state["expected_buffers"][name])
                        if name.endswith("num_batches_tracked"):
                            self.assertEqual(state["buffers"][name].item(), 1 if rank == 0 else 0)
                for name in state["gradients"]:
                    torch.testing.assert_close(state["gradients"][name], states[n]["gradients"][name])
        for name in states[1]["gradients"]:
            with self.subTest(parameter=name):
                torch.testing.assert_close(states[1]["gradients"][name], states[2]["gradients"][name])
        torch.testing.assert_close(states[1]["loss_sums"], states[2]["loss_sums"])

    def test_invalid_batch_conditions_fail_before_loading(self):
        for n, batch, micro, threads, message in (
            (1, 10000, 4, 1, "sample count must be at least batch_size"),
            (1, 15, 4, 1, "batch_size must be a multiple of micro_batch_size"),
            (2, 12, 4, 1, "K must be a multiple of WORLD_SIZE"),
            (1, 0, 4, 1, "batch_size must be a positive integer"),
            (1, 16, 0, 1, "micro_batch_size must be a positive integer"),
            (1, 16, 4, 0, "replay-threads must be a positive integer"),
        ):
            with self.subTest(n=n, batch=batch, micro=micro, threads=threads):
                out = self.root / f"invalid-{n}-{batch}-{micro}-{threads}"
                args = train_arguments(self.initial / "checkpoint.pt", out)
                for flag, value in (("--batch-size", batch), ("--micro-batch-size", micro),
                                    ("--replay-threads", threads)):
                    args[args.index(flag) + 1] = str(value)
                # バッチの整除性の検査は、存在しない教師リストを読むより前である。
                if batch != 10000:
                    Path(args[args.index("--data-list") + 1]).unlink()
                result = run_cli(args, n)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(message, result.stderr)
                self.assertFalse((out / "checkpoint.pt").exists())

    def test_train_outside_torchrun_and_missing_environment_variables_fail(self):
        args = train_arguments(self.initial / "checkpoint.pt", self.root / "outside")
        env = environment()
        names = ("WORLD_SIZE", "LOCAL_WORLD_SIZE", "RANK", "LOCAL_RANK")
        for name in names:
            env.pop(name, None)
        result = subprocess.run([sys.executable, "-m", "minase_zero", *args], cwd=ZERO,
                                env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 1)
        self.assertIn("train must run under torchrun", result.stderr)
        complete = dict(zip(names, ("1", "1", "0", "0"), strict=True))
        for missing in names:
            with self.subTest(missing=missing), patch.dict(os.environ, complete, clear=True):
                del os.environ[missing]
                with self.assertRaisesRegex(ValueError, "torchrun.*missing.*" + missing):
                    with training_fabric("cpu", "fp32"):
                        self.fail("Fabric must not launch")
        with patch.dict(os.environ, {**complete, "WORLD_SIZE": "2"}, clear=True):
            with self.assertRaisesRegex(ValueError, "WORLD_SIZE must equal LOCAL_WORLD_SIZE"):
                with training_fabric("cpu", "fp32"):
                    self.fail("Fabric must not launch")

    def test_old_training_checkpoint_is_rejected_explicitly(self):
        out = self.root / "old"
        out.mkdir()
        state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        state.update(optimizer={}, completed_steps=1, total_steps=2)
        torch.save(state, out / "checkpoint.pt")
        result = run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "1"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("old trainer", result.stderr)
        self.assertIn("micro_batch_size is missing", result.stderr)
