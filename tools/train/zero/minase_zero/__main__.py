"""初期化、学習、再開、および固定局面集の推論を明示した引数で実行する。

作業ディレクトリをtools/train/zeroにして、`uv run --frozen python -m minase_zero <サブコマンド>`で起動する。
引数はすべて既定値を持たない。

- init：--blocks、--channels、--seedのネットを作り、--out-dir（存在しないか空）へ書く。
- train（新しいサイクル）：--init（すべての更新を終えたチェックポイント）、--data（学習窓の
  MNSDファイルの列）、--latest-data（最新サイクルのファイルの列で、dataの末尾と一致する）、
  --out-dir（存在しないか空）、--cycle、--seed、--batch-size、--lr、
  --momentum、--weight-decay、--clip-grad-norm、--value-lambda、--precision（bf16かfp32）、
  --checkpoint-steps、--ema-period-samples、--ema-scale、--device（cudaかcpu）、
  --trainer-commit（40桁の16進数字）。EMAの2引数は両方0で無効、両方正で有効とする。
- train --resume <出力先> --device：ほかの設定はチェックポイントから読み、中断した学習を続ける。
- fixture-eval：--checkpoint、--onnx、--fixture、--out、--weights（rawかema）。
- external-eval：--checkpoint、--data（αβのMNSDの列）、--games、--seed、--device、--out。
- seed-ledger：--ledger（既存の台帳）、--name、--seed、--games。
- resign-calibration：--summary（直前のサイクルの生成器の集計の列）。
- run-cycles：NASとColab機の間で生成、学習、照合を順に実行する。--window-positionsで学習窓の
  局面数、--generation-commitと--trainer-commitで生成と学習のコミットを指定する。
  引数と再開の契約はrunner.pyにある。

引数の誤りと実行時の条件違反は、標準エラーの1行と終了コード1で報告する。
出力先と学習の契約はtrain.pyのdocstringにある。
"""

import argparse
import json
from pathlib import Path
import pickle
import sys

from .external import external_eval
from .rl import register_seed_range, resign_calibration
from .runner import Options, Remote, Runner, add_arguments
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
        train.add_argument("--latest-data", nargs="+", required=True)
        train.add_argument("--out-dir", type=Path, required=True)
        for name in ("cycle", "seed", "batch-size", "checkpoint-steps", "ema-period-samples"):
            train.add_argument(f"--{name}", type=int, required=True)
        for name in ("lr", "momentum", "weight-decay", "clip-grad-norm", "value-lambda", "ema-scale"):
            train.add_argument(f"--{name}", type=float, required=True)
        train.add_argument("--precision", choices=("bf16", "fp32"), required=True)
        train.add_argument("--trainer-commit", required=True)

    fixture = commands.add_parser("fixture-eval", allow_abbrev=False)
    for name in ("checkpoint", "onnx", "fixture", "out"):
        fixture.add_argument(f"--{name}", type=Path, required=True)
    fixture.add_argument("--weights", choices=("raw", "ema"), required=True)

    external = commands.add_parser("external-eval", allow_abbrev=False)
    for name in ("checkpoint", "out"):
        external.add_argument(f"--{name}", type=Path, required=True)
    external.add_argument("--data", nargs="+", required=True)
    for name in ("games", "seed"):
        external.add_argument(f"--{name}", type=int, required=True)
    external.add_argument("--device", choices=("cuda", "cpu"), required=True)

    ledger = commands.add_parser("seed-ledger", allow_abbrev=False)
    ledger.add_argument("--ledger", type=Path, required=True)
    ledger.add_argument("--name", required=True)
    for name in ("seed", "games"):
        ledger.add_argument(f"--{name}", type=int, required=True)

    calibration = commands.add_parser("resign-calibration", allow_abbrev=False)
    calibration.add_argument("--summary", type=Path, nargs="+", required=True)
    add_arguments(commands.add_parser("run-cycles", allow_abbrev=False))
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
        elif command == "external-eval":
            external_eval(**args)
        elif command == "seed-ledger":
            print(json.dumps(register_seed_range(**args), ensure_ascii=False, allow_nan=False))
        elif command == "resign-calibration":
            print(json.dumps(resign_calibration(**args), allow_nan=False))
        elif command == "run-cycles":
            options = Options(**args)
            Runner(options, Remote(options.session)).run()
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
