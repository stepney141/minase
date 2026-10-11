"""強化学習のサイクル（生成、学習、進捗測定、記録の1行）を繰り返す。

1サイクルは次の手順からなる（docs/plans/alphazero.md の「生成と学習を交互に行う」と「強化学習」）。

1. 生成: 前のサイクルのネットで`minase data zero-selfplay`を実行する。
2. 学習: 直近kサイクルの生成データで、前のサイクルのチェックポイントから`minase_zero.train`を1エポック実行し、
   出力したネットを`nets/`へ写して検査和を確かめる。
3. 進捗測定: サイクル番号が`--progress-every`の倍数なら、新しいネットと`--progress-every`サイクル前のネットを
   `minase match run`の固定ペア数で対局させ、Eloの点推定を得る。
4. 記録: サイクルの結果を`record.jsonl`へ1行追記する。

進行状態は`--root`の`state.json`へ段階ごとに原子的に保存し、同じ`--root`で再実行すると続きから走る。
外部コマンドは`run_command`だけを通して呼ぶので、試験では偽の実行関数に置き換えられる。

終了コードは、指定のサイクル数を終えたら0、外部コマンドの失敗などの誤りで1、
進捗測定の点推定が2回続けて0以下なら2、期限を超える見込みなら3である。

使い方:
    python -m minase_zero.cycle --root data/zero/rl --minase target/release/minase --commit <hash> \\
        --rules engine-default --net sl/net.onnx --init-checkpoint sl/checkpoint.pt --games 2400 \\
        --nn-cache-dir data/cache/trt-minase --lr 0.02 --window-schedule 1:10,2:30,4 --seed 1 \\
        --cycles 100 --deadline 2026-11-10T00:00:00+09:00
"""

from __future__ import annotations

import argparse
import itertools
import json
import math
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from minase_zero.mnzt import sha256_file

STATE_FORMAT = "minase-zero-cycle-state"
"""`state.json`の形式名。"""
STATE_VERSION = 1
"""`state.json`の形式の版。"""

GENERATION_SEED_STRIDE = 1_000_003
"""サイクル番号ごとに生成と学習のシードを離す幅。"""
PROGRESS_SEED_STRIDE = 7_000_003
"""サイクル番号ごとに進捗測定のシードを離す幅。"""
FALSE_RESIGN_WARNING = 0.05
"""誤った投了の割合がこれを超えたら警告する（設計書「強化学習」）。"""
MAX_PLY_WARNING = 0.10
"""手数上限に達して除いた対局の割合がこれを超えたら警告する（設計書「強化学習」）。"""
DEADLINE_HISTORY = 3
"""期限の見込みに使う直近の完了サイクルの数。"""

EXIT_COMPLETED = 0
"""指定のサイクル数を終えた。"""
EXIT_PROGRESS_STOP = 2
"""進捗測定の停止規則で止めた。"""
EXIT_DEADLINE = 3
"""期限を超える見込みのため次のサイクルを始めなかった。"""

# 再開時に`state.json`の記録との一致を求める設定。データの内容と来歴を変える値である。
FIXED_KEYS = (
    "seed",
    "games",
    "simulations",
    "max_ply",
    "rules",
    "commit",
    "blocks",
    "channels",
    "batch_size",
    "lr",
    "value_lambda",
    "window_schedule",
    "progress_every",
    "progress_pairs",
    "progress_simulations",
)


class CycleError(Exception):
    """サイクルを続けられない誤り。"""


Runner = Callable[[list[str], Path, str | None], None]
"""外部コマンドを実行する関数の型。引数はコマンド、ログの経路、および標準入力へ渡す文字列である。"""


def run_command(argv: list[str], log_path: Path, stdin: str | None = None) -> None:
    """外部コマンドを実行し、標準出力と標準エラーを`log_path`へ追記する。

    ログは追記なので、再実行した段階のログは前の試行の後に続く。

    Raises:
        CycleError: コマンドを起動できないか、終了コードが0でない場合。
    """
    log_path.parent.mkdir(parents=True, exist_ok=True)
    feed = {"input": stdin} if stdin is not None else {"stdin": subprocess.DEVNULL}
    with log_path.open("a", encoding="utf-8") as log:
        log.write(f"$ {shlex.join(argv)}\n")
        log.flush()
        try:
            completed = subprocess.run(
                argv, stdout=log, stderr=subprocess.STDOUT, text=True, check=False, **feed
            )
        except OSError as error:
            raise CycleError(f"cannot run {argv[0]}: {error}") from error
    if completed.returncode != 0:
        raise CycleError(
            f"{shlex.join(argv)} exited with status {completed.returncode}; see {log_path}"
        )


@dataclass(frozen=True)
class WindowSchedule:
    """学習窓の幅kをサイクル番号から決める段階表。

    `1:10,2:30,4`は、サイクル1〜10でk=1、11〜30でk=2、以後k=4を表す。
    """

    steps: tuple[tuple[int, int], ...]
    """（幅、その幅を使う最後のサイクル番号）の組の列。最後のサイクル番号は狭義に増える。"""
    final: int
    """`steps`の後のすべてのサイクルで使う幅。"""

    @classmethod
    def parse(cls, text: str) -> WindowSchedule:
        """`幅:最後のサイクル,…,幅`の表記を読む。

        Raises:
            ValueError: 表記が不正な場合。
        """
        parts = text.split(",")
        steps = []
        for part in parts[:-1]:
            width, separator, last = part.partition(":")
            if not separator:
                raise ValueError(f"'{part}' must be WIDTH:LAST_CYCLE; only the last entry omits it")
            steps.append((_positive(width), _positive(last)))
        if ":" in parts[-1]:
            raise ValueError("the last entry must be a width without a last cycle")
        lasts = [last for _, last in steps]
        if any(later <= earlier for earlier, later in itertools.pairwise(lasts)):
            raise ValueError("the last cycles must increase strictly")
        return cls(tuple(steps), _positive(parts[-1]))

    def __str__(self) -> str:
        """`parse`が読む表記で返す。"""
        return ",".join([*(f"{width}:{last}" for width, last in self.steps), str(self.final)])

    def width(self, cycle: int) -> int:
        """サイクル`cycle`の学習窓の幅を返す。"""
        for width, last in self.steps:
            if cycle <= last:
                return width
        return self.final


def _positive(text: str) -> int:
    """正の整数を読む。"""
    try:
        value = int(text)
    except ValueError:
        raise ValueError(f"'{text}' is not an integer") from None
    if value < 1:
        raise ValueError(f"'{text}' must be positive")
    return value


def window_cycles(cycle: int, width: int) -> range:
    """サイクル`cycle`の学習窓に入るサイクル番号の範囲を返す。"""
    return range(max(1, cycle - width + 1), cycle + 1)


@dataclass(frozen=True)
class Layout:
    """`--root`の下のファイルの配置。"""

    root: Path

    def cycle_dir(self, cycle: int) -> Path:
        """サイクルのディレクトリ。"""
        return self.root / "cycles" / f"{cycle:03d}"

    def selfplay(self, cycle: int) -> Path:
        """生成器の`--out-dir`。"""
        return self.cycle_dir(cycle) / "selfplay"

    def train(self, cycle: int) -> Path:
        """学習器の実行ディレクトリ。"""
        return self.cycle_dir(cycle) / "train"

    def log(self, cycle: int, stage: str) -> Path:
        """段階のログ。"""
        return self.cycle_dir(cycle) / f"{stage}.log"

    def net(self, cycle: int) -> Path:
        """サイクルの結果のネット。`nets/000.onnx`は初期ネットである。"""
        return self.root / "nets" / f"{cycle:03d}.onnx"

    def progress(self, cycle: int) -> Path:
        """進捗測定の対局ハーネスの実行ディレクトリ。"""
        return self.root / "progress" / f"{cycle:03d}"

    @property
    def record(self) -> Path:
        """1サイクル1行の記録。"""
        return self.root / "record.jsonl"

    @property
    def state(self) -> Path:
        """進行状態。"""
        return self.root / "state.json"


def now() -> str:
    """現在時刻をUTCのISO 8601表記で返す。"""
    return datetime.now(UTC).isoformat(timespec="seconds")


def generation_seed(seed: int, cycle: int) -> int:
    """サイクルの生成と学習のシードを返す。"""
    return seed + cycle * GENERATION_SEED_STRIDE


def progress_seed(seed: int, cycle: int) -> int:
    """サイクルの進捗測定のシードを返す。"""
    return seed + PROGRESS_SEED_STRIDE * cycle


def atomic_write_text(path: Path, text: str) -> None:
    """一時ファイルへ書いて同期し、`path`へ名前を変えて確定させる。"""
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w", encoding="utf-8") as stream:
        stream.write(text)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def read_json(path: Path):
    """JSONファイルを読む。

    Raises:
        CycleError: 読めないか、JSONとして不正な場合。
    """
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise CycleError(f"cannot read {path}: {error}") from error


def read_jsonl(path: Path) -> list:
    """1行1値のJSONファイルを読む。ファイルがなければ空とする。

    Raises:
        CycleError: 読めないか、JSONとして不正な行がある場合。
    """
    if not path.exists():
        return []
    try:
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]
    except (OSError, ValueError) as error:
        raise CycleError(f"cannot read {path}: {error}") from error


def config_of(args: argparse.Namespace) -> dict:
    """`state.json`に保存する設定を作る。学習窓の段階表は表記の文字列で保存する。"""
    config = {key: getattr(args, key) for key in FIXED_KEYS}
    config["window_schedule"] = str(args.window_schedule)
    return config


def prepare(layout: Layout, args: argparse.Namespace) -> dict:
    """状態ファイルを読むか、なければ初期ネットを`nets/000.onnx`へ写して新しい状態を作る。

    Raises:
        CycleError: 状態ファイルが不正か、再開時の設定が記録と異なる場合。
    """
    config = config_of(args)
    if layout.state.exists():
        state = read_json(layout.state)
        if (
            not isinstance(state, dict)
            or (state.get("format"), state.get("version")) != (STATE_FORMAT, STATE_VERSION)
            or not isinstance(state.get("config"), dict)
        ):
            raise CycleError(f"{layout.state}: not a {STATE_FORMAT} version {STATE_VERSION} file")
        differing = [key for key in FIXED_KEYS if state["config"].get(key) != config[key]]
        if differing:
            details = ", ".join(
                f"{key}: recorded {state['config'].get(key)!r}, given {config[key]!r}"
                for key in differing
            )
            raise CycleError(f"the settings differ from {layout.state}: {details}")
        print(
            f"resuming {layout.root} after cycle {state['completed_cycles']}", flush=True
        )
        return state
    if not args.net.is_file():
        raise CycleError(f"initial network {args.net} does not exist")
    copy_atomically(args.net, layout.net(0))
    state = {
        "format": STATE_FORMAT,
        "version": STATE_VERSION,
        "config": config,
        "completed_cycles": 0,
        "cycle": None,
    }
    save_state(layout, state)
    print(f"started {layout.root} from {args.net}", flush=True)
    return state


def save_state(layout: Layout, state: dict) -> None:
    """状態ファイルを原子的に書く。"""
    layout.root.mkdir(parents=True, exist_ok=True)
    atomic_write_text(layout.state, json.dumps(state, indent=2) + "\n")


def copy_atomically(source: Path, destination: Path, expected: str | None = None) -> str:
    """`source`を一時名へ写して同期し、`destination`へ名前を変えて確定させ、検査和を返す。

    Raises:
        CycleError: `expected`を与え、写したファイルの検査和がそれと異なる場合。`destination`は変えない。
    """
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    shutil.copyfile(source, temporary)
    with temporary.open("rb") as stream:
        os.fsync(stream.fileno())
    checksum = sha256_file(temporary)
    if expected is not None and checksum != expected:
        temporary.unlink()
        raise CycleError(f"{source} has checksum {checksum}, but {expected} was expected")
    os.replace(temporary, destination)
    return checksum


def selfplay_command(args: argparse.Namespace, layout: Layout, cycle: int) -> list[str]:
    """生成器のコマンドを作る。出力ディレクトリが既にあれば`--resume`を付ける。"""
    out_dir = layout.selfplay(cycle)
    argv = [
        str(args.minase), "data", "zero-selfplay",
        "--out-dir", str(out_dir),
        "--games", str(args.games),
        "--seed", str(generation_seed(args.seed, cycle)),
        "--rules", args.rules,
        "--games-in-flight", str(args.games_in_flight),
        "--threads", str(args.threads),
        "--simulations", str(args.simulations),
        "--max-ply", str(args.max_ply),
        "--evaluator", "ort",
        "--nn-backend", args.nn_backend,
        "--nn-model", str(layout.net(cycle - 1)),
        "--nn-cache-dir", str(args.nn_cache_dir),
        "--nn-batch-size", str(args.nn_batch_size),
    ]
    if out_dir.exists():
        argv.append("--resume")
    return argv


def window_data(layout: Layout, cycle: int, width: int) -> list[Path]:
    """学習窓のMNZTの経路を、サイクル番号と塊の番号の順に返す。

    Raises:
        CycleError: 窓のいずれかのサイクルに塊がない場合。
    """
    paths = []
    for member in window_cycles(cycle, width):
        chunks = sorted(layout.selfplay(member).glob("chunk-*.mnzt"))
        if not chunks:
            raise CycleError(f"cycle {member} has no chunk-*.mnzt in {layout.selfplay(member)}")
        paths.extend(chunks)
    return paths


def train_command(
    args: argparse.Namespace, layout: Layout, cycle: int, width: int,
) -> list[str]:
    """学習器のコマンドを作る。

    実行ディレクトリにチェックポイントがあれば、設定をチェックポイントから読む再開の形にする。
    学習器は`--resume`と設定の引数の併用を拒否するからである。
    """
    run_dir = layout.train(cycle)
    base = [sys.executable, "-m", "minase_zero.train"]
    machine = ["--workers", str(args.workers), "--device", args.device]
    if (run_dir / "checkpoint.pt").exists():
        return [*base, "--resume", str(run_dir), *machine]
    init = args.init_checkpoint if cycle == 1 else layout.train(cycle - 1) / "checkpoint.pt"
    return [
        *base,
        "--data", *(str(path) for path in window_data(layout, cycle, width)),
        "--out", str(run_dir),
        "--init", str(init),
        "--blocks", str(args.blocks),
        "--channels", str(args.channels),
        "--batch-size", str(args.batch_size),
        "--epochs", "1",
        "--lr", str(args.lr),
        "--lambda", str(args.value_lambda),
        "--seed", str(generation_seed(args.seed, cycle)),
        *machine,
    ]


def prewarm_command(args: argparse.Namespace) -> list[str]:
    """TensorRTのエンジンのキャッシュを作るためにMCTSのエンジンを起動するコマンドを作る。"""
    return [str(args.minase), "--protocol", "usi", "--rules", args.rules, "--search", "mcts"]


def prewarm_input(args: argparse.Namespace, net: Path) -> str:
    """キャッシュを作るエンジンへ送るUSIのコマンド列を作る。`isready`への応答を待ってから終了する。"""
    return "".join(
        f"{line}\n"
        for line in (
            "usi",
            *(f"setoption name {name} value {value}" for name, value in engine_options(args, net)),
            "isready",
            "quit",
        )
    )


def engine_options(args: argparse.Namespace, net: Path) -> list[tuple[str, str]]:
    """進捗測定のエンジンに与えるUSIオプションを返す。"""
    return [
        ("NnBackend", args.nn_backend),
        ("NnModelPath", str(net)),
        ("NnCacheDir", str(args.nn_cache_dir)),
    ]


def progress_command(args: argparse.Namespace, layout: Layout, cycle: int) -> list[str]:
    """進捗測定の対局ハーネスのコマンドを作る。実行ディレクトリが既にあれば再開の形にする。

    ハーネスは`--resume`と実行条件の引数の併用を拒否するからである。
    """
    run_dir = layout.progress(cycle)
    base = [str(args.minase), "match", "run"]
    if run_dir.exists():
        return [*base, "--resume", str(run_dir)]
    argv = [
        *base,
        "--run-dir", str(run_dir),
        "--seed", str(progress_seed(args.seed, cycle)),
        "--rules", args.rules,
    ]
    for side, net in (
        ("candidate", layout.net(cycle)),
        ("baseline", layout.net(cycle - args.progress_every)),
    ):
        argv += [f"--{side}", f"commit:{args.commit}", f"--{side}-args", "--search mcts"]
        for name, value in engine_options(args, net):
            argv += [f"--{side}-option", f"{name}={value}"]
    return [
        *argv,
        "--each", f"nodes={args.progress_simulations}",
        "--concurrency", "2",
        "elo", "--pairs", str(args.progress_pairs),
    ]


def summarize_chunks(out_dir: Path, network_checksum: str) -> dict:
    """生成器の塊のサマリ`chunk-*.json`を足し合わせる。

    Raises:
        CycleError: 塊がないか、欄が欠けているか、推論のネットの検査和が`network_checksum`と異なる場合。
    """
    paths = sorted(out_dir.glob("chunk-*.json"))
    if not paths:
        raise CycleError(f"the generator wrote no chunk-*.json in {out_dir}")
    keys = (
        "games", "positions", "max_ply_games", "false_resign_candidates",
        "false_resignations", "total_plies",
    )
    totals = dict.fromkeys(keys, 0)
    for path in paths:
        summary = read_json(path)
        try:
            for key in keys:
                value = summary[key]
                if not isinstance(value, int) or value < 0:
                    raise CycleError(f"{path}: {key} is not a non-negative integer")
                totals[key] += value
            checksum = summary["network_checksum"]
        except (KeyError, TypeError) as error:
            raise CycleError(f"{path}: missing or malformed field {error}") from error
        if checksum != network_checksum:
            raise CycleError(
                f"{path}: network checksum {checksum} differs from the input network {network_checksum}"
            )
    candidates = totals["false_resign_candidates"]
    return {
        "chunks": len(paths),
        "network_sha256": network_checksum,
        **{key: totals[key] for key in keys if key != "total_plies"},
        "max_ply_rate": totals["max_ply_games"] / totals["games"] if totals["games"] else None,
        "false_resign_rate": totals["false_resignations"] / candidates if candidates else None,
        "average_plies": totals["total_plies"] / totals["games"] if totals["games"] else None,
    }


def install_net(run_dir: Path, destination: Path) -> str:
    """学習器の`net.onnx`を`destination`へ写し、`net.sha256`の検査和と一致することを確かめて返す。

    Raises:
        CycleError: 検査和のファイルが不正か、写したネットの検査和が一致しない場合。
    """
    try:
        expected = (run_dir / "net.sha256").read_text(encoding="utf-8").split()[0]
    except (OSError, IndexError) as error:
        raise CycleError(f"cannot read {run_dir / 'net.sha256'}: {error}") from error
    return copy_atomically(run_dir / "net.onnx", destination, expected)


def final_losses(log_path: Path) -> tuple[dict, dict | None]:
    """`log.jsonl`の最後の学習損失の行と、あれば最後の検証の行を返す。

    Raises:
        CycleError: 学習損失の行がない場合。
    """
    entries = read_jsonl(log_path)
    train = [entry for entry in entries if entry.get("kind") == "train"]
    validation = [entry for entry in entries if entry.get("kind") == "validation"]
    if not train:
        raise CycleError(f"{log_path} has no training loss")
    return train[-1], validation[-1] if validation else None


ELO_PATTERN = re.compile(r"^elo: estimate=(\S+) ci95=\[(\S+), (\S+)\]$")
"""対局ハーネスの固定局数Eloの行。"""


def parse_match_summary(text: str) -> dict:
    """対局ハーネスの標準出力の最後の`elo:`行と`engine_failures:`行を読む。

    無限大のEloは、JSONに書けるよう`"+inf"`または`"-inf"`の文字列で返す。

    Raises:
        CycleError: 行がないか、有効なペアがなくEloが求まらなかった場合。
    """
    elo_lines = [line for line in text.splitlines() if line.startswith("elo: ")]
    failure_lines = [line for line in text.splitlines() if line.startswith("engine_failures: ")]
    if not elo_lines or not failure_lines:
        raise CycleError("the match output has no elo: or engine_failures: line")
    match = ELO_PATTERN.match(elo_lines[-1])
    if match is None:
        raise CycleError(f"the match produced no Elo estimate: {elo_lines[-1]}")
    try:
        estimate, lower, upper = (json_number(float(value)) for value in match.groups())
        failures = {
            name: int(count)
            for name, count in (
                field.split("=", 1) for field in failure_lines[-1].split()[1:]
            )
        }
    except ValueError as error:
        raise CycleError(f"cannot parse the match summary: {error}") from error
    return {"elo": estimate, "ci95": [lower, upper], "engine_failures": failures}


def json_number(value: float) -> float | str:
    """有限の数はそのまま、無限大は`"+inf"`または`"-inf"`にする。`float()`で元に戻せる。"""
    if math.isfinite(value):
        return value
    return "+inf" if value > 0 else "-inf"


def progress_stalled(records: list[dict]) -> bool:
    """進捗測定のある直近2つの記録の点推定が、どちらも0以下かを返す。"""
    elos = [float(record["progress"]["elo"]) for record in records if record.get("progress")]
    return len(elos) >= 2 and all(elo <= 0 for elo in elos[-2:])


def projected_finish(
    records: list[dict], remaining: int, current: datetime,
) -> tuple[datetime, float]:
    """直近のサイクルの平均所要時間から、残りのサイクルを終える見込みの時刻と平均秒数を返す。

    完了したサイクルがなければ平均を0とする。
    """
    recent = [record["elapsed_seconds"]["total"] for record in records[-DEADLINE_HISTORY:]]
    mean = sum(recent) / len(recent) if recent else 0.0
    return datetime.fromtimestamp(current.timestamp() + mean * remaining, UTC), mean


def gpu_names() -> list[str] | None:
    """`nvidia-smi`が報告するGPUの名前を返す。得られなければ`None`とする。"""
    try:
        completed = subprocess.run(
            ["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"],
            capture_output=True, text=True, check=False,
        )
    except OSError:
        return None
    if completed.returncode != 0:
        return None
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


def timed(action: Callable[[], None]) -> float:
    """`action`を実行し、経過秒数を返す。"""
    started = time.monotonic()
    action()
    return time.monotonic() - started


def warn(message: str) -> None:
    """警告を標準エラーへ書く。"""
    print(f"warning: {message}", file=sys.stderr, flush=True)


def run_selfplay(args: argparse.Namespace, layout: Layout, cycle: int, runner: Runner) -> dict:
    """生成の段階を実行し、塊のサマリの集計を返す。"""
    argv = selfplay_command(args, layout, cycle)
    elapsed = timed(lambda: runner(argv, layout.log(cycle, "selfplay"), None))
    summary = summarize_chunks(layout.selfplay(cycle), sha256_file(layout.net(cycle - 1)))
    if summary["false_resign_rate"] is not None and summary["false_resign_rate"] > FALSE_RESIGN_WARNING:
        warn(
            f"cycle {cycle}: false resignation rate {summary['false_resign_rate']:.1%} exceeds "
            f"{FALSE_RESIGN_WARNING:.0%}; review the resignation threshold"
        )
    if summary["max_ply_rate"] is not None and summary["max_ply_rate"] > MAX_PLY_WARNING:
        warn(
            f"cycle {cycle}: {summary['max_ply_rate']:.1%} of the games reached the move limit, "
            f"more than {MAX_PLY_WARNING:.0%}; review --max-ply"
        )
    return {**summary, "elapsed_seconds": elapsed}


def run_train(args: argparse.Namespace, layout: Layout, cycle: int, runner: Runner) -> dict:
    """学習の段階を実行し、ネットを`nets/`へ写して、損失と来歴を返す。"""
    width = args.window_schedule.width(cycle)
    argv = train_command(args, layout, cycle, width)
    run_dir = layout.train(cycle)
    elapsed = timed(lambda: runner(argv, layout.log(cycle, "train"), None))
    checksum = install_net(run_dir, layout.net(cycle))
    train, validation = final_losses(run_dir / "log.jsonl")
    provenance = read_json(run_dir / "provenance.json")
    try:
        data, init, config = provenance["data"], provenance["init"], provenance["config"]
    except (KeyError, TypeError) as error:
        raise CycleError(f"{run_dir / 'provenance.json'}: missing field {error}") from error
    return {
        "elapsed_seconds": elapsed,
        "window": width,
        "window_cycles": list(window_cycles(cycle, width)),
        "net_sha256": checksum,
        "final_train": train,
        "final_validation": validation,
        "data": data,
        "init": init,
        "config": config,
    }


def run_progress(args: argparse.Namespace, layout: Layout, cycle: int, runner: Runner) -> dict:
    """進捗測定の段階を実行し、Eloと異常件数を返す。

    対局の前に両方のネットでエンジンを1回起動し、TensorRTのエンジンのキャッシュを作っておく。
    対局ハーネスの応答タイムアウトの間にエンジンの作成が終わらず、反則負けになるのを避けるためである。
    """
    baseline = cycle - args.progress_every
    log = layout.log(cycle, "progress")
    nets = (layout.net(cycle), layout.net(baseline))
    argv = progress_command(args, layout, cycle)

    def measure() -> None:
        for net in nets:
            runner(prewarm_command(args), layout.log(cycle, "prewarm"), prewarm_input(args, net))
        runner(argv, log, None)

    elapsed = timed(measure)
    result = parse_match_summary(log.read_text(encoding="utf-8"))
    if any(result["engine_failures"].values()):
        warn(f"cycle {cycle}: the progress match had engine failures {result['engine_failures']}")
    return {
        "elapsed_seconds": elapsed,
        "baseline_cycle": baseline,
        "seed": progress_seed(args.seed, cycle),
        **result,
    }


def build_record(args: argparse.Namespace, cycle: dict) -> dict:
    """完了したサイクルの記録の1行を作る。"""
    number, stages = cycle["number"], cycle["stages"]
    selfplay, train, progress = stages["selfplay"], stages["train"], stages.get("progress")
    elapsed = {
        "selfplay": selfplay["elapsed_seconds"],
        "train": train["elapsed_seconds"],
        "progress": progress["elapsed_seconds"] if progress else None,
    }
    elapsed["total"] = sum(value for value in elapsed.values() if value is not None)
    return {
        "cycle": number,
        "started_at": cycle["started_at"],
        "finished_at": now(),
        "network": {"input_sha256": selfplay["network_sha256"], "sha256": train["net_sha256"]},
        "selfplay": {
            key: selfplay[key]
            for key in (
                "chunks", "games", "positions", "max_ply_games", "max_ply_rate",
                "false_resign_candidates", "false_resignations", "false_resign_rate",
                "average_plies",
            )
        },
        "train": {
            "window": train["window"],
            "window_cycles": train["window_cycles"],
            "final_train": train["final_train"],
            "final_validation": train["final_validation"],
        },
        "progress": (
            {key: progress[key] for key in ("baseline_cycle", "elo", "ci95", "engine_failures")}
            if progress
            else None
        ),
        "elapsed_seconds": elapsed,
        "machine": {"node": platform.node(), "gpus": gpu_names()},
        "provenance": {
            "generation_seed": generation_seed(args.seed, number),
            "progress_seed": progress["seed"] if progress else None,
            "commit": args.commit,
            "image_digest": args.image_digest,
            "train_config": train["config"],
            "init": train["init"],
            "window_data": train["data"],
        },
    }


def append_record(layout: Layout, record: dict) -> None:
    """記録の1行を追記する。最後の行が同じサイクルなら、状態の保存前に中断した再実行なので書かない。"""
    records = read_jsonl(layout.record)
    if records and records[-1].get("cycle") == record["cycle"]:
        return
    with layout.record.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(record) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def run(args: argparse.Namespace, runner: Runner = run_command) -> int:
    """サイクルを`--cycles`まで、または停止条件まで繰り返し、終了コードを返す。

    Raises:
        CycleError: 外部コマンドの失敗、または出力や状態ファイルの不正。
    """
    layout = Layout(args.root.resolve())
    state = prepare(layout, args)
    while True:
        if state["cycle"] is None:
            number = state["completed_cycles"] + 1
            if number > args.cycles:
                print(f"completed {state['completed_cycles']} cycles", flush=True)
                return EXIT_COMPLETED
            remaining = args.cycles - state["completed_cycles"]
            finish, mean = projected_finish(read_jsonl(layout.record), remaining, datetime.now(UTC))
            if finish > args.deadline:
                print(
                    f"not starting cycle {number}: {remaining} remaining cycles at {mean:.0f} s each "
                    f"would finish at {finish.isoformat(timespec='seconds')}, "
                    f"after the deadline {args.deadline.isoformat()}",
                    flush=True,
                )
                return EXIT_DEADLINE
            state["cycle"] = {"number": number, "started_at": now(), "stages": {}}
            save_state(layout, state)
        cycle = state["cycle"]
        number, stages = cycle["number"], cycle["stages"]
        print(f"cycle {number}", flush=True)
        steps = [("selfplay", run_selfplay), ("train", run_train)]
        if number % args.progress_every == 0:
            steps.append(("progress", run_progress))
        for name, stage in steps:
            if name not in stages:
                stages[name] = stage(args, layout, number, runner)
                save_state(layout, state)
        append_record(layout, build_record(args, cycle))
        state["completed_cycles"] = number
        state["cycle"] = None
        save_state(layout, state)
        if "progress" in stages:
            print(f"cycle {number}: progress Elo {stages['progress']['elo']}", flush=True)
            if progress_stalled(read_jsonl(layout.record)):
                print(
                    "stopping: the last two progress measurements were 0 Elo or less; "
                    "investigate before resuming",
                    flush=True,
                )
                return EXIT_PROGRESS_STOP


def parse_deadline(text: str) -> datetime:
    """ISO 8601の日時を読む。時差がなければ実行機の地方時とする。"""
    try:
        value = datetime.fromisoformat(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"'{text}' is not an ISO 8601 date and time") from None
    return value if value.tzinfo is not None else value.astimezone()


def parse_schedule(text: str) -> WindowSchedule:
    """`--window-schedule`を読む。"""
    try:
        return WindowSchedule.parse(text)
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from None


def positive_int(text: str) -> int:
    """正の整数の引数を読む。"""
    try:
        return _positive(text)
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from None


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    """コマンドライン引数を読む。"""
    parser = argparse.ArgumentParser(description="強化学習のサイクルを繰り返す。")
    required = parser.add_argument_group("必須の引数")
    required.add_argument("--root", type=Path, required=True, help="サイクルの全ファイルを置くディレクトリ")
    required.add_argument("--minase", type=Path, required=True, help="生成と進捗測定に使うminaseの実行ファイル")
    required.add_argument("--commit", required=True, help="進捗測定で両エンジンをビルドするコミット")
    required.add_argument("--rules", required=True, help="規則セット名またはコード列")
    required.add_argument("--net", type=Path, required=True, help="最初のサイクルの生成に使う初期ネット（ONNX）")
    required.add_argument(
        "--init-checkpoint", type=Path, required=True,
        help="最初のサイクルの学習の`--init`に使うチェックポイント（SLのcheckpoint.pt）",
    )
    required.add_argument("--games", type=positive_int, required=True, help="1サイクルで生成する対局数")
    required.add_argument("--nn-cache-dir", type=Path, required=True, help="TensorRTのエンジンのキャッシュ")
    required.add_argument("--lr", type=float, required=True, help="各サイクルの学習の学習率の初期値")
    required.add_argument(
        "--window-schedule", type=parse_schedule, required=True,
        help="学習窓の幅の段階表。1:10,2:30,4 はサイクル1〜10で1、11〜30で2、以後4",
    )
    required.add_argument("--seed", type=int, required=True, help="生成、学習、および進捗測定の基本シード")
    required.add_argument("--cycles", type=positive_int, required=True, help="実行するサイクルの総数")
    required.add_argument(
        "--deadline", type=parse_deadline, required=True,
        help="RLの期間の上限（ISO 8601）。超える見込みなら次のサイクルを始めない",
    )
    parser.add_argument("--simulations", type=positive_int, default=32, help="生成の1手の探索回数")
    parser.add_argument("--max-ply", type=positive_int, default=1000, help="生成の1局の手数の上限")
    parser.add_argument("--games-in-flight", type=positive_int, default=256, help="生成で同時に進める対局数")
    parser.add_argument("--threads", type=positive_int, default=2, help="生成のスレッド数")
    parser.add_argument("--nn-batch-size", type=positive_int, default=128, help="生成の推論のバッチの上限")
    parser.add_argument("--nn-backend", default="tensorrt", help="ortの実行プロバイダ")
    parser.add_argument("--blocks", type=positive_int, default=10, help="ネットの残差ブロックの数")
    parser.add_argument("--channels", type=positive_int, default=128, help="ネットのチャネル数")
    parser.add_argument("--batch-size", type=positive_int, default=1024, help="学習のバッチの大きさ")
    parser.add_argument(
        "--lambda", dest="value_lambda", type=float, default=0.333, help="価値の教師の根の価値の重み"
    )
    parser.add_argument("--workers", type=int, default=4, help="学習のデータ読み込みのワーカーの数")
    parser.add_argument("--device", default="cuda", help="学習に使う装置")
    parser.add_argument("--progress-every", type=positive_int, default=10, help="進捗測定を行うサイクルの間隔")
    parser.add_argument("--progress-pairs", type=positive_int, default=200, help="進捗測定のペア数")
    parser.add_argument(
        "--progress-simulations", type=positive_int, default=800, help="進捗測定の1手の探索回数"
    )
    parser.add_argument("--image-digest", help="記録に残すコンテナイメージの識別子")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    """コマンドラインの入口。"""
    args = parse_args(argv)
    try:
        code = run(args)
    except CycleError as error:
        sys.exit(f"error: {error}")
    sys.exit(code)


if __name__ == "__main__":
    main()
