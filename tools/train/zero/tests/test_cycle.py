"""サイクル制御が組み立てるコマンド、状態と記録の更新、再開、停止規則、および期限の判定を、偽の実行関数で検査する。"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from minase_zero import cycle
from minase_zero.cycle import CycleError, Layout, WindowSchedule

SEED = 100
"""試験の基本シード。"""


def sha256(path: Path) -> str:
    """ファイルのSHA-256を返す。"""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def value_after(argv: list[str], flag: str) -> str:
    """`argv`の`flag`の直後の値を返す。"""
    return argv[argv.index(flag) + 1]


class Crash(CycleError):
    """偽の実行関数が中断を模して投げる誤り。"""


class FakeRunner:
    """生成器、学習器、および対局ハーネスの最小の出力を書く偽の実行関数。

    `crash`に段階名（`selfplay`、`train`、`progress`）を入れると、その段階の出力の途中で一度だけ中断する。
    `elos`は進捗測定が順に返すEloの点推定である。
    """

    def __init__(self, elos=(), crash=None):
        self.calls: list[tuple[list[str], Path, str | None]] = []
        self.elos = list(elos)
        self.crash = crash

    def __call__(self, argv: list[str], log_path: Path, stdin: str | None) -> None:
        self.calls.append((argv, log_path, stdin))
        log_path.parent.mkdir(parents=True, exist_ok=True)
        if argv[1:3] == ["data", "zero-selfplay"]:
            self.selfplay(argv)
        elif argv[1:3] == ["-m", "minase_zero.train"]:
            self.train(argv)
        elif argv[1:3] == ["match", "run"]:
            self.match(argv, log_path)

    def crash_now(self, stage: str) -> None:
        """`stage`で中断する指定があれば、一度だけ中断する。"""
        if self.crash == stage:
            self.crash = None
            raise Crash(f"simulated crash in {stage}")

    def selfplay(self, argv: list[str]) -> None:
        out_dir = Path(value_after(argv, "--out-dir"))
        out_dir.mkdir(parents=True, exist_ok=True)
        self.crash_now("selfplay")
        summary = {
            "chunk": 0, "seed": int(value_after(argv, "--seed")), "nn_model": None,
            "network_checksum": sha256(Path(value_after(argv, "--nn-model"))),
            "games": 10, "positions": 4000, "endings": {}, "max_ply_games": 1,
            "resigned_games": 3, "playthrough_games": 2, "false_resign_candidates": 10,
            "false_resignations": 1, "total_plies": 4500, "average_plies": 450.0,
            "elapsed_seconds": 1.0, "positions_per_second": 4000.0,
        }
        (out_dir / "chunk-00000.json").write_text(json.dumps(summary))
        (out_dir / "chunk-00000.mnzt").write_bytes(out_dir.as_posix().encode())

    def train(self, argv: list[str]) -> None:
        if "--resume" in argv:
            run_dir = Path(value_after(argv, "--resume"))
            started = json.loads((run_dir / "checkpoint.pt").read_text())
        else:
            run_dir = Path(value_after(argv, "--out"))
            run_dir.mkdir(parents=True)
            data = argv[argv.index("--data") + 1:argv.index("--out")]
            started = {"data": data, "init": value_after(argv, "--init"),
                       "seed": int(value_after(argv, "--seed"))}
            (run_dir / "checkpoint.pt").write_text(json.dumps(started))
        self.crash_now("train")
        net = run_dir / "net.onnx"
        net.write_bytes(run_dir.as_posix().encode())
        (run_dir / "net.sha256").write_text(f"{sha256(net)}  net.onnx\n")
        log = [
            {"kind": "train", "step": 1, "loss": 3.0},
            {"kind": "validation", "step": 1, "loss": 3.5},
            {"kind": "train", "step": 2, "loss": 2.5},
        ]
        (run_dir / "log.jsonl").write_text("".join(json.dumps(entry) + "\n" for entry in log))
        provenance = {
            "data": [{"path": path, "sha256": sha256(Path(path))} for path in started["data"]],
            "init": {"path": started["init"], "sha256": "0" * 64},
            "config": {"seed": started["seed"]},
        }
        (run_dir / "provenance.json").write_text(json.dumps(provenance))

    def match(self, argv: list[str], log_path: Path) -> None:
        if "--run-dir" in argv:
            Path(value_after(argv, "--run-dir")).mkdir(parents=True)
        self.crash_now("progress")
        elo = self.elos.pop(0)
        with log_path.open("a") as log:
            log.write("summary: mode=elo pairs=4 valid_pairs=4 discarded_pairs=0\n")
            log.write(f"elo: estimate={elo:.6f} ci95=[{elo - 30:.6f}, {elo + 30:.6f}]\n")
            log.write("engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 "
                      "rejected_moves=0\n")


@pytest.fixture
def workspace(tmp_path, monkeypatch):
    """初期ネットとSLのチェックポイントを置き、GPUの名前の問い合わせを固定する。"""
    monkeypatch.setattr(cycle, "gpu_names", lambda: ["Fake GPU"])
    (tmp_path / "initial.onnx").write_bytes(b"initial network")
    (tmp_path / "sl.pt").write_bytes(b"sl checkpoint")
    return tmp_path


def arguments(workspace: Path, *extra: str, cycles: int = 2, deadline: str | None = None):
    """偽の実行関数で数サイクル走らせる引数を作る。"""
    deadline = deadline or (datetime.now(UTC) + timedelta(days=30)).isoformat()
    return cycle.parse_args([
        "--root", str(workspace / "root"), "--minase", "/opt/minase", "--commit", "c0ffee",
        "--rules", "engine-default", "--net", str(workspace / "initial.onnx"),
        "--init-checkpoint", str(workspace / "sl.pt"), "--games", "10",
        "--nn-backend", "cpu", "--nn-cache-dir", str(workspace / "cache"), "--lr", "0.02",
        "--window-schedule", "1:1,2", "--seed", str(SEED), "--cycles", str(cycles),
        "--deadline", deadline, "--device", "cpu", "--workers", "0", "--progress-every", "2",
        "--progress-pairs", "4", *extra,
    ])


def commands(runner: FakeRunner) -> list[list[str]]:
    """偽の実行関数が受けたコマンドの列を返す。"""
    return [argv for argv, _, _ in runner.calls]


def test_window_schedule_parses_widths_and_rejects_malformed_text():
    """段階表は各区間の幅を返し、表記へ戻せて、不正な表記を拒否する。"""
    schedule = WindowSchedule.parse("1:10,2:30,4")
    assert [schedule.width(n) for n in (1, 10, 11, 30, 31, 500)] == [1, 1, 2, 2, 4, 4]
    assert str(schedule) == "1:10,2:30,4"
    assert WindowSchedule.parse("4").width(1) == 4
    for text in ("", "1:10", "1,2", "1:10,2:5,4", "0", "1:10,x", "1:0,2", "1:10,2:20"):
        with pytest.raises(ValueError):
            WindowSchedule.parse(text)
    assert list(cycle.window_cycles(5, 4)) == [2, 3, 4, 5]
    assert list(cycle.window_cycles(2, 4)) == [1, 2]
    with pytest.raises(argparse.ArgumentTypeError, match="last entry"):
        cycle.parse_schedule("1:10")


def test_match_summary_parsing_handles_finite_infinite_and_unavailable_elo():
    """最後の`elo:`行の有限と無限大の推定を読み、Eloが求まらない出力は誤りにする。"""
    failures = ("engine_failures: illegal_moves=0 crashes=1 timeouts=0 time_forfeits=0 "
                "rejected_moves=2\n")
    text = ("elo: estimate=5.000000 ci95=[1.000000, 9.000000]\n"
            "elo: estimate=-12.500000 ci95=[-40.000000, 15.000000]\n" + failures)
    result = cycle.parse_match_summary(text)
    assert result == {
        "elo": -12.5, "ci95": [-40.0, 15.0],
        "engine_failures": {"illegal_moves": 0, "crashes": 1, "timeouts": 0,
                            "time_forfeits": 0, "rejected_moves": 2},
    }
    infinite = cycle.parse_match_summary("elo: estimate=-inf ci95=[-inf, +inf]\n" + failures)
    assert infinite["elo"] == "-inf" and infinite["ci95"] == ["-inf", "+inf"]
    assert float(infinite["elo"]) < 0
    assert cycle.parse_match_summary("elo: estimate=+inf ci95=[3.0, +inf]\n" + failures)["elo"] == "+inf"
    for broken in ("elo: unavailable ci95=unavailable\n" + failures,
                   "elo: estimate=1.0 ci95=[0.0, 2.0]\n", failures):
        with pytest.raises(CycleError):
            cycle.parse_match_summary(broken)


def test_cycles_build_the_expected_commands_and_records(workspace, capsys):
    """2サイクルで、生成、窓の学習、進捗測定のコマンドと、記録、状態、ネットの写しを検査する。"""
    args = arguments(workspace)
    runner = FakeRunner(elos=[25.0])
    assert cycle.run(args, runner) == cycle.EXIT_COMPLETED

    root = (workspace / "root").resolve()
    layout = Layout(root)
    cache = str(workspace / "cache")
    calls = commands(runner)
    assert calls[0] == [
        "/opt/minase", "data", "zero-selfplay",
        "--out-dir", str(root / "cycles/001/selfplay"), "--games", "10",
        "--seed", str(SEED + 1_000_003), "--rules", "engine-default",
        "--games-in-flight", "256", "--threads", "2", "--simulations", "32", "--max-ply", "1000",
        "--evaluator", "ort", "--nn-backend", "cpu", "--nn-model", str(root / "nets/000.onnx"),
        "--nn-cache-dir", cache, "--nn-batch-size", "128",
    ]
    assert calls[1] == [
        sys.executable, "-m", "minase_zero.train",
        "--data", str(root / "cycles/001/selfplay/chunk-00000.mnzt"),
        "--out", str(root / "cycles/001/train"), "--init", str(workspace / "sl.pt"),
        "--blocks", "10", "--channels", "128", "--batch-size", "1024", "--epochs", "1",
        "--lr", "0.02", "--lambda", "0.333", "--seed", str(SEED + 1_000_003),
        "--workers", "0", "--device", "cpu",
    ]
    # サイクル2は前のサイクルのネットで生成し、幅2の窓で前のサイクルのチェックポイントから学習する。
    assert value_after(calls[2], "--nn-model") == str(root / "nets/001.onnx")
    assert value_after(calls[2], "--seed") == str(SEED + 2 * 1_000_003)
    assert calls[3][calls[3].index("--data") + 1:calls[3].index("--out")] == [
        str(root / "cycles/001/selfplay/chunk-00000.mnzt"),
        str(root / "cycles/002/selfplay/chunk-00000.mnzt"),
    ]
    assert value_after(calls[3], "--init") == str(root / "cycles/001/train/checkpoint.pt")
    # 進捗測定は両方のネットのキャッシュを作ってから、サイクル2のネットと0のネットを対局させる。
    for (argv, log, stdin), net in zip(runner.calls[4:6], ("002", "000"), strict=True):
        assert argv == ["/opt/minase", "--protocol", "usi", "--rules", "engine-default",
                        "--search", "mcts"]
        assert log == layout.log(2, "prewarm")
        assert stdin == (
            "usi\nsetoption name NnBackend value cpu\n"
            f"setoption name NnModelPath value {root / 'nets' / (net + '.onnx')}\n"
            f"setoption name NnCacheDir value {cache}\nisready\nquit\n"
        )
    assert calls[6] == [
        "/opt/minase", "match", "run", "--run-dir", str(root / "progress/002"),
        "--seed", str(SEED + 7_000_003 * 2), "--rules", "engine-default",
        "--candidate", "commit:c0ffee", "--candidate-args", "--search mcts",
        "--candidate-option", "NnBackend=cpu",
        "--candidate-option", f"NnModelPath={root / 'nets/002.onnx'}",
        "--candidate-option", f"NnCacheDir={cache}",
        "--baseline", "commit:c0ffee", "--baseline-args", "--search mcts",
        "--baseline-option", "NnBackend=cpu",
        "--baseline-option", f"NnModelPath={root / 'nets/000.onnx'}",
        "--baseline-option", f"NnCacheDir={cache}",
        "--each", "nodes=800", "--concurrency", "2", "elo", "--pairs", "4",
    ]
    assert len(calls) == 7

    assert sha256(layout.net(0)) == sha256(workspace / "initial.onnx")
    for number in (1, 2):
        assert sha256(layout.net(number)) == sha256(layout.train(number) / "net.onnx")
    state = json.loads(layout.state.read_text())
    assert state["completed_cycles"] == 2 and state["cycle"] is None
    assert state["config"]["window_schedule"] == "1:1,2"

    first, second = (json.loads(line) for line in layout.record.read_text().splitlines())
    assert first["cycle"] == 1 and first["progress"] is None
    assert first["network"] == {"input_sha256": sha256(layout.net(0)),
                                "sha256": sha256(layout.net(1))}
    assert first["selfplay"]["positions"] == 4000
    assert first["selfplay"]["games"] == 10
    assert first["selfplay"]["max_ply_games"] == 1
    assert first["selfplay"]["false_resign_rate"] == pytest.approx(0.1)
    assert first["train"]["final_train"] == {"kind": "train", "step": 2, "loss": 2.5}
    assert first["train"]["final_validation"] == {"kind": "validation", "step": 1, "loss": 3.5}
    assert first["machine"]["gpus"] == ["Fake GPU"]
    assert first["provenance"]["commit"] == "c0ffee"
    assert first["provenance"]["generation_seed"] == SEED + 1_000_003
    assert [entry["path"] for entry in second["provenance"]["window_data"]] == calls[3][
        calls[3].index("--data") + 1:calls[3].index("--out")
    ]
    assert second["train"]["window"] == 2 and second["train"]["window_cycles"] == [1, 2]
    assert second["progress"]["elo"] == 25.0 and second["progress"]["baseline_cycle"] == 0
    assert second["provenance"]["progress_seed"] == SEED + 7_000_003 * 2
    assert set(second["elapsed_seconds"]) == {"selfplay", "train", "progress", "total"}

    # 誤った投了の割合が10%なので、5%を超えた警告を出す。
    assert "false resignation rate 10.0%" in capsys.readouterr().err


@pytest.mark.parametrize("stage", ["selfplay", "train", "progress"])
def test_resume_skips_finished_stages_and_resumes_the_interrupted_one(workspace, stage):
    """中断した段階は再開の形で呼び直し、完了済みの段階は呼ばない。"""
    root = (workspace / "root").resolve()
    crashing = FakeRunner(elos=[], crash=stage)
    with pytest.raises(Crash):
        cycle.run(arguments(workspace), crashing)
    state = json.loads((root / "state.json").read_text())
    done = {"selfplay": [], "train": ["selfplay"], "progress": ["selfplay", "train"]}[stage]
    number = 2 if stage == "progress" else 1
    assert state["cycle"]["number"] == number
    assert sorted(state["cycle"]["stages"]) == sorted(done)

    runner = FakeRunner(elos=[3.0])
    assert cycle.run(arguments(workspace), runner) == cycle.EXIT_COMPLETED
    first = commands(runner)[0]
    if stage == "selfplay":
        assert first[1:3] == ["data", "zero-selfplay"] and first[-1] == "--resume"
    elif stage == "train":
        assert first == [sys.executable, "-m", "minase_zero.train",
                         "--resume", str(root / "cycles/001/train"),
                         "--workers", "0", "--device", "cpu"]
    else:
        assert commands(runner)[2] == ["/opt/minase", "match", "run",
                                       "--resume", str(root / "progress/002")]
    records = [json.loads(line) for line in (root / "record.jsonl").read_text().splitlines()]
    assert [record["cycle"] for record in records] == [1, 2]


def test_resume_rejects_changed_settings(workspace):
    """再開時にデータと来歴を変える設定が記録と異なれば誤りにする。"""
    assert cycle.run(arguments(workspace, cycles=1), FakeRunner()) == cycle.EXIT_COMPLETED
    changed = arguments(workspace, cycles=1)
    changed.seed = SEED + 1
    with pytest.raises(CycleError, match="seed"):
        cycle.run(changed, FakeRunner())


def test_two_consecutive_non_positive_progress_measurements_stop_the_run(workspace, capsys):
    """進捗測定の点推定が2回続けて0以下なら、記録を書いてから終了コード2で止まる。"""
    runner = FakeRunner(elos=[-1.0, 0.0])
    assert cycle.run(arguments(workspace, cycles=10), runner) == cycle.EXIT_PROGRESS_STOP
    root = (workspace / "root").resolve()
    records = [json.loads(line) for line in (root / "record.jsonl").read_text().splitlines()]
    assert [record["cycle"] for record in records] == [1, 2, 3, 4]
    assert [record["progress"]["elo"] for record in records if record["progress"]] == [-1.0, 0.0]
    assert "stopping" in capsys.readouterr().out

    assert not cycle.progress_stalled([{"progress": {"elo": -1.0}}, {"progress": None},
                                       {"progress": {"elo": "+inf"}}, {"progress": {"elo": -2.0}}])
    assert cycle.progress_stalled([{"progress": {"elo": "-inf"}}, {"progress": {"elo": -2.0}}])
    assert not cycle.progress_stalled([{"progress": {"elo": -3.0}}])


def test_deadline_stops_before_a_cycle_that_would_not_finish(workspace):
    """直近の平均所要時間で残りのサイクルが期限を超える見込みなら、始めずに終了コード3で止まる。"""
    assert cycle.run(arguments(workspace, cycles=1), FakeRunner()) == cycle.EXIT_COMPLETED
    past = (datetime.now(UTC) - timedelta(minutes=1)).isoformat()
    runner = FakeRunner()
    assert cycle.run(arguments(workspace, cycles=3, deadline=past), runner) == cycle.EXIT_DEADLINE
    assert runner.calls == []

    current = datetime(2026, 10, 1, tzinfo=UTC)
    records = [{"elapsed_seconds": {"total": total}} for total in (10.0, 20.0, 30.0, 40.0)]
    finish, mean = cycle.projected_finish(records, 2, current)
    assert mean == 30.0 and finish == current + timedelta(seconds=60)
    assert cycle.projected_finish([], 5, current) == (current, 0.0)
    # 時差のない期限は実行機の地方時として読む。
    assert cycle.parse_deadline("2026-11-01T00:00:00").tzinfo is not None


def test_record_is_not_duplicated_when_the_state_was_not_saved(tmp_path):
    """記録を書いた後に状態の保存前で止まった再実行は、同じサイクルの行を重ねて書かない。"""
    layout = Layout(tmp_path)
    cycle.append_record(layout, {"cycle": 1})
    cycle.append_record(layout, {"cycle": 1})
    cycle.append_record(layout, {"cycle": 2})
    assert [json.loads(line)["cycle"] for line in layout.record.read_text().splitlines()] == [1, 2]


def test_net_with_a_wrong_checksum_is_not_installed(tmp_path):
    """学習器の`net.sha256`と一致しないネットは`nets/`へ写さない。"""
    (tmp_path / "net.onnx").write_bytes(b"network")
    (tmp_path / "net.sha256").write_text("0" * 64 + "  net.onnx\n")
    destination = tmp_path / "nets" / "001.onnx"
    with pytest.raises(CycleError, match="checksum"):
        cycle.install_net(tmp_path, destination)
    assert not destination.exists()
    assert list((tmp_path / "nets").iterdir()) == []
