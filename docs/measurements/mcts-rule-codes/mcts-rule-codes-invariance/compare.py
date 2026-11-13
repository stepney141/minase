#!/usr/bin/env python3
"""変更の前後のminaseで、ネットの出力と固定回数のMCTSの結果が一致するかを調べる。

規則セットごとに、2つのバイナリを`--protocol usi --search mcts`で起動し、
USIの独自コマンド`nn-eval`の出力（固定の288局面の価値とlogit）と、固定の15局面に対する
`go nodes <回数>`の最善手、評価値、および読み筋を集めて比べる。
不一致が1つでもあれば終了コード1で終わる。

推論にはONNX Runtimeが要るので、`tools/train/zero/env.sh`をbashで読み込み、
`XDG_CACHE_HOME`を設定した環境で実行する。
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

RULE_SETS = ["lishogi", "JCSA"]

# crates/minase-core/src/test_util.rs の BENCH_SFENS の15局面。先頭は初期局面である。
# `position sfen`は4欄の拡張SFENを要するので、先獅子の欄`-`と手数`1`を足している。
POSITIONS = [
    "sfen lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 1",
    "sfen lfcsgekgsc1l/a1b1txot1bfa/mvrhd1d4r/1p1p1p2+INpp/p1pi2pp4/4p1n5/9P2/3IP2P4/PPPP1PP1POPP/MVR1D2D1RVM/A1B1T1XT1B1A/LFCSGKEGSCFL b - 1",
    "sfen lfcsgekg1c1l/a1b1txo2s1a/mvrh1d6/1p1p1p2+IN+Op/p2i2ppB3/2p1p1n5/9P2/3IP2P4/PPPP1PP1P1PP/MVR1D2D1RVM/A3T1XT1B1A/LFCSGKEGSCFL b - 1",
    "sfen 1q1b2ko3l/1d1h1ett1b1a/1dm7r1/1+g1psn1xpmv1/2pisghcip2/3+p5fpp/6N5/2P3PH4/4R2TMV2/3T1DBDG3/2H2OXS3A/Q2SGKE1FC1L b - 1",
    "sfen 3+s1ok5/5ett4/12/3ps1x+bpm1+r/3i1g1cip+p1/4+b+pP5/4n2N4/7E4/12/12/4S+VD5/5KS1FC2 w - 1",
    "sfen 2q1gekog3/4tddtsb2/fvrbs3h3/2mp1c1xp1v1/1ppip1ppi2m/5p6/3I5p2/1P2P1PPI3/1MPP4PN2/2VRDQD1+c3/2BHTOXR4/1FCSGKEGS1n1 b - 1",
    "sfen 4gekog3/5d1ts3/1v1t8/2mp1c2p3/Xf1ipsppmx2/1pp9/3I8/1P2PpPN4/1MPP8/2V1OGHE3+d/R3T7/1FCSGK2S1+v+r b - 1",
    "sfen 1hcs1ek3+P1/1f2gxgq4/m1rd1t2ton1/1v1pp3p1s1/2p3N5/1p3p2X3/3I4H3/1PP1PP2IQ2/L2P4P2+L/VC4D3V1/2F1TO1TC3/3SGKEGS1F1 b - 1",
    "sfen 2cs1keh4/1f4g3o1/m2dtg4+H1/1v1pp7/11O/1pp9/3I1pQ4+V/1PPPPP2I3/L7P3/VC4+L5/2F1TEGT1F2/3SGK2S3 w - 1",
    "sfen l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppih1ppippp/4pn6/1P1N8/P1PIPPPPIPPP/MF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL w - 1",
    "sfen l1c1gekgsc1l/3st2t1b1a/1vr1dq1dhrv1/a2p1poxp1fm/1p2h1ppippp/p4n6/1PFbf7/PN3PPPIPPP/MC1P1OC1P3/2RHD1Q1HRVM/A1B1T1DT1B1A/LV1SGKEGS1FL w - 1",
    "sfen l3g1k4l/a2stegt3a/m7d2m/p2+I+D+P+S4p/1F9h/10s1/P8f2/3P1PP2nc1/2R8P/4CTG1TF1A/A5XE1C2/L4K2Q2L w - 1",
    "sfen l5k4l/a2sge1t3a/12/p4+P5p/3+f3m2E1/1+I1+F5T2/P2C2P5/3P8/11P/11A/A3K7/L10L w - 1",
    "sfen +l+d4k4v/4exqhob2/1+rr5mc2/3tt5f1/5ppNp1p1/1m7p2/5P4P1/4O2Pi3/2+a3P3M1/3T2GDR2F/2S2GXEHV2/2Q3K1SC2 w - 1",
    "sfen 4+l1k2+b1v/2+d3e5/6+m5/2+r9/8pNp1/5p3p2/6p3P1/2o9/5G6/6E4F/8+R1VC/6QKS3 b - 1",
]


def read_until(process: subprocess.Popen, prefix: str) -> list[str]:
    """`prefix`で始まる行までを読み、その行を含む全行を返す。"""
    lines = []
    while True:
        line = process.stdout.readline()
        if line == "":
            raise RuntimeError(f"engine exited before {prefix!r}: {lines[-5:]}")
        line = line.rstrip("\n")
        if line.startswith("info string error"):
            raise RuntimeError(line)
        lines.append(line)
        if line.startswith(prefix):
            return lines


def run_engine(binary: Path, rules: str, net: Path, nodes: int, nn_eval_out: Path) -> list[dict]:
    process = subprocess.Popen(
        [str(binary), "--protocol", "usi", "--search", "mcts"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )

    def send(text: str) -> None:
        process.stdin.write(text + "\n")
        process.stdin.flush()

    send("usi")
    read_until(process, "usiok")
    send(f"setoption name RuleSet value {rules}")
    send(f"setoption name NnModel value {net}")
    send("isready")
    read_until(process, "readyok")
    send(f"nn-eval out {nn_eval_out}")
    read_until(process, "info string end")
    results = []
    for position in POSITIONS:
        send("usinewgame")
        send(f"position {position}")
        send(f"go nodes {nodes}")
        lines = read_until(process, "bestmove")
        info = [line.split() for line in lines if line.startswith("info depth")][-1]
        results.append(
            {
                "position": position,
                "bestmove": lines[-1],
                "score": " ".join(info[info.index("score") + 1 : info.index("nodes")]),
                "nodes": info[info.index("nodes") + 1],
                "pv": " ".join(info[info.index("pv") + 1 :]),
            }
        )
    send("quit")
    if process.wait(timeout=60) != 0:
        raise RuntimeError(f"{binary} exited with {process.returncode}")
    return results


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--net", type=Path, required=True)
    parser.add_argument("--nodes", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True)
    mismatches = 0
    for rules in RULE_SETS:
        searches = {}
        evaluations = {}
        for label, binary in (("before", args.before), ("after", args.after)):
            nn_eval_out = (args.out / f"{rules}.{label}.nn-eval.json").resolve()
            searches[label] = run_engine(
                binary.resolve(), rules, args.net.resolve(), args.nodes, nn_eval_out
            )
            evaluations[label] = nn_eval_out.read_bytes()
            (args.out / f"{rules}.{label}.search.json").write_text(
                json.dumps(searches[label], ensure_ascii=False, indent=1) + "\n"
            )
        nn_equal = evaluations["before"] == evaluations["after"]
        differing = [
            before["position"]
            for before, after in zip(searches["before"], searches["after"], strict=True)
            if before != after
        ]
        mismatches += int(not nn_equal) + len(differing)
        print(
            f"{rules}: nn-eval {'identical' if nn_equal else 'DIFFERENT'} "
            f"({len(evaluations['before'])} bytes), "
            f"search {len(POSITIONS) - len(differing)}/{len(POSITIONS)} identical"
        )
        for position in differing:
            print(f"  differs: {position}")
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
