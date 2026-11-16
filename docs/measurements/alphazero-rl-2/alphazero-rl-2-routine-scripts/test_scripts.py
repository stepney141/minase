"""指示書3の対局条件、USI入力、照合手順を実エンジンなしで検証する。

期待値は指示書3と、同書が参照する設計書・出力照合の手引きに基づく。
対局3種の全引数列、入力の欠落と不正値、照合6組の正常終了と失敗を扱う。
"""

import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


# 共有スクリプトのディレクトリにはキャッシュも書かない。
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_outputs
import run_rl2_match


class MatchTests(unittest.TestCase):
    def arguments(self, command):
        argv = [
            command, "--harness", "target/release/minase", "--name", "rl2-test",
            "--mcts-commit", "0123456789abcdef0123456789abcdef01234567",
            "--net", "nets/candidate.onnx", "--seed", "123", "--pairs", "20",
        ]
        if command == "versus":
            argv += ["--baseline-net", "nets/baseline.onnx"]
        return argv

    def test_stc_full_argv(self):
        args = run_rl2_match.parser().parse_args(self.arguments("stc"))
        self.assertEqual(run_rl2_match.build_argv(args), [
            "target/release/minase", "match", "run",
            "--run-dir", "data/matches/rl2-test", "--seed", "123",
            "--candidate", "commit:0123456789abcdef0123456789abcdef01234567",
            "--candidate-args", "--search mcts",
            "--candidate-option", "NnModel=nets/candidate.onnx",
            "--candidate-option", "NnBatchSize=16",
            "--baseline", "commit:45d5db5498e1739b2307b3719891a1eec24f7fbd",
            "--baseline-args", "--rules lishogi",
            "--each", "time=10000+100", "--rules", "lishogi", "--max-ply", "4096",
            "--concurrency", "1", "elo", "--pairs", "20",
        ])

    def test_random_full_argv(self):
        args = run_rl2_match.parser().parse_args(self.arguments("random"))
        self.assertEqual(run_rl2_match.build_argv(args), [
            "target/release/minase", "match", "run",
            "--run-dir", "data/matches/rl2-test", "--seed", "123",
            "--candidate", "commit:0123456789abcdef0123456789abcdef01234567",
            "--candidate-args", "--search mcts",
            "--candidate-option", "NnModel=nets/candidate.onnx",
            "--candidate-option", "NnBatchSize=16",
            "--baseline", "random",
            "--each", "nodes=1", "--rules", "lishogi", "--max-ply", "4096",
            "--concurrency", "2", "elo", "--pairs", "20",
        ])

    def test_versus_full_argv(self):
        args = run_rl2_match.parser().parse_args(self.arguments("versus"))
        self.assertEqual(run_rl2_match.build_argv(args), [
            "target/release/minase", "match", "run",
            "--run-dir", "data/matches/rl2-test", "--seed", "123",
            "--candidate", "commit:0123456789abcdef0123456789abcdef01234567",
            "--candidate-args", "--search mcts",
            "--candidate-option", "NnModel=nets/candidate.onnx",
            "--candidate-option", "NnBatchSize=16",
            "--baseline", "commit:0123456789abcdef0123456789abcdef01234567",
            "--baseline-args", "--search mcts",
            "--baseline-option", "NnModel=nets/baseline.onnx",
            "--baseline-option", "NnBatchSize=16",
            "--each", "nodes=800", "--rules", "lishogi", "--max-ply", "4096",
            "--concurrency", "2", "elo", "--pairs", "20",
        ])

    def test_invalid_commits_and_network_paths(self):
        for option, values in (
            ("--mcts-commit", ("", "45d5db54", "a" * 39, "a" * 41, "g" * 40, "a" * 39 + "\n")),
            ("--net", ("", "nets/a b.onnx", "nets/a\tb.onnx", "nets/a\nb.onnx", "nets/a\u3000b.onnx")),
            ("--baseline-net", ("", "nets/a b.onnx", "nets/a\tb.onnx", "nets/a\nb.onnx")),
        ):
            for value in values:
                with self.subTest(option=option, value=value):
                    argv = self.arguments("versus")
                    argv[argv.index(option) + 1] = value
                    with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                        run_rl2_match.parser().parse_args(argv)
                    self.assertEqual(error.exception.code, 2)

    def test_all_match_arguments_required(self):
        for command in ("stc", "random", "versus"):
            original = self.arguments(command)
            for index in range(1, len(original), 2):
                with self.subTest(command=command, missing=original[index]):
                    with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                        run_rl2_match.parser().parse_args(original[:index] + original[index + 2:])
                    self.assertEqual(error.exception.code, 2)


class OutputTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(dir=Path(__file__).resolve().parent)
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.out = self.directory / "check"
        self.argv = ["--net", str(self.directory / "net.onnx"),
                     "--reference", str(self.directory / "reference.json"), "--out", str(self.out)]
        self.args = check_outputs.parser().parse_args(self.argv)

    def test_cpu_usi_lines(self):
        self.assertEqual(check_outputs.usi_lines(Path("/net.onnx"), "cpu", 7, Path("/check"), "cpu-b7"), [
            "usi", "setoption name RuleSet value lishogi",
            "setoption name NnModel value /net.onnx",
            "setoption name NnBackend value cpu", "setoption name NnBatchSize value 7",
            "isready", "nn-eval out /check/cpu-b7.json", "quit",
        ])

    def test_tensorrt_usi_lines(self):
        self.assertEqual(check_outputs.usi_lines(Path("/net.onnx"), "tensorrt", 256, Path("/check"), "trt-b256"), [
            "usi", "setoption name RuleSet value lishogi",
            "setoption name NnModel value /net.onnx",
            "setoption name NnBackend value tensorrt", "setoption name NnBatchSize value 256",
            "setoption name NnCacheDir value /check/trt-cache",
            "isready", "nn-eval out /check/trt-b256.json", "quit",
        ])

    def test_missing_or_empty_environment_rejected_before_launch(self):
        for environment in ({}, {"ORT_DYLIB_PATH": ""}):
            with self.subTest(environment=environment), patch.dict("os.environ", environment, clear=True), \
                    patch.object(check_outputs.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "env.sh"):
                    check_outputs.check_outputs(self.args)
                run.assert_not_called()
                self.assertFalse(self.out.exists())

    def test_nonempty_output_rejected_without_overwrite(self):
        self.out.mkdir()
        saved = self.out / "summary.json"
        saved.write_text("previous check", encoding="utf-8")
        with patch.dict("os.environ", {"ORT_DYLIB_PATH": "/unused/libonnxruntime.so"}), \
                patch.object(check_outputs.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                check_outputs.check_outputs(self.args)
            run.assert_not_called()
        self.assertEqual(saved.read_text(encoding="utf-8"), "previous check")

    def test_output_file_rejected(self):
        self.out.write_text("preserve", encoding="utf-8")
        with patch.dict("os.environ", {"ORT_DYLIB_PATH": "/unused/libonnxruntime.so"}), \
                patch.object(check_outputs.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                check_outputs.check_outputs(self.args)
            run.assert_not_called()
        self.assertEqual(self.out.read_text(encoding="utf-8"), "preserve")

    def test_all_check_arguments_required(self):
        for index in (0, 2, 4):
            with self.subTest(missing=self.argv[index]), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    check_outputs.parser().parse_args(self.argv[:index] + self.argv[index + 2:])
                self.assertEqual(error.exception.code, 2)

    def test_usi_path_newlines_rejected(self):
        for option in ("--net", "--out"):
            for value in ("", "a\nb", "a\rb"):
                argv = self.argv.copy()
                argv[argv.index(option) + 1] = value
                with self.subTest(option=option, value=value), contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as error:
                        check_outputs.parser().parse_args(argv)
                    self.assertEqual(error.exception.code, 2)

    def test_engine_failure_stops_before_comparison(self):
        for code, output in ((9, "info string end\n"), (0, ""), (0, "info string end extra\n")):
            with self.subTest(code=code, output=output), \
                    tempfile.TemporaryDirectory(dir=self.directory) as directory:
                self.args.out = Path(directory)

                def engine(argv, **kwargs):
                    kwargs["stdout"].write(output)
                    return subprocess.CompletedProcess(argv, code)

                with patch.dict("os.environ", {"ORT_DYLIB_PATH": "/unused/libonnxruntime.so"}), \
                        patch.object(check_outputs.subprocess, "run", side_effect=engine) as run:
                    with self.assertRaises(RuntimeError):
                        check_outputs.check_outputs(self.args)
                    self.assertEqual(run.call_count, 1)
                self.assertEqual((self.args.out / "cpu-b1.usi.log").read_text(encoding="utf-8"), output)
                self.assertFalse((self.args.out / "summary.json").exists())

    def test_six_outputs_then_all_comparisons_and_summary(self):
        # 手引き3・5節と指示書3の順序・許容誤差。実装の定数は参照しない。
        cases = [
            ("cpu", 1, "cpu-b1", "fp16-logits", "1e-4", "1e-4"),
            ("tensorrt", 1, "trt-b1", "probabilities", "5e-2", "5e-2"),
            ("cpu", 7, "cpu-b7", "fp16-logits", "1e-4", "1e-4"),
            ("tensorrt", 7, "trt-b7", "probabilities", "5e-2", "5e-2"),
            ("cpu", 256, "cpu-b256", "fp16-logits", "1e-4", "1e-4"),
            ("tensorrt", 256, "trt-b256", "probabilities", "5e-2", "5e-2"),
        ]
        root = Path(__file__).resolve().parents[4]
        for failed in (False, True):
            with self.subTest(failed=failed), tempfile.TemporaryDirectory(dir=self.directory) as directory:
                self.args.out = Path(directory) / "out"
                if failed:
                    self.args.out.mkdir()  # 既存の空ディレクトリも受理する。
                out = self.args.out
                calls = []

                def process(argv, **kwargs):
                    index = len(calls)
                    calls.append(argv)
                    self.assertEqual(kwargs["cwd"], root)
                    if index < 6:
                        backend, batch, name, _, _, _ = cases[index]
                        self.assertEqual(argv, [str(root / "target/release/minase"),
                                                "--protocol", "usi", "--search", "mcts"])
                        lines = ["usi", "setoption name RuleSet value lishogi",
                                 f"setoption name NnModel value {self.directory / 'net.onnx'}",
                                 f"setoption name NnBackend value {backend}",
                                 f"setoption name NnBatchSize value {batch}"]
                        if backend == "tensorrt":
                            lines.append(f"setoption name NnCacheDir value {out / 'trt-cache'}")
                        lines += ["isready", f"nn-eval out {out / (name + '.json')}", "quit"]
                        self.assertEqual(kwargs["input"], "\n".join(lines) + "\n")
                        self.assertTrue(kwargs["text"])
                        self.assertEqual(Path(kwargs["stdout"].name), out / f"{name}.usi.log")
                        kwargs["stdout"].write("usiok\nreadyok\ninfo string end\n")
                        (out / f"{name}.json").write_text("{}", encoding="utf-8")
                        return subprocess.CompletedProcess(argv, 0)
                    _, _, name, policy, ptol, vtol = cases[index - 6]
                    self.assertEqual(argv, [sys.executable, "-I", str(root / "tools/train/zero/nn_compare.py"),
                                            "--reference", str(self.directory / "reference.json"),
                                            "--candidate", str(out / f"{name}.json"),
                                            "--policy", policy, "--policy-tolerance", ptol,
                                            "--value-tolerance", vtol])
                    self.assertEqual(Path(kwargs["stdout"].name), out / f"{name}.compare.json")
                    self.assertEqual(Path(kwargs["stderr"].name), out / f"{name}.compare.err")
                    kwargs["stdout"].write('{"comparison": true}\n')
                    kwargs["stderr"].write("comparison diagnostic\n")
                    return subprocess.CompletedProcess(argv, 2 if failed and index == 6 else 0)

                printed = io.StringIO()
                with patch.dict("os.environ", {"ORT_DYLIB_PATH": "/unused/libonnxruntime.so"}), \
                        patch.object(check_outputs.subprocess, "run", side_effect=process), \
                        patch.object(check_outputs.time, "monotonic", side_effect=[100.0, 112.5]), \
                        contextlib.redirect_stdout(printed):
                    self.assertEqual(check_outputs.check_outputs(self.args), 1 if failed else 0)
                self.assertEqual(len(calls), 12)
                saved = (out / "summary.json").read_text(encoding="utf-8")
                self.assertEqual(printed.getvalue(), saved)
                self.assertEqual(json.loads(saved), {
                    "results": [
                        {"backend": "cpu", "batch": 1, "exit_code": 2 if failed else 0},
                        {"backend": "tensorrt", "batch": 1, "exit_code": 0},
                        {"backend": "cpu", "batch": 7, "exit_code": 0},
                        {"backend": "tensorrt", "batch": 7, "exit_code": 0},
                        {"backend": "cpu", "batch": 256, "exit_code": 0},
                        {"backend": "tensorrt", "batch": 256, "exit_code": 0},
                    ], "elapsed_seconds": 12.5,
                })
                for _, _, name, _, _, _ in cases:
                    self.assertEqual((out / f"{name}.compare.json").read_text(encoding="utf-8"),
                                     '{"comparison": true}\n')
                    self.assertEqual((out / f"{name}.compare.err").read_text(encoding="utf-8"),
                                     "comparison diagnostic\n")


if __name__ == "__main__":
    unittest.main()
