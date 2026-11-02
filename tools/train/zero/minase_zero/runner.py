"""Colab機で自己対局、学習、出力の照合を順に行い、確定したファイルをNASへ受け取る。

run-cyclesの引数はすべて明示する。cycle1-reference-summaryだけは開始サイクルが1の
ときに必須で、それ以外では指定できない。deadlineは時差を含むISO 8601の時刻とする。
generation-commitとtrainer-commitはそれぞれ40桁の16進数字で指定する。Colab機の
/content/srcにはtrainer-commitの書庫を展開し、導入とビルドを済ませておく。
--minaseは作業機で集計に使う実行ファイルを指定し、借りた機械で使うENGINEとは別である。
NASのrootと/content/azは同じ相対パスを使い、接続と転送は1本ずつ実行する。

--cycles-dirと--base-cycles-dirはrootからの相対ディレクトリを必須とする。
cycles-dirはシード台帳の名前にも使うため、空白文字を含められない。
--branch-from以下のサイクルはbase-cycles-dirから、それより後はcycles-dirから読む。
first-cycleはbranch-fromより後とし、生成、学習、照合の出力はcycles-dirに置く。
本線は--cycles-dir cycles --base-cycles-dir cycles --branch-from 0で実行する。
cycles-dirがcycles以外なら、記録は<cycles-dir>.jsonlと<cycles-dir>-sessions.jsonl、
確定したネットはnets/<cycles-dir>/cycle-NNNN.onnxに置く。
通常生成のシードはサイクル番号×10⁹を基本とし、別系列では7×10⁸を加える。
再生成は通常生成のシードに5×10⁸を加える。共有のシード台帳には、本線ではcycle-NNNN-gen、
別系列では<cycles-dir>-NNNN-genの名前で登録し、再生成では末尾をregenにする。

通常生成は--target-positionsを生成器へ渡し、前のサイクルの1局あたりの訓練局面数から
見積もった対局数を上限とする。
塊の配置は対局数の上限と塊の対局数で固定し、塊0から連続する確定済み塊の
positions + resigned_positionsが初めて目標以上になる塊までをサイクルに含める。
全塊でも目標に届かない場合は全塊を含める。
誤った投了による生成し直しでは、本体のresigned_positionsを目標、resigned_gamesを
対局数の上限とし、投了を無効にする。
目標は各対局の生成条件を変えないためconfig.jsonには保存せず、summary.jsonと
サイクルのgenerationおよびregenerationに記録する。
summary.jsonのchunksは含めた塊の数であり、その範囲の3種類のファイルをすべて検査する。
範囲より後の確定済み塊も転送するが、学習にはsummary.jsonのtraining_filesだけを使う。

学習窓は最新のサイクルからさかのぼり、生成し直しを含むtraining_positionsの合計が
初めてwindow-positions以上になるまでとする。
--window-first-cycleは1以上、first-cycle以下の整数を必須とし、別系列でも同じ通し番号を使う。
合計が目標に届かない場合もwindow-first-cycleでさかのぼりを止め、それより前の集計は読まない。
ファイルは古いサイクルから、各生成のtraining_filesの順に並べ、生成し直しを本体の
後に加える。窓全体を--data、最新のサイクルだけを--latest-dataとして学習器へ渡す。
窓のサイクル番号、window-first-cycle、window-positions、生成と学習のコミットをサイクルの記録に残す。
NASにsummary.jsonがある生成はconfig.jsonのgeneration_commitを受け入れる。
未完了の生成はgeneration-commitが一致する場合だけ再開する。

自己対局は前のサイクルのtrain/model.onnxを使う。最初のサイクルではcycle-0000/model.onnxを
参照し、ネットがなければエラーにする。学習後のネットはCPUとTensorRTのバッチ1、7、256の
6通りで照合し、本線ではnets/cycle-NNNN.onnxへ読み取り専用で確定する。
記録には各照合の結果を残す。

送信ではColab機にないファイルを一括で調べ、1回のrsync --files-fromで送り、既存分も
含む全ファイルを1回のsha256sumで照合する。生成の塊の復元も同じ経路を使う。
一括の転送と照合の制限時間は、送信バイト数を1 MB/秒（1 MB = 10⁶ B）で割った秒数に
120秒を足す。ほかの外部コマンドは120秒とする。

塊はJSONを最後に、生成と学習はsummary.jsonを最後に受け取る。SHA-256を照合してから
一時ファイルを改名し、既存ファイルの相違はエラーにする。置き換えを許すのは学習中の
checkpoint.ptと追記する台帳だけである。NASで受信済みの工程は機械を替えても再実行しない。
シードの台帳は既存のものを使い、見つからなければエラーにする。checkpoint.ptはColab機で
一時の名前へ複写し、その版の検査和を求めて受け取る。複写は受信の成否にかかわらず削除する。
工程のstarted.jsonとfinished.jsonは作業機での初回の監視開始と受信完了を保存するので、
再開を挟んだ経過秒数には停止中の時間も含む。Colab機の照合自体の秒数は別に記録する。

PIDが残っていて終了コードがなく、プロセスもいなければ異常終了として止める。
接続の終了コード255とHTTP 429だけは30秒間隔で最大5回再試行する。
期限またはCU残高が不足すれば正常終了し、セッションを止める操作は行わない。
"""

import argparse
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

from . import rl


SOURCE = "/content/src/tools/train/zero"
FIXTURE = "/content/src/crates/minase/tests/fixtures/nn-positions.jsonl"
ENGINE = "/content/src/target/release/minase"
PYTHON = ["uv", "run", "--frozen", "--project", SOURCE, "python"]
TRAINER = [*PYTHON, "-m", "minase_zero"]


@dataclass(frozen=True)
class Options:
    root: Path
    cycles_dir: Path
    base_cycles_dir: Path
    branch_from: int
    session: str
    generation_commit: str
    trainer_commit: str
    first_cycle: int
    last_cycle: int
    deadline: datetime
    cycle_minutes: float
    cycle_cu: float
    target_positions: int
    chunks: int
    simulations: int
    threads: int
    games_per_thread: int
    window_first_cycle: int
    window_positions: int
    batch_size: int
    lr: float
    momentum: float
    weight_decay: float
    clip_grad_norm: float
    value_lambda: float
    precision: str
    checkpoint_steps: int
    train_seed_base: int
    minase: Path
    poll_seconds: float
    cycle1_reference_summary: Path | None

    def __post_init__(self) -> None:
        for name in ("cycles_dir", "base_cycles_dir"):
            path = getattr(self, name)
            if path.is_absolute() or not path.parts or ".." in path.parts:
                raise ValueError(f"{name} must be a relative directory without '..'")
        if any(character.isspace() for character in str(self.cycles_dir)):
            raise ValueError("cycles_dir must contain no whitespace because it names seed-ledger entries")
        if type(self.branch_from) is not int or not 0 <= self.branch_from < self.first_cycle:
            raise ValueError("branch-from must be nonnegative and less than first-cycle")
        if re.fullmatch(r"[A-Za-z0-9_-]+", self.session) is None:
            raise ValueError("session must contain only letters, digits, underscores or hyphens")
        for name in ("generation_commit", "trainer_commit"):
            if re.fullmatch(r"[0-9a-fA-F]{40}", getattr(self, name)) is None:
                raise ValueError(f"{name} must be 40 hexadecimal digits")
        for name in ("first_cycle", "last_cycle", "target_positions", "chunks", "simulations",
                     "threads", "games_per_thread", "window_first_cycle", "window_positions",
                     "batch_size", "checkpoint_steps"):
            if type(getattr(self, name)) is not int or getattr(self, name) < 1:
                raise ValueError(f"{name} must be a positive integer")
        if self.last_cycle < self.first_cycle:
            raise ValueError("last_cycle must be at least first_cycle")
        if self.window_first_cycle > self.first_cycle:
            raise ValueError("window_first_cycle must be at most first_cycle")
        if (self.first_cycle == 1) != (self.cycle1_reference_summary is not None):
            raise ValueError("cycle1-reference-summary is required exactly when first-cycle is 1")
        if self.deadline.utcoffset() is None:
            raise ValueError("deadline must include a UTC offset")
        for name in ("cycle_minutes", "cycle_cu", "lr", "momentum", "clip_grad_norm", "poll_seconds"):
            value = getattr(self, name)
            if not math.isfinite(value) or value <= 0:
                raise ValueError(f"{name} must be finite and positive")
        if not math.isfinite(self.weight_decay) or self.weight_decay < 0:
            raise ValueError("weight_decay must be finite and nonnegative")
        if not math.isfinite(self.value_lambda) or not 0 <= self.value_lambda <= 1:
            raise ValueError("value_lambda must be in [0, 1]")
        if self.precision not in ("bf16", "fp32"):
            raise ValueError("precision must be bf16 or fp32")
        if (type(self.train_seed_base) is not int or self.train_seed_base < 0
                or self.train_seed_base + self.last_cycle >= 2**64):
            raise ValueError("training seeds must fit uint64")

    def cycle_path(self, cycle: int) -> str:
        """分岐番号だけで参照先を決め、ファイルの有無では切り替えない。"""
        directory = self.base_cycles_dir if cycle <= self.branch_from else self.cycles_dir
        return (directory / f"cycle-{cycle:04d}").as_posix()

    def network_path(self, cycle: int) -> str:
        """系列に対応する確定ネットの相対パスを返す。"""
        directory = Path("nets") if self.cycles_dir == Path("cycles") else Path("nets") / self.cycles_dir
        return (directory / f"cycle-{cycle:04d}.onnx").as_posix()


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """run-cyclesだけの必須引数を登録する。"""
    for name in ("root", "minase", "cycles-dir", "base-cycles-dir"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    for name in ("session", "generation-commit", "trainer-commit"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--deadline", type=datetime.fromisoformat, required=True)
    for name in ("first-cycle", "last-cycle", "target-positions", "chunks", "simulations", "threads",
                 "games-per-thread", "window-first-cycle", "window-positions", "batch-size", "checkpoint-steps",
                 "train-seed-base", "branch-from"):
        parser.add_argument(f"--{name}", type=int, required=True)
    for name in ("cycle-minutes", "cycle-cu", "lr", "momentum", "weight-decay", "clip-grad-norm",
                 "value-lambda", "poll-seconds"):
        parser.add_argument(f"--{name}", type=float, required=True)
    parser.add_argument("--precision", choices=("bf16", "fp32"), required=True)
    parser.add_argument("--cycle1-reference-summary", type=Path)


def read_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected a JSON object")
    return value


def summaries(options: Options, cycle: int) -> list[Path]:
    paths = [options.root / options.cycle_path(cycle) / "selfplay/summary.json"]
    regen = options.root / options.cycle_path(cycle) / "selfplay-regen/summary.json"
    if regen.exists():
        paths.append(regen)
    return paths


def game_counts(paths: list[Path], target_positions: int, chunks: int) -> tuple[int, int]:
    """上限到達で捨てた対局も分母に含め、必要な対局数と塊の対局数を切り上げる。"""
    games, positions = 0, 0
    for path in paths:
        summary = read_json(path)
        for field in ("games", "training_positions"):
            if type(summary[field]) is not int or summary[field] < 0:
                raise ValueError(f"{path}: invalid {field}")
        games += summary["games"]
        positions += summary["training_positions"]
    if min(games, positions, target_positions, chunks) <= 0:
        raise ValueError("games, training positions, target positions and chunks must be positive")
    count = (target_positions * games + positions - 1) // positions
    return count, (count + chunks - 1) // chunks


def resignation(options: Options, cycle: int) -> tuple[int, dict]:
    if cycle == 1:
        calibration = {"n": 0, "false_resignations": 0, "upper_bound": None,
                       "enable_resignation": False}
    else:
        calibration = rl.resign_calibration(summaries(options, cycle - 1))
    return (20 if calibration["enable_resignation"] else 100), calibration


def training_window(options: Options, cycle: int, window_positions: int) -> tuple[list[int], list[str], list[str]]:
    """局面数から窓を決め、サイクル番号、窓のファイル、最新のファイルを返す。"""
    by_cycle = []
    positions = 0
    for previous in range(cycle, options.window_first_cycle - 1, -1):
        current = []
        for path in summaries(options, previous):
            summary = read_json(path)
            count = summary["training_positions"]
            if type(count) is not int or count < 0:
                raise ValueError(f"{path}: invalid training_positions")
            positions += count
            names = summary["training_files"]
            if not isinstance(names, list) or not names:
                raise ValueError(f"{path}: training_files must be a nonempty list")
            for name in names:
                if not isinstance(name, str) or re.fullmatch(r"chunk-[0-9]+(?:\.resigned)?\.mnsd", name) is None:
                    raise ValueError(f"{path}: invalid training file: {name!r}")
                current.append((path.parent / name).relative_to(options.root).as_posix())
        by_cycle.append(current)
        if positions >= window_positions:
            break
    files = [name for current in reversed(by_cycle) for name in current]
    if not files or len(set(files)) != len(files):
        raise ValueError("training window must contain distinct files")
    return list(range(cycle - len(by_cycle) + 1, cycle + 1)), files, by_cycle[0]


def parse_balance(output: str) -> float:
    matches = re.findall(r"^\s*Current balance:\s*([0-9]+(?:\.[0-9]+)?)\s+compute units\s*$",
                         output, re.MULTILINE)
    if len(matches) != 1 or not math.isfinite(float(matches[0])):
        raise ValueError("colab usage: expected one finite Current balance in compute units")
    return float(matches[0])


def can_start(now: datetime, deadline: datetime, minutes: float, balance: float, cu: float) -> bool:
    return now + timedelta(minutes=minutes) <= deadline and balance >= cu


def sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def _sync_replace(temporary: Path, path: Path) -> None:
    with temporary.open("rb") as source:
        os.fsync(source.fileno())
    os.replace(temporary, path)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _write(path: Path, contents: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(contents)
        _sync_replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def _json_bytes(value: dict) -> bytes:
    return (json.dumps(value, ensure_ascii=False, allow_nan=False) + "\n").encode("utf-8")


def read_rows(path: Path) -> list[dict]:
    if not path.exists():
        return []
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]
    if any(not isinstance(row, dict) for row in rows):
        raise ValueError(f"{path}: expected JSON objects")
    return rows


def append_cycle(path: Path, row: dict) -> None:
    """同じサイクルの二重登録を拒否し、全体を一時ファイルから置き換える。"""
    rows = read_rows(path)
    if any(previous["cycle"] == row["cycle"] for previous in rows):
        raise ValueError(f"{path}: duplicate cycle {row['cycle']}")
    contents = path.read_bytes() if path.exists() else b""
    if contents and not contents.endswith(b"\n"):
        contents += b"\n"
    _write(path, contents + _json_bytes(row))


def seed_registration(ledger: Path, name: str, seed: int, games: int) -> dict:
    entries = read_rows(ledger)
    names = [entry["name"] for entry in entries]
    if len(set(names)) != len(names):
        raise ValueError(f"{ledger}: duplicate seed names")
    for entry in entries:
        if entry["name"] == name:
            if (entry["seed"], entry["games"], entry["low"], entry["high"]) != (
                seed, games, seed + 1, seed + games,
            ):
                raise ValueError(f"{ledger}: seed or games differ for {name}")
            return entry
    return rl.register_seed_range(ledger, name, seed, games)


class Remote:
    """ssh、rsync、残高の照会を逐次実行し、接続障害だけを有限回再試行する。"""

    def __init__(self, session: str):
        if re.fullmatch(r"[A-Za-z0-9_-]+", session) is None:
            raise ValueError("invalid session name")
        self.ssh = ["ssh", "-o", f"ProxyCommand=colab ssh --proxy-mode -s {session}",
                    "-o", "ControlMaster=auto", "-o", f"ControlPath=/tmp/colab-cm-{session}",
                    "-o", "ControlPersist=900", "-o", "StrictHostKeyChecking=no",
                    "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR"]

    def _call(self, argv: list[str], timeout: float) -> str:
        for attempt in range(6):
            try:
                result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
            except subprocess.TimeoutExpired as error:
                raise RuntimeError(f"{argv[0]} timed out after {timeout} seconds") from error
            if result.returncode == 0:
                return result.stdout
            detail = result.stderr + result.stdout
            transient = (result.returncode == 255
                         or re.search(r"\bHTTP(?:\s+Error|/[0-9.]+)?\s*:?\s*429\b", detail) is not None
                         or (argv[0] == "rsync" and re.search(r"\b(?:code|status) 255\b", detail) is not None))
            if not transient or attempt == 5:
                raise RuntimeError(f"{argv[0]} exited {result.returncode}: {detail.strip()}")
            print(f"{datetime.now(UTC).isoformat()} connection retry={attempt + 1}/5", file=sys.stderr)
            time.sleep(30)
        raise AssertionError("retry loop did not return")

    def shell(self, script: str, timeout: float) -> str:
        command = shlex.join(["bash", "-c", "set -e\ncd /content/az\n" + script])
        return self._call([*self.ssh, "root@colab", command], timeout)

    def exists(self, relative: str) -> bool:
        return self.shell(f"if test -f {shlex.quote(relative)}; then echo yes; else echo no; fi", 120).strip() == "yes"

    def missing(self, files: list[str]) -> list[str]:
        script = ("import json,pathlib; print(json.dumps([name for name in " + json.dumps(files)
                  + " if not pathlib.Path(name).is_file()]))")
        return json.loads(self.shell(shlex.join(["python3", "-c", script]), 120))

    def files(self, relative: str) -> list[str]:
        script = ("import json,pathlib; p=pathlib.Path(" + repr(relative) + "); "
                  "print(json.dumps(sorted(x.name for x in p.iterdir() if x.is_file()) if p.exists() else []))")
        return json.loads(self.shell(shlex.join(["python3", "-c", script]), 120))

    def sha256(self, relative: str) -> str:
        digest = self.shell(shlex.join(["sha256sum", "--", relative]), 120).split()[0]
        if re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise ValueError(f"invalid remote SHA-256: {relative}")
        return digest

    def sha256_many(self, files: list[str], timeout: float) -> dict[str, str]:
        output = self.shell(shlex.join(["sha256sum", "--zero", "--", *files]), timeout)
        records = output.split("\0")
        if records.pop() != "":
            raise ValueError("incomplete remote SHA-256 output")
        hashes = {}
        for record in records:
            match = re.fullmatch(r"([0-9a-f]{64}) [ *](.+)", record, re.DOTALL)
            if match is None or match[2] in hashes:
                raise ValueError("invalid remote SHA-256 output")
            hashes[match[2]] = match[1]
        if set(hashes) != set(files):
            raise ValueError("remote SHA-256 files differ from requested files")
        return hashes

    def upload(self, root: Path, files: list[str], timeout: float) -> None:
        """相対パスの列を1回のrsyncで送り、作業ディレクトリからの配置を保つ。"""
        with tempfile.NamedTemporaryFile() as manifest:
            manifest.write(("\0".join(files) + "\0").encode("utf-8"))
            manifest.flush()
            self._call(["rsync", "-a", "-s", "--from0", "--files-from", manifest.name,
                        "-e", shlex.join(self.ssh), "--", str(root) + "/", "root@colab:/content/az/"], timeout)

    def download(self, relative: str, target: Path) -> None:
        self._call(["rsync", "-a", "-s", "-e", shlex.join(self.ssh), "--",
                    f"root@colab:/content/az/{relative}", str(target)], 120)

    def copy(self, source: str, target: str) -> None:
        """開いたファイルを複写し、改名による置き換えの影響を受けない版を作る。"""
        self.shell(shlex.join(["cp", "--", source, target]), 120)

    def remove(self, relative: str) -> None:
        self.shell(shlex.join(["rm", "-f", "--", relative]), 120)

    def usage(self) -> float:
        return parse_balance(self._call(["colab", "usage"], 120))

    def machine(self) -> dict:
        script = """import json,pathlib,subprocess
gpu = subprocess.check_output(['nvidia-smi', '--query-gpu=name', '--format=csv,noheader'], text=True).strip()
cpu = next(line.split(':', 1)[1].strip() for line in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name'))
count = int(subprocess.check_output(['nproc'], text=True))
if not gpu or not cpu or count < 1:
    raise ValueError('incomplete machine information')
print(json.dumps({'gpu': gpu, 'cpu': cpu, 'nproc': count}))"""
        return json.loads(self.shell(shlex.join(["python3", "-c", script]), 120))

    @staticmethod
    def _status_script(stem: str) -> str:
        pid, code = shlex.quote(stem + ".pid"), shlex.quote(stem + ".exit")
        return f"""if test -f {code}; then cat {code}
elif test -f {pid}; then
    pid=$(cat {pid})
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] || exit 1
    if kill -0 "$pid" 2>/dev/null && [[ $(ps -p "$pid" -o stat=) != Z* ]]; then
        echo running
    else
        echo {shlex.quote('process disappeared without an exit code: ' + stem)} >&2
        exit 1
    fi
else echo new
fi"""

    def status(self, stem: str) -> str | int:
        state = self.shell(self._status_script(stem), 120).strip()
        if state in ("new", "running"):
            return state
        if re.fullmatch(r"[0-9]+", state) is None or int(state) > 255:
            raise ValueError(f"invalid process status for {stem}: {state!r}")
        return int(state)

    def start(self, stem: str, body: str, stdout: str, stderr: str) -> None:
        # 再試行でもPIDと終了コードを確かめ、受信だけが失敗した起動を繰り返さない。
        code = shlex.quote(stem + ".exit")
        temporary = shlex.quote(stem + ".exit.tmp")
        job = ("(\nset -euo pipefail\nsource " + SOURCE + "/env.sh\n"
               f"export XDG_CACHE_HOME=/content/cache PYTHONPATH={SOURCE}\n{body}\n)\n"
               f"result=$?\nprintf '%s\\n' \"$result\" > {temporary}\nmv -- {temporary} {code}\n")
        redirect = (f"> {shlex.quote(stdout)} 2>&1" if stdout == stderr else
                    f"> {shlex.quote(stdout)} 2> {shlex.quote(stderr)}")
        script = (f"mkdir -p -- {shlex.quote(str(Path(stem).parent))}\n"
                  f"state=$(\n{self._status_script(stem)}\n) || exit 1\n"
                  'if [[ "$state" == new ]]; then\n'
                  f"setsid nohup bash -c {shlex.quote(job)} {redirect} < /dev/null &\n"
                  f"echo $! > {shlex.quote(stem + '.pid')}\nfi")
        self.shell(script, 120)


def receive(remote: Remote, root: Path, relative: str, replace: bool) -> bool:
    """検査和の一致した一時ファイルだけを確定する。既存の異なる内容は保持する。"""
    path = root / relative
    source = str(Path(relative).with_name(f".{path.name}.snapshot")) if replace else relative
    try:
        if replace:
            remote.copy(relative, source)
        expected = remote.sha256(source)
        if path.exists():
            if sha256(path) == expected:
                return False
            if not replace:
                raise ValueError(f"immutable file differs: {relative}")
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(f".{path.name}.tmp")
        try:
            remote.download(source, temporary)
            if sha256(temporary) != expected:
                raise ValueError(f"checksum mismatch: {relative}")
            _sync_replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)
        return True
    finally:
        if replace:
            remote.remove(source)


def send(remote: Remote, root: Path, files: list[str]) -> None:
    """不足するファイルだけを一括送信し、既存分も含めて全件の検査和を照合する。"""
    expected = {name: sha256(root / name) for name in files}
    missing = remote.missing(files)
    timeout = 120 + sum((root / name).stat().st_size for name in missing) / 1_000_000
    if missing:
        remote.upload(root, missing, timeout)
    actual = remote.sha256_many(files, timeout)
    for name in files:
        if actual[name] != expected[name]:
            raise ValueError(f"checksum mismatch: {name}")


def chunk_files(names: list[str]) -> list[str]:
    """確定印のある塊を番号順に並べ、各塊ではJSONを最後にする。"""
    files = []
    for name in sorted((n for n in names if re.fullmatch(r"chunk-[0-9]+\.json", n)),
                       key=lambda n: int(n[6:-5])):
        stem = name[:-5]
        chunk = [stem + ".mnsd", stem + ".resigned.mnsd", name]
        if any(part not in names for part in chunk):
            raise ValueError(f"incomplete committed chunk: {stem}")
        files.extend(chunk)
    return files


def publish_network(options: Options, cycle: int) -> str:
    source = options.root / options.cycle_path(cycle) / "train/model.onnx"
    target = options.root / options.network_path(cycle)
    digest = sha256(source)
    if target.exists():
        if sha256(target) != digest:
            raise ValueError(f"immutable network differs: {target}")
        return digest
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(f".{target.name}.tmp")
    try:
        shutil.copyfile(source, temporary)
        if sha256(temporary) != digest:
            raise ValueError(f"checksum mismatch: {target}")
        temporary.chmod(0o444)
        _sync_replace(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)
    return digest


class Runner:
    """確定済みのNASのファイルとColab機のプロセス状態から、サイクルの続きを決める。"""

    def __init__(self, options: Options, remote: Remote):
        self.options = options
        self.root = options.root
        self.remote = remote

    def log(self, cycle: int, message: str) -> None:
        print(f"{datetime.now(UTC).isoformat()} cycle={cycle:04d} {message}", file=sys.stderr, flush=True)

    def _begin(self, stem: str) -> None:
        path = self.root / (stem + ".started.json")
        if not path.exists():
            _write(path, _json_bytes({"started_at": datetime.now(UTC).isoformat()}))

    def _finish(self, stem: str) -> float:
        path = self.root / (stem + ".finished.json")
        if not path.exists():
            started = datetime.fromisoformat(read_json(self.root / (stem + ".started.json"))["started_at"])
            ended = datetime.now(UTC)
            _write(path, _json_bytes({"finished_at": ended.isoformat(),
                                     "elapsed_seconds": (ended - started).total_seconds()}))
        return read_json(path)["elapsed_seconds"]

    def _monitor(self, cycle: int, stem: str, progress) -> int:
        while True:
            state = self.remote.status(stem)
            count = progress()
            self.log(cycle, f"{Path(stem).name} status={state} received={count}")
            if isinstance(state, int):
                return state
            if state != "running":
                raise RuntimeError(f"job was not started: {stem}")
            time.sleep(self.options.poll_seconds)

    def _pull_chunks(self, out: str) -> int:
        names = self.remote.files(out)
        files = (["config.json"] if "config.json" in names else []) + chunk_files(names)
        return sum(receive(self.remote, self.root, f"{out}/{name}", False)
                   for name in files if not (self.root / out / name).exists())

    def restore_generation(self, out: str) -> None:
        directory = self.root / out
        if (directory / "config.json").exists():
            names = ["config.json", *chunk_files([path.name for path in directory.iterdir() if path.is_file()])]
            send(self.remote, self.root, [f"{out}/{name}" for name in names])

    def generation(self, cycle: int, out: str, network: str, seed: int, games: int,
                   chunk_games: int, playthrough: int, target_positions: int) -> tuple[dict, float]:
        local_summary = self.root / out / "summary.json"
        config_path = local_summary.parent / "config.json"
        if not local_summary.exists():
            if not config_path.exists() and self.remote.exists(out + "/config.json"):
                receive(self.remote, self.root, out + "/config.json", False)
            if config_path.exists() and read_json(config_path)["generation_commit"] != self.options.generation_commit:
                raise ValueError(f"generation commit differs: {out}")
            state = self.remote.status(out)
            if state == "new" and not self.remote.exists(f"{out}/summary.json"):
                send(self.remote, self.root, [network])
                self.restore_generation(out)
                command = [ENGINE, "data", "zero-selfplay", "--output-dir", out, "--nn-model", network,
                           "--nn-backend", "tensorrt", "--generation-commit", self.options.generation_commit,
                           "--seed", str(seed), "--games", str(games), "--chunk-games", str(chunk_games),
                           "--target-positions", str(target_positions),
                           "--simulations", str(self.options.simulations), "--playthrough-percent", str(playthrough),
                           "--threads", str(self.options.threads), "--games-per-thread", str(self.options.games_per_thread)]
                self._begin(out)
                self.remote.start(out, shlex.join(command), out + ".out", out + ".err")
                state = "running"
            self._begin(out)
            if state != "new":
                code = self._monitor(cycle, out, lambda: self._pull_chunks(out))
                if code != 0:
                    raise RuntimeError(f"generation exited {code}: {out}")
            self._pull_chunks(out)
            if read_json(config_path)["generation_commit"] != self.options.generation_commit:
                raise ValueError(f"generation commit differs: {out}")
            for relative in (out + ".out", out + ".err", out + "/summary.json"):
                receive(self.remote, self.root, relative, False)
        self._begin(out)
        summary = read_json(local_summary)
        config = read_json(config_path)
        expected = {"network_sha256": sha256(self.root / network),
                    "nn_backend": "tensorrt", "seed": seed, "games": games, "chunk_games": chunk_games,
                    "simulations": self.options.simulations, "playthrough_percent": playthrough}
        if (any(config[key] != value for key, value in expected.items())
                or type(summary["target_positions"]) is not int
                or summary["target_positions"] != target_positions
                or type(summary["games"]) is not int or not 1 <= summary["games"] <= games):
            raise ValueError(f"generation settings differ: {out}")
        chunks = summary["chunks"]
        if type(chunks) is not int or not 1 <= chunks <= (games + chunk_games - 1) // chunk_games:
            raise ValueError(f"invalid generation chunks: {out}")
        if any(not (local_summary.parent / f"chunk-{chunk:05d}{suffix}").is_file()
               for chunk in range(chunks) for suffix in (".mnsd", ".resigned.mnsd", ".json")):
            raise ValueError(f"missing committed chunks: {out}")
        for relative in (out + ".out", out + ".err"):
            if not (self.root / relative).is_file():
                raise ValueError(f"missing generation log: {relative}")
        return summary, self._finish(out)

    def training_command(self, cycle: int, files: list[str], latest_files: list[str]) -> list[str]:
        out = self.options.cycle_path(cycle) + "/train"
        checkpoint = out + "/checkpoint.pt"
        if self.remote.exists(checkpoint):
            return [*TRAINER, "train", "--resume", out, "--device", "cuda"]
        if (self.root / checkpoint).exists():
            send(self.remote, self.root, [checkpoint])
            return [*TRAINER, "train", "--resume", out, "--device", "cuda"]
        initial = (self.options.cycle_path(0) + "/checkpoint.pt" if cycle == 1 else
                   self.options.cycle_path(cycle - 1) + "/train/checkpoint.pt")
        command = [*TRAINER, "train", "--init", initial, "--data", *files,
                   "--latest-data", *latest_files, "--out-dir", out,
                   "--cycle", str(cycle), "--seed", str(self.options.train_seed_base + cycle)]
        for name in ("batch_size", "lr", "momentum", "weight_decay", "clip_grad_norm",
                     "value_lambda", "precision", "checkpoint_steps"):
            command.extend(["--" + name.replace("_", "-"), str(getattr(self.options, name))])
        return [*command, "--device", "cuda", "--trainer-commit", self.options.trainer_commit]

    def training(self, cycle: int, files: list[str], latest_files: list[str]) -> tuple[dict, float]:
        out = self.options.cycle_path(cycle) + "/train"
        local_summary = self.root / out / "summary.json"
        if not local_summary.exists():
            state = self.remote.status(out)
            if state == "new" and not self.remote.exists(out + "/summary.json"):
                send(self.remote, self.root, files)
                initial = (self.options.cycle_path(0) + "/checkpoint.pt" if cycle == 1 else
                           self.options.cycle_path(cycle - 1) + "/train/checkpoint.pt")
                send(self.remote, self.root, [initial])
                command = self.training_command(cycle, files, latest_files)
                self._begin(out)
                self.remote.start(out, shlex.join(command), out + ".log", out + ".log")
                state = "running"
            self._begin(out)
            checkpoint = out + "/checkpoint.pt"

            def pull_checkpoint() -> int:
                if not self.remote.exists(checkpoint):
                    return 0
                return int(receive(self.remote, self.root, checkpoint, True))

            if state != "new":
                code = self._monitor(cycle, out, pull_checkpoint)
                if code != 0:
                    raise RuntimeError(f"training exited {code}: {out}")
            # 集計を一時領域で読み、ネットを受信・照合してから完了印を置く。
            with tempfile.TemporaryDirectory(prefix="minase-zero-summary-") as directory:
                staging = Path(directory)
                receive(self.remote, staging, out + "/summary.json", False)
                summary = read_json(staging / out / "summary.json")
                for relative in (checkpoint, out + "/model.onnx", out + ".log"):
                    receive(self.remote, self.root, relative, relative == checkpoint)
                if summary["cycle"] != cycle or summary["network_sha256"] != sha256(self.root / out / "model.onnx"):
                    raise ValueError(f"training summary does not match cycle or network: {out}")
                _write(local_summary, (staging / out / "summary.json").read_bytes())
        self._begin(out)
        summary = read_json(local_summary)
        if summary["cycle"] != cycle or summary["network_sha256"] != sha256(self.root / out / "model.onnx"):
            raise ValueError(f"training summary does not match cycle or network: {out}")
        return summary, self._finish(out)

    def check_script(self, cycle: int) -> str:
        out = self.options.cycle_path(cycle) + "/check"
        train = self.options.cycle_path(cycle) + "/train"
        elapsed = f'printf "%s\\n" "$((SECONDS - check_started))" > {shlex.quote(out + "/elapsed-seconds")}'
        lines = [f"mkdir -p -- {shlex.quote(out)}", "check_started=$SECONDS",
                 f"trap {shlex.quote(elapsed)} EXIT", "failed=0"]
        network = f"{train}/model.onnx"
        reference = f"{out}/pytorch.json"
        lines.append(shlex.join([*TRAINER, "fixture-eval", "--checkpoint", train + "/checkpoint.pt",
                                 "--onnx", network, "--fixture", FIXTURE,
                                 "--out", reference]))
        for batch in (1, 7, 256):
            for backend, label, policy, ptol, vtol in (("cpu", "cpu", "logits", "1e-4", "1e-4"),
                                                      ("tensorrt", "trt", "probabilities", "1e-2", "5e-2")):
                name = f"{out}/{label}-b{batch}"
                command = [ENGINE, "dev", "nn-eval", "--nn-model", network, "--backend", backend,
                           "--batch", str(batch), "--out", name + ".json"]
                if backend == "tensorrt":
                    command += ["--cache-dir", "/content/cache/nn-eval"]
                lines.append(shlex.join(command))
                compare = [*PYTHON, SOURCE + "/nn_compare.py", "--reference", reference,
                           "--candidate", name + ".json", "--policy", policy,
                           "--policy-tolerance", ptol, "--value-tolerance", vtol]
                lines.extend([f"if {shlex.join(compare)} > {shlex.quote(name + '.compare.json')} "
                              f"2> {shlex.quote(name + '.compare.err')}; then",
                              "    code=0", "else", "    code=$?", "    failed=1", "fi",
                              f"printf '%s\\n' \"$code\" > {shlex.quote(name + '.compare.exit')}"])
        lines.append('exit "$failed"')
        return "\n".join(lines)

    def checking(self, cycle: int) -> tuple[dict, float]:
        out = self.options.cycle_path(cycle) + "/check"
        code_path = self.root / (out + ".exit")
        if not code_path.exists():
            state = self.remote.status(out)
            if state == "new":
                send(self.remote, self.root, [f"{self.options.cycle_path(cycle)}/train/{name}"
                                             for name in ("checkpoint.pt", "model.onnx")])
                self._begin(out)
                self.remote.start(out, self.check_script(cycle), out + ".log", out + ".log")
            self._begin(out)
            self._monitor(cycle, out, lambda: 0)
            for name in self.remote.files(out):
                receive(self.remote, self.root, f"{out}/{name}", False)
            receive(self.remote, self.root, out + ".log", False)
            receive(self.remote, self.root, out + ".exit", False)
        self._begin(out)
        code = int(code_path.read_text(encoding="utf-8"))
        if code != 0:
            raise RuntimeError(f"network check exited {code}; outputs saved in {self.root / out}")
        results = []
        for batch in (1, 7, 256):
            for backend in ("cpu", "trt"):
                path = self.root / out / f"{backend}-b{batch}.compare"
                status = int(Path(str(path) + ".exit").read_text(encoding="utf-8"))
                output = Path(str(path) + ".json").read_text(encoding="utf-8").strip()
                if status != 0 or len(output.splitlines()) != 1:
                    raise ValueError(f"failed or malformed comparison: {path}")
                results.append({"backend": backend, "batch": batch, "exit_code": status,
                                "stdout": output, "result": json.loads(output)})
        elapsed = float((self.root / out / "elapsed-seconds").read_text(encoding="utf-8"))
        if not math.isfinite(elapsed) or elapsed < 0:
            raise ValueError("invalid remote check elapsed seconds")
        return {"results": results, "elapsed_seconds": elapsed}, self._finish(out)

    def run(self) -> None:
        options = self.options
        try:
            result = subprocess.run([str(options.minase.resolve()), "data", "teacher-stats", "--help"],
                                    capture_output=True, text=True)
        except OSError as error:
            raise RuntimeError(f"cannot run minase data teacher-stats: {options.minase}: {error}") from error
        if result.returncode != 0:
            raise RuntimeError(f"minase data teacher-stats --help exited {result.returncode}: {result.stderr}")
        if not self.remote.exists(FIXTURE):
            raise RuntimeError(f"deployed commit {options.trainer_commit} is missing fixture: {FIXTURE}")
        mainline = options.cycles_dir == Path("cycles")
        cycles_log = self.root / f"{options.cycles_dir}.jsonl"
        sessions_log = self.root / ("sessions.jsonl" if mainline else f"{options.cycles_dir}-sessions.jsonl")
        seed_name = "cycle" if mainline else options.cycles_dir.as_posix()
        started = datetime.now(UTC)
        cycles = read_rows(cycles_log)
        numbers = [row["cycle"] for row in cycles]
        if len(set(numbers)) != len(numbers):
            raise ValueError(f"{cycles_log}: duplicate cycles")
        machine = None
        start_balance = None
        for cycle in range(options.first_cycle, options.last_cycle + 1):
            if cycle in numbers:
                if not (self.root / options.network_path(cycle)).is_file():
                    raise ValueError(f"recorded cycle has no immutable network: {cycle}")
                self.log(cycle, "already complete")
                continue
            if any(row["cycle"] == cycle for row in read_rows(sessions_log)):
                raise ValueError(f"{sessions_log}: duplicate cycle {cycle}")
            now = datetime.now(UTC)
            if now + timedelta(minutes=options.cycle_minutes) > options.deadline:
                self.log(cycle, "deadline does not allow another cycle")
                return
            balance = self.remote.usage()
            if not can_start(datetime.now(UTC), options.deadline, options.cycle_minutes, balance, options.cycle_cu):
                self.log(cycle, "time or balance does not allow another cycle")
                return
            if start_balance is None:
                start_balance = balance
            if machine is None:
                machine = self.remote.machine()
            playthrough, calibration = resignation(options, cycle)
            reference = ([options.cycle1_reference_summary] if cycle == 1 else summaries(options, cycle - 1))
            games, chunk_games = game_counts(reference, options.target_positions, options.chunks)
            seed = cycle * 10**9 + (0 if mainline else 7 * 10**8)
            network = (self.options.cycle_path(0) + "/model.onnx" if cycle == 1 else
                       self.options.cycle_path(cycle - 1) + "/train/model.onnx")
            generation = {"network": network, "network_sha256": sha256(self.root / network), "seed": seed,
                          "games": games, "chunk_games": chunk_games, "playthrough_percent": playthrough,
                          "target_positions": options.target_positions,
                          "threads": options.threads, "games_per_thread": options.games_per_thread,
                          "simulations": options.simulations}
            seed_registration(self.root / "seed-ledger.jsonl", f"{seed_name}-{cycle:04d}-gen", seed, games)
            self.log(cycle, f"generation games={games} chunk_games={chunk_games} playthrough_percent={playthrough}")
            summary, generation_seconds = self.generation(cycle, self.options.cycle_path(cycle) + "/selfplay", network,
                                                         seed, games, chunk_games, playthrough, options.target_positions)
            config = read_json(self.root / self.options.cycle_path(cycle) / "selfplay/config.json")
            generation["generation_commit"] = config["generation_commit"]
            regen_summary, regen_settings = None, None
            if summary["resigned_games_excluded"]:
                count = summary["resigned_games"]
                if type(count) is not int or count < 0:
                    raise ValueError("invalid resigned_games")
                if count:
                    regen_target = summary.get("resigned_positions")
                    if type(regen_target) is not int or regen_target <= 0:
                        raise ValueError("invalid resigned_positions")
                    regen_seed = seed + 5 * 10**8
                    regen_chunk = (count + options.chunks - 1) // options.chunks
                    seed_registration(self.root / "seed-ledger.jsonl", f"{seed_name}-{cycle:04d}-regen", regen_seed, count)
                    regen_summary, seconds = self.generation(cycle, self.options.cycle_path(cycle) + "/selfplay-regen", network,
                                                            regen_seed, count, regen_chunk, 100, regen_target)
                    generation_seconds += seconds
                    regen_config = read_json(self.root / self.options.cycle_path(cycle) / "selfplay-regen/config.json")
                    regen_settings = {**generation, "seed": regen_seed, "games": count,
                                      "target_positions": regen_target,
                                      "chunk_games": regen_chunk, "playthrough_percent": 100,
                                      "generation_commit": regen_config["generation_commit"]}
            window_cycles, files, current_files = training_window(options, cycle, options.window_positions)
            train_summary, training_seconds = self.training(cycle, files, current_files)
            checks, checking_seconds = self.checking(cycle)
            result = subprocess.run([str(options.minase.resolve()), "data", "teacher-stats", *current_files], cwd=self.root,
                                    capture_output=True, text=True)
            if result.returncode != 0:
                raise RuntimeError(f"minase data teacher-stats exited {result.returncode}: {result.stderr}")
            stats = json.loads(result.stdout)
            if not isinstance(stats, dict):
                raise ValueError("minase data teacher-stats must output a JSON object")
            network_digest = publish_network(options, cycle)
            end_balance = self.remote.usage()
            ended = datetime.now(UTC)
            settings = {"lr": options.lr,
                        "cycles_dir": options.cycles_dir.as_posix(),
                        "base_cycles_dir": options.base_cycles_dir.as_posix(), "branch_from": options.branch_from}
            interval = {**settings, "session": options.session, "cycle": cycle, "started_at": started.isoformat(),
                        "ended_at": ended.isoformat(), "start_balance": start_balance,
                        "end_balance": end_balance, "balance_difference": start_balance - end_balance}
            if end_balance > start_balance:
                interval["balance_increased"] = True
            record = {**settings, "cycle": cycle, "generation_commit": generation["generation_commit"],
                      "trainer_commit": options.trainer_commit, "generation": generation,
                      "resign_calibration": calibration, "selfplay_summary": summary,
                      "regeneration": regen_settings, "selfplay_regen_summary": regen_summary,
                      "training_files": files, "window_cycles": window_cycles,
                      "window_first_cycle": options.window_first_cycle,
                      "window_positions": options.window_positions, "teacher_stats": stats,
                      "training_summary": train_summary, "network_sha256": network_digest, "checks": checks,
                      "elapsed_seconds": {"generation": generation_seconds, "training": training_seconds,
                                          "checking": checking_seconds}, "machine": machine, "interval": interval}
            _json_bytes(record)
            # 各台帳は全体を改名する。2つの台帳の間で中断した場合は二重登録の検査で止める。
            append_cycle(sessions_log, interval)
            append_cycle(cycles_log, record)
            self.log(cycle, f"complete balance={end_balance}")
            started, start_balance = ended, end_balance
