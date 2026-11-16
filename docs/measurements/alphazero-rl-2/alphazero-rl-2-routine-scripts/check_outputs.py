"""学習した機械のPyTorch出力と、この作業ツリーの推論出力を照合する。

bashで tools/train/zero/env.sh を読み込んでから実行する。
実行の組と許容誤差の根拠は docs/guides/alphazero-network-check.md の
3節と5節にある。規則セットは2回目の強化学習に合わせてlishogiとする。
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[4]


def usi_path(text):
    if not text or "\n" in text or "\r" in text:
        raise argparse.ArgumentTypeError("パスは空文字や改行を含められません")
    return Path(text).resolve()


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--net", type=usi_path, required=True)
    result.add_argument("--reference", type=Path, required=True)
    result.add_argument("--out", type=usi_path, required=True)
    return result


def usi_lines(net, backend, batch, out, name):
    lines = [
        "usi", "setoption name RuleSet value lishogi",
        f"setoption name NnModel value {net}",
        f"setoption name NnBackend value {backend}",
        f"setoption name NnBatchSize value {batch}",
    ]
    if backend == "tensorrt":
        lines.append(f"setoption name NnCacheDir value {out / 'trt-cache'}")
    return lines + ["isready", f"nn-eval out {out / (name + '.json')}", "quit"]


def check_outputs(args):
    started = time.monotonic()
    if not os.environ.get("ORT_DYLIB_PATH"):
        raise ValueError("ORT_DYLIB_PATHがありません。bashで tools/train/zero/env.sh を読み込んでください")
    out = args.out
    if out.exists():
        if not out.is_dir() or any(out.iterdir()):
            raise ValueError(f"--outには存在しないか空のディレクトリを指定してください: {out}")
    else:
        out.mkdir(parents=True)

    cases = []
    for batch in (1, 7, 256):
        for backend, label, policy, ptol, vtol in (
            ("cpu", "cpu", "fp16-logits", "1e-4", "1e-4"),
            ("tensorrt", "trt", "probabilities", "5e-2", "5e-2"),
        ):
            name = f"{label}-b{batch}"
            log_path = out / f"{name}.usi.log"
            with log_path.open("w", encoding="utf-8") as log:
                result = subprocess.run(
                    [str(ROOT / "target/release/minase"), "--protocol", "usi", "--search", "mcts"],
                    input="\n".join(usi_lines(args.net, backend, batch, out, name)) + "\n",
                    text=True, stdout=log, cwd=ROOT,
                )
            if result.returncode != 0:
                raise RuntimeError(f"{name}: エンジンの終了コードが{result.returncode}です: {log_path}")
            if "info string end" not in log_path.read_text(encoding="utf-8").splitlines():
                raise RuntimeError(f"{name}: エンジンの完了行 info string end がありません: {log_path}")
            cases.append((backend, batch, name, policy, ptol, vtol))

    results = []
    for backend, batch, name, policy, ptol, vtol in cases:
        with (out / f"{name}.compare.json").open("w", encoding="utf-8") as stdout, \
                (out / f"{name}.compare.err").open("w", encoding="utf-8") as stderr:
            result = subprocess.run(
                [sys.executable, "-I", str(ROOT / "tools/train/zero/nn_compare.py"),
                 "--reference", str(args.reference.resolve()),
                 "--candidate", str(out / f"{name}.json"),
                 "--policy", policy, "--policy-tolerance", ptol, "--value-tolerance", vtol],
                stdout=stdout, stderr=stderr, cwd=ROOT,
            )
        results.append({"backend": backend, "batch": batch, "exit_code": result.returncode})

    summary = {"results": results, "elapsed_seconds": time.monotonic() - started}
    serialized = json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    (out / "summary.json").write_text(serialized, encoding="utf-8")
    print(serialized, end="")
    return int(any(result["exit_code"] != 0 for result in results))


def main():
    args = parser().parse_args()
    try:
        return check_outputs(args)
    except (OSError, ValueError, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
