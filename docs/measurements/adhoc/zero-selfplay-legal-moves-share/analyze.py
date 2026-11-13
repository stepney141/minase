"""perf scriptの出力を読み、スレッドごとのCPU時間と、関数を含むスタックの時間を集計する。

入力は `perf script -F comm,tid,period,ip,sym,dso` の出力である。
cpu-clockの標本のperiodはナノ秒なので、periodの和がCPU時間になる。
"""

import collections
import json
import re
import sys

HEADER = re.compile(r"^(?P<comm>.+?)\s+(?P<tid>\d+)\s+(?P<period>\d+)\s*$")
FRAME = re.compile(r"^\s+[0-9a-f]+\s+(?P<sym>.*)\s+\((?P<dso>[^()]*)\)\s*$")

# 包含時間を求める関数。キーは出力の名前、値はシンボルに含まれる文字列。
INCLUSIVE = {
    "legal_moves": "minase_core::game::referee::Game>::legal_moves",
    "piece_moves_full": "generate_piece_moves::<false, false>",
    "push_with_promotion": "movegen::expand::push_with_promotion",
    "promotion_choice_for": "MoveRules>::promotion_choice_for",
    "special_move_is_legal": "MoveRules>::special_move_is_legal",
    "mcts": "minase::search::mcts::",
    "records": "minase::training::records::",
    "selfplay": "zero_selfplay::",
}


def strip_offset(sym: str) -> str:
    return re.sub(r"\+0x[0-9a-f]+$", "", sym.strip())


def samples(path: str):
    comm = tid = period = None
    stack: list[tuple[str, str]] = []
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if not line.strip():
                if comm is not None:
                    yield comm, tid, period, stack
                comm = None
                stack = []
                continue
            if not line[0].isspace():
                match = HEADER.match(line)
                if match is None:
                    raise SystemExit(f"見出しを読めない: {line!r}")
                comm = match["comm"].strip()
                tid = int(match["tid"])
                period = int(match["period"])
                continue
            match = FRAME.match(line)
            if match is None:
                raise SystemExit(f"フレームを読めない: {line!r}")
            stack.append((strip_offset(match["sym"]), match["dso"].rsplit("/", 1)[-1]))
        if comm is not None:
            yield comm, tid, period, stack


def main() -> None:
    path = sys.argv[1]
    threads: dict[int, dict] = {}
    for comm, tid, period, stack in samples(path):
        entry = threads.setdefault(
            tid,
            {
                "comm": comm,
                "total_ns": 0,
                "inclusive_ns": collections.Counter(),
                "legal_moves_caller_ns": collections.Counter(),
                "leaf_dso_ns": collections.Counter(),
                "self_ns": collections.Counter(),
            },
        )
        entry["total_ns"] += period
        if not stack:
            entry["leaf_dso_ns"]["(スタックなし)"] += period
            continue
        symbols = [sym for sym, _ in stack]
        entry["leaf_dso_ns"][stack[0][1]] += period
        entry["self_ns"][symbols[0]] += period
        for name, needle in INCLUSIVE.items():
            if any(needle in sym for sym in symbols):
                entry["inclusive_ns"][name] += period
        for index, sym in enumerate(symbols):
            if INCLUSIVE["legal_moves"] in sym:
                # インライン展開されたフレームを飛ばし、実体のある最初の呼び出し元を取る。
                caller = next(
                    (sym for sym, dso in stack[index + 1 :] if dso != "inlined"),
                    "(呼び出し元なし)",
                )
                entry["legal_moves_caller_ns"][caller] += period
                break
    output = []
    for tid, entry in sorted(threads.items(), key=lambda item: -item[1]["total_ns"]):
        output.append(
            {
                "tid": tid,
                "comm": entry["comm"],
                "total_s": entry["total_ns"] / 1e9,
                "inclusive_s": {k: v / 1e9 for k, v in entry["inclusive_ns"].most_common()},
                "legal_moves_caller_s": {
                    k: v / 1e9 for k, v in entry["legal_moves_caller_ns"].most_common(5)
                },
                "leaf_dso_s": {k: v / 1e9 for k, v in entry["leaf_dso_ns"].most_common(8)},
                "self_top_s": {k: v / 1e9 for k, v in entry["self_ns"].most_common(25)},
            }
        )
    json.dump(output, sys.stdout, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
