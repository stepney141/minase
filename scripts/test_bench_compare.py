#!/usr/bin/env python3
"""benchの準備命令と、ネット全体のSHA-256によるキャッシュの区別を検証する。"""

from contextlib import redirect_stdout
import hashlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import bench_compare as bench


OUTPUT = ("info string position=start depth=5 nodes=100 best=7g7f score=10 elapsed=1.0s\n"
          "info string median: depth=5 nodes=100 elapsed=1.0s nps=100\n"
          "info string end\n")


class BenchTests(unittest.TestCase):
    def test_bench_prepares_rules_and_network_before_search(self):
        with patch.object(subprocess, "run", return_value=subprocess.CompletedProcess([], 0, OUTPUT)) as run:
            self.assertEqual(bench.run_bench(Path("/engine"), 5, 2, 3, Path("/net file.bin")), OUTPUT)
        self.assertEqual(run.call_args.args[0], ["/engine", "--protocol", "usi"])
        self.assertEqual(run.call_args.kwargs["input"],
                         "setoption name RuleSet value JCSA\n"
                         "setoption name EvalFile value /net file.bin\nisready\n"
                         "bench depth 5 threads 2 repetitions 3\nquit\n")
        self.assertTrue(run.call_args.kwargs["check"])

    def test_reference_cache_changes_when_only_network_header_changes(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            network = root / "network.bin"
            for commit in ("reference", "baseline", "parent"):
                (root / commit).mkdir()
            arguments = ["bench_compare.py", "--reference", "reference", "--baseline", "baseline",
                         "--parent", "parent", "--eval-file", str(network), "--rounds", "1", "--threads", "2"]
            with (patch.object(sys, "argv", arguments),
                  patch.object(bench, "run", return_value=str(root)),
                  patch.object(bench, "resolve_commit", side_effect=lambda root, spec: spec),
                  patch.object(bench, "build_commit", side_effect=lambda root, commit: root / commit / "target/release/minase"),
                  patch.object(bench, "build_worktree", return_value=root / "target/release/minase"),
                  patch.object(bench, "run_bench", return_value=OUTPUT) as run,
                  redirect_stdout(io.StringIO())):
                digests = []
                for content, expected_calls in ((b"header A\x00weights", 6),
                                                 (b"header A\x00weights", 5),
                                                 (b"header B\x00weights", 6)):
                    network.write_bytes(content)
                    run.reset_mock()
                    self.assertEqual(bench.main(), 0)
                    self.assertEqual(run.call_count, expected_calls)
                    self.assertTrue(all(call.args[-1] == network for call in run.call_args_list))
                    digests.append(hashlib.sha256(content).hexdigest())
                self.assertEqual({path.name for path in (root / "reference").glob("*.txt")},
                                 {f"bench-depth5-threads2-{digest}.txt" for digest in digests})


if __name__ == "__main__":
    unittest.main()
