#!/usr/bin/env python3
"""固定したランダム対局でPGOプロファイルを生成し、通常ビルドで照合する。

使い方:
    python3 scripts/pgo_profile.py [--seed 9001] [--games 12] [--depth 5]

学習局面はusi_randomの対局から抽出し、benchの局面を学習には使わない。
llvm-profdataには、使用するRustのllvm-toolsコンポーネントを用いる。
"""

import argparse
import hashlib
import json
import os
import select
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RULES = "engine-default"
PLIES = (12, 40, 80, 120, 160, 200, 260, 320)
MAX_PLY = 340


class Usi:
    """行単位でUSIをやり取りし、終局時の無応答に時間制限を設ける。"""

    def __init__(self, binary, environment):
        self.proc = subprocess.Popen(
            [str(binary), "--protocol", "usi", "--rules", RULES],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            bufsize=0,
            env=environment,
        )
        self.buffer = b""

    def __enter__(self):
        try:
            self.send("usi")
            self.read_until("usiok", timeout=10)
            self.send("isready")
            self.read_until("readyok", timeout=10)
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise
        return self

    def __exit__(self, exc_type, exc_value, traceback):
        try:
            if self.proc.poll() is None:
                self.send("quit")
            self.proc.communicate(timeout=10)
        except (BrokenPipeError, subprocess.TimeoutExpired):
            self.proc.kill()
            self.proc.communicate()
            if exc_type is None:
                raise
        if exc_type is None and self.proc.returncode != 0:
            raise RuntimeError(f"engine exited with status {self.proc.returncode}")

    def send(self, line):
        self.proc.stdin.write((line + "\n").encode())

    def read_until(self, prefix, timeout=None):
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            if b"\n" in self.buffer:
                raw, self.buffer = self.buffer.split(b"\n", 1)
                line = raw.decode().strip()
                if line.startswith("info string error:") and line != "info string error: go requires an active game":
                    raise RuntimeError(line)
                if line.startswith(prefix):
                    return line
                continue
            remaining = None if deadline is None else max(0, deadline - time.monotonic())
            ready, _, _ = select.select([self.proc.stdout], [], [], remaining)
            if not ready:
                raise TimeoutError(f"engine did not return {prefix} within {timeout}s")
            chunk = os.read(self.proc.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError(f"engine exited before {prefix}")
            self.buffer += chunk

    def bestmove(self, moves, go, timeout=None):
        self.send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        self.send(go)
        return self.read_until("bestmove", timeout).split()[1]


def random_game(engine, seed):
    """対局ごとにシードを明示し、最大340手までの着手列を作る。"""
    engine.send(f"setoption name Seed value {seed}")
    engine.send("usinewgame")
    moves = []
    while len(moves) < MAX_PLY:
        try:
            move = engine.bestmove(moves, "go", timeout=5)
        except TimeoutError:
            # 終局した局面ではusi_randomがbestmoveを返さない。
            break
        if move in ("resign", "win", "none"):
            break
        moves.append(move)
    return moves


def train(binary_directory, training, environment):
    """ランダム対局から指定手数の局面を抽出し、固定深さで探索する。"""
    with Usi(binary_directory / "usi_random", environment) as random_engine:
        games = [random_game(random_engine, seed) for seed in training["game_seeds"]]
    searched = 0
    with Usi(binary_directory / "minase", environment) as engine:
        for index, moves in enumerate(games):
            for ply in training["plies"]:
                if ply >= len(moves):
                    continue
                engine.send("usinewgame")
                best = engine.bestmove(moves[:ply], f"go depth {training['depth']}")
                searched += 1
                print(f"game={index + 1} ply={ply} bestmove={best}", file=sys.stderr)
    if searched == 0:
        raise RuntimeError("training produced no search positions")
    print(f"searched={searched}", file=sys.stderr)


def llvm_profdata(environment):
    """実際に使用するRustと同じツールチェーンのllvm-profdataを探す。"""
    rustc = environment["RUSTC"] if "RUSTC" in environment else "rustc"
    version = subprocess.check_output([rustc, "-Vv"], text=True, env=environment)
    host = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
    sysroot = subprocess.check_output([rustc, "--print", "sysroot"], text=True, env=environment).strip()
    executable = Path(sysroot) / "lib/rustlib" / host / "bin/llvm-profdata"
    if not executable.is_file():
        raise RuntimeError(f"{executable} is missing; run rustup component add llvm-tools")
    return executable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=9001)
    parser.add_argument("--games", type=int, default=12)
    parser.add_argument("--depth", type=int, default=5)
    args = parser.parse_args()
    if args.seed < 1 or args.games < 1 or args.seed + args.games - 1 > 2**64 - 1:
        parser.error("seeds must be positive u64 values and --games must be positive")
    if not 1 <= args.depth <= 256:
        parser.error("--depth must be between 1 and 256")

    start = time.monotonic()
    environment = os.environ.copy()
    # 最終ビルドでは生成用指定と外部の最適化フラグを引き継がない。
    for variable in ("MINASE_PGO_GENERATE", "MINASE_PGO_WRITE_MANIFEST", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        environment.pop(variable, None)
    profdata = llvm_profdata(environment)
    pgo = ROOT / "pgo"
    pgo.mkdir(exist_ok=True)
    target = ROOT / "target/pgo-generate"
    training = {
        "seed": args.seed,
        "games": args.games,
        "game_seeds": [args.seed + game for game in range(args.games)],
        "plies": list(PLIES),
        "max_ply": MAX_PLY,
        "depth": args.depth,
        "rules": RULES,
    }
    with tempfile.TemporaryDirectory(prefix="minase-pgo-") as temporary:
        generate_environment = environment | {
            "MINASE_PGO_GENERATE": "1",
            "RUSTFLAGS": f"-C target-cpu=native -C profile-generate={temporary}",
            "CARGO_TARGET_DIR": str(target),
            "LLVM_PROFILE_FILE": f"{temporary}/%p.profraw",
        }
        subprocess.run(
            ["cargo", "build", "--release", "--bin", "minase", "--bin", "usi_random"],
            cwd=ROOT, env=generate_environment, check=True,
        )
        (pgo / "training.json").write_text(json.dumps(training, indent=2) + "\n")
        train(target / "release", training, generate_environment)
        profiles = sorted(Path(temporary).glob("*.profraw"))
        if not profiles:
            raise RuntimeError("training produced no .profraw files")
        subprocess.run(
            [str(profdata), "merge", "-o", str(pgo / "minase.profdata"), *map(str, profiles)],
            cwd=ROOT, env=environment, check=True,
        )
    build = ["cargo", "build", "--release", "--bin", "minase"]
    subprocess.run(
        build, cwd=ROOT, env=environment | {"MINASE_PGO_WRITE_MANIFEST": "1"}, check=True,
    )
    subprocess.run(build, cwd=ROOT, env=environment, check=True)
    profile = (pgo / "minase.profdata").read_bytes()
    print(f"elapsed={time.monotonic() - start:.3f}s bytes={len(profile)} sha256={hashlib.sha256(profile).hexdigest()}")


if __name__ == "__main__":
    main()
