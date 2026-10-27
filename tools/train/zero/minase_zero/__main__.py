"""初期化、学習、再開、および固定局面集の推論を明示した引数で実行する。"""

import argparse
from pathlib import Path
import pickle
import sys

from .train import fixture_eval, initialize, resume, train_new


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError(message)


def _parse(argv: list[str]) -> argparse.Namespace:
    parser = _Parser(description=__doc__, allow_abbrev=False)
    commands = parser.add_subparsers(dest="command", required=True)
    init = commands.add_parser("init", allow_abbrev=False)
    for name in ("blocks", "channels", "seed"):
        init.add_argument(f"--{name}", type=int, required=True)
    init.add_argument("--out-dir", type=Path, required=True)
    init.add_argument("--trainer-commit", required=True)

    train = commands.add_parser("train", allow_abbrev=False)
    train.add_argument("--device", choices=("cuda", "cpu"), required=True)
    resuming = bool(argv) and argv[0] == "train" and any(
        arg == "--resume" or arg.startswith("--resume=") for arg in argv[1:]
    )
    if resuming:
        train.add_argument("--resume", type=Path, required=True)
    else:
        train.add_argument("--init", required=True)
        train.add_argument("--data", nargs="+", required=True)
        train.add_argument("--out-dir", type=Path, required=True)
        for name in ("cycle", "seed", "batch-size", "checkpoint-steps"):
            train.add_argument(f"--{name}", type=int, required=True)
        for name in ("lr", "momentum", "weight-decay", "clip-grad-norm", "value-lambda"):
            train.add_argument(f"--{name}", type=float, required=True)
        train.add_argument("--precision", choices=("bf16", "fp32"), required=True)
        train.add_argument("--trainer-commit", required=True)

    fixture = commands.add_parser("fixture-eval", allow_abbrev=False)
    for name in ("checkpoint", "onnx", "fixture", "out"):
        fixture.add_argument(f"--{name}", type=Path, required=True)
    return parser.parse_args(argv)


def main() -> int:
    """引数と実行時の条件違反を標準エラーの1行と終了コード1に変換する。"""
    try:
        args = vars(_parse(sys.argv[1:]))
        command = args.pop("command")
        if command == "init":
            initialize(**args)
        elif command == "fixture-eval":
            fixture_eval(**args)
        elif "resume" in args:
            resume(args["resume"], args["device"])
        else:
            out_dir = args.pop("out_dir")
            train_new(args, out_dir)
    except (OSError, ValueError, KeyError, IndexError, TypeError, RuntimeError,
            EOFError, OverflowError, pickle.UnpicklingError) as error:
        print("minase_zero: " + " ".join(str(error).split()), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
