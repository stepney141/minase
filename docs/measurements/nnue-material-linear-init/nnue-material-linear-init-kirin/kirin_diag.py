"""人間の棋譜で麒麟の3筋と7筋への置き直しを比べる単独の診断スクリプト。

定義は docs/plans/archived/nnue-material-linear-init.md の「麒麟の診断」に従う。
evaluate_games に渡す評価器は、SFENの列を受け取り、同じ順の
(評価値cp, 非線形部分cp) の列を返す。テストでは外部プロセスを必要としない。
棋譜は最後まで再生し、失敗した対局からは途中の局面も採用しない。
再生は着手の表記、手番、駒の存在と捕獲を検査し、移動の合法性は判定しない。

JSONの positions は麒麟が残っている局面を保持し、Rに使えない局面も含む。
counts の excluded_positions は、再生に成功した対局の採取時点のうち、
麒麟がない局面と、3筋・7筋を同じ段で比べられない局面の合計である。
Rの対象が空ならRはnull、表の対象が空なら各偏差はnullとする。
judge は保存済みのRを使わず、生の評価値から再計算する。
"""

import argparse
import hashlib
import json
import math
import random
import re
import statistics
import subprocess
import tempfile
import time
from collections import defaultdict
from pathlib import Path


START = (
    "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/"
    "3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
)
RANKS = "abcdefghijkl"
PLIES = (20, 21, 40, 41, 60, 61)
MOVE = re.compile(r"(?:([1-9]|1[0-2])[a-l]){2,3}\+?")
SQUARE = re.compile(r"([1-9]|1[0-2])([a-l])")


class ReplayError(ValueError):
    """対局全体を除外する棋譜の再生失敗。"""


def parse_board(sfen_board):
    """SFENの盤面を (盤の筋, 0始まりの段) -> 駒の辞書にする。"""
    rows = sfen_board.split("/")
    if len(rows) != 12:
        raise ValueError("SFEN must have 12 ranks")
    board = {}
    for rank, row in enumerate(rows):
        tokens = re.findall(r"[0-9]+|\+?[a-zA-Z]", row)
        if "".join(tokens) != row:
            raise ValueError("invalid SFEN board token")
        file = 12
        for token in tokens:
            if token.isdigit():
                if not 1 <= int(token) <= 12:
                    raise ValueError("invalid SFEN empty-square count")
                file -= int(token)
            else:
                if file < 1:
                    raise ValueError("SFEN rank is too long")
                board[file, rank] = token
                file -= 1
        if file != 0:
            raise ValueError("SFEN rank must contain 12 squares")
    return board


def emit_board(board):
    rows = []
    for rank in range(12):
        row = []
        empty = 0
        for file in range(12, 0, -1):
            if (file, rank) not in board:
                empty += 1
                continue
            if empty:
                row.append(str(empty))
                empty = 0
            row.append(board[file, rank])
        if empty:
            row.append(str(empty))
        rows.append("".join(row))
    return "/".join(rows)


def apply_move(board, kirins, move, side):
    if MOVE.fullmatch(move) is None:
        raise ReplayError(f"invalid move: {move}")
    squares = [(int(f), RANKS.index(r)) for f, r in SQUARE.findall(move)]
    origin, destination = squares[0], squares[-1]
    if origin not in board:
        raise ReplayError(f"empty origin: {move}")
    piece = board.pop(origin)
    if piece.isupper() != (side == "b"):
        raise ReplayError(f"wrong side to move: {move}")
    promotes = move.endswith("+")
    if promotes and piece.startswith("+"):
        raise ReplayError(f"piece already promoted: {move}")
    moving_kirin = kirins[side] == origin
    for square in squares[1:]:
        if square in board:
            if board[square].isupper() == piece.isupper():
                raise ReplayError(f"capture of own piece: {move}")
            del board[square]
            for owner in kirins:
                if kirins[owner] == square:
                    kirins[owner] = None
    board[destination] = "+" + piece if promotes else piece
    if moving_kirin:
        kirins[side] = None if promotes else destination


def replay_game(game_id, moves):
    board = parse_board(START)
    kirins = {"b": (7, 10), "w": (6, 1)}
    positions = []
    missing = 0
    for ply, move in enumerate(moves, 1):
        side = "b" if ply % 2 else "w"
        apply_move(board, kirins, move, side)
        if ply in PLIES:
            turn = "w" if ply % 2 else "b"
            if kirins[turn] is None:
                missing += 1
            else:
                positions.append({
                    "game_id": game_id,
                    "ply": ply,
                    "sfen": f"{emit_board(board)} {turn} - 1",
                })
    return positions, missing


def select_positions(games):
    positions = []
    counts = dict(input_games=0, filtered_games=0, eligible_games=0,
                  replay_failed_games=0, replayed_games=0,
                  excluded_positions_missing_kirin=0)
    seen = set()
    for game in games:
        counts["input_games"] += 1
        if "initialSfen" in game or not game.get("moves"):
            counts["filtered_games"] += 1
            continue
        players = game["players"]
        human = all(
            side in players
            and "aiLevel" not in players[side]
            and isinstance(players[side].get("user"), dict)
            and players[side]["user"].get("title") != "BOT"
            for side in ("sente", "gote")
        )
        moves = game["moves"].split()
        if not human or len(moves) < 62:
            counts["filtered_games"] += 1
            continue
        game_id = game["id"]
        if not isinstance(game_id, str) or not game_id or game_id in seen:
            raise ValueError(f"missing or duplicate game ID: {game_id!r}")
        seen.add(game_id)
        counts["eligible_games"] += 1
        try:
            selected, missing = replay_game(game_id, moves)
        except ReplayError:
            counts["replay_failed_games"] += 1
            continue
        counts["replayed_games"] += 1
        counts["excluded_positions_missing_kirin"] += missing
        positions.extend(selected)
    return positions, counts


def relocations(sfen):
    """自陣4段内の、空き升または現在地で、同じ色の升だけを返す。"""
    board_text, side, _, _ = sfen.split()
    if side not in ("b", "w"):
        raise ValueError("invalid side to move")
    board = parse_board(board_text)
    piece = "O" if side == "b" else "o"
    origins = [square for square, p in board.items() if p == piece]
    if len(origins) != 1:
        raise ValueError("expected one unpromoted kirin for the side to move")
    origin = origins[0]
    targets = []
    for rank in range(1, 5):
        board_rank = 12 - rank if side == "b" else rank - 1
        for file in range(1, 13):
            board_file = file if side == "b" else 13 - file
            square = (board_file, board_rank)
            if square != origin and square in board:
                continue
            if sum(square) % 2 != sum(origin) % 2:
                continue
            moved = board.copy()
            del moved[origin]
            moved[square] = piece
            targets.append((file, rank, f"{emit_board(moved)} {side} - 1"))
    return targets


def parse_evaluations(lines, expected):
    """evalの2種類の整数行を順に対応させ、欠落・余分・エラーを拒否する。"""
    values = []
    pending = None
    usi_ok = ready_ok = False
    for line in lines:
        line = line.strip()
        if line.startswith("info string error"):
            raise ValueError(f"minase rejected a command: {line}")
        if line == "usiok":
            usi_ok = True
        elif line == "readyok":
            ready_ok = True
        elif line.startswith("info string evaluation "):
            if pending is not None:
                raise ValueError("missing nonlinear evaluation")
            pending = int(line.removeprefix("info string evaluation "))
        elif line.startswith("info string nonlinear "):
            if pending is None:
                raise ValueError("nonlinear output without an evaluation")
            values.append((pending, int(line.removeprefix("info string nonlinear "))))
            pending = None
    if not usi_ok or not ready_ok:
        raise ValueError("missing USI handshake response")
    if pending is not None or len(values) != expected:
        raise ValueError(f"evaluation count mismatch: expected {expected}, got {len(values)}")
    return values


def usi_evaluate(binary, sfens):
    """1プロセスで評価し、大量の盤面診断行をメモリに蓄積せずに読む。"""
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as commands, \
            tempfile.TemporaryFile(mode="w+", encoding="utf-8") as errors:
        commands.write("usi\nisready\nusinewgame\n")
        expected = 0
        for sfen in sfens:
            commands.write(f"position sfen {sfen}\neval\n")
            expected += 1
        commands.write("quit\n")
        commands.seek(0)
        with subprocess.Popen(
            [str(binary), "--protocol", "usi", "--rules", "engine-default"],
            stdin=commands, stdout=subprocess.PIPE, stderr=errors,
            text=True, encoding="utf-8",
        ) as process:
            try:
                values = parse_evaluations(process.stdout, expected)
            except BaseException:
                process.kill()
                raise
            if process.wait() != 0:
                errors.seek(0)
                raise ValueError(f"minase exited with {process.returncode}: {errors.read()}")
    return values


def position_d(position):
    targets = {(t["file"], t["rank"]): t["evaluation_cp"] for t in position["targets"]}
    differences = [targets[3, rank] - targets[7, rank] for rank in range(1, 5)
                   if (3, rank) in targets and (7, rank) in targets]
    return statistics.fmean(differences) if differences else None


def file_table(positions):
    """局面内で全候補を中心化し、標本20以上の升の標本を筋ごとにまとめる。"""
    cells = defaultdict(list)
    for position in positions:
        targets = position["targets"]
        if not targets:
            continue
        mean = statistics.fmean(t["evaluation_cp"] for t in targets)
        nonlinear_mean = statistics.fmean(t["nonlinear_cp"] for t in targets)
        for target in targets:
            evaluation = target["evaluation_cp"] - mean
            nonlinear = target["nonlinear_cp"] - nonlinear_mean
            cells[target["file"], target["rank"]].append(
                (evaluation, nonlinear, evaluation - nonlinear))
    table = []
    for file in range(12, 0, -1):
        values = [value for (f, _), cell in cells.items()
                  if f == file and len(cell) >= 20 for value in cell]
        row = {"file": file, "n": len(values)}
        for i, name in enumerate(("evaluation_cp", "nonlinear_cp", "linear_cp")):
            row[name] = statistics.fmean(v[i] for v in values) if values else None
        table.append(row)
    return table


def evaluate_games(games, evaluator):
    positions, counts = select_positions(games)
    sfens = []
    all_targets = []
    for position in positions:
        position["targets"] = []
        for file, rank, sfen in relocations(position["sfen"]):
            target = {"file": file, "rank": rank}
            position["targets"].append(target)
            all_targets.append(target)
            sfens.append(sfen)
    values = list(evaluator(sfens))
    if len(values) != len(all_targets):
        raise ValueError(f"evaluation count mismatch: expected {len(all_targets)}, got {len(values)}")
    for target, (evaluation, nonlinear) in zip(all_targets, values):
        if type(evaluation) is not int or type(nonlinear) is not int:
            raise ValueError("evaluations must be integer cp values")
        target.update(evaluation_cp=evaluation, nonlinear_cp=nonlinear)
    ds = [(p["game_id"], position_d(p)) for p in positions]
    usable = [(game_id, d) for game_id, d in ds if d is not None]
    counts.update(positions=len(positions), games=len({p["game_id"] for p in positions}),
                  evaluations=len(values), r_positions=len(usable),
                  r_games=len({game_id for game_id, _ in usable}),
                  excluded_positions_no_pair=len(positions) - len(usable))
    counts["excluded_positions"] = (counts["excluded_positions_missing_kirin"]
                                    + counts["excluded_positions_no_pair"])
    return {"R": statistics.fmean(d for _, d in usable) if usable else None,
            "counts": counts, "file_table": file_table(positions), "positions": positions}


def index_positions(result):
    indexed = {}
    for position in result["positions"]:
        game_id, ply = position["game_id"], position["ply"]
        if not isinstance(game_id, str) or not game_id or type(ply) is not int or ply not in PLIES:
            raise ValueError("invalid game ID or sampled ply")
        key = (game_id, ply)
        if key in indexed:
            raise ValueError(f"duplicate position: {key}")
        if not isinstance(position["sfen"], str) or not position["sfen"]:
            raise ValueError(f"missing SFEN: {key}")
        squares = set()
        for target in position["targets"]:
            file, rank = target["file"], target["rank"]
            if type(file) is not int or type(rank) is not int or not 1 <= file <= 12 or not 1 <= rank <= 4:
                raise ValueError(f"invalid target square: {key}")
            square = (file, rank)
            if square in squares:
                raise ValueError(f"duplicate target square: {key}, {square}")
            squares.add(square)
            for field in ("evaluation_cp", "nonlinear_cp"):
                if type(target[field]) is not int:
                    raise ValueError(f"invalid integer cp value: {key}, {field}")
        indexed[key] = (position, squares)
    return indexed


def percentile(values, probability):
    """昇順のn値の添字 (n−1)p を両隣の値で線形補間する（type 7）。"""
    if not values or not 0 <= probability <= 1:
        raise ValueError("percentile needs values and a probability in [0, 1]")
    ordered = sorted(values)
    index = (len(ordered) - 1) * probability
    low, high = math.floor(index), math.ceil(index)
    return ordered[low] + (ordered[high] - ordered[low]) * (index - low)


def judge(current, new):
    """Rに寄与する対局を1,000回復元抽出し、対応を保って95%区間を作る。

    乱数はrandom.Random(1)、対局の順はIDの辞書順とする。
    各対局のdの和と局面数を使うため、復元抽出時の重複と局面の等重みを保つ。
    dが定義できる局面を1つも持たない対局は、Rの標本から除く。
    区間の両端はpercentileの線形補間による2.5、97.5パーセンタイル点。
    """
    old_index, new_index = index_positions(current), index_positions(new)
    if old_index.keys() != new_index.keys():
        raise ValueError("position sets differ between the two JSON files")
    groups = defaultdict(list)
    for key in sorted(old_index):
        old_position, old_squares = old_index[key]
        new_position, new_squares = new_index[key]
        if old_position["sfen"] != new_position["sfen"]:
            raise ValueError(f"position SFEN differs: {key}")
        if old_squares != new_squares:
            raise ValueError(f"relocation target sets differ: {key}")
        old_d, new_d = position_d(old_position), position_d(new_position)
        if old_d is not None:
            groups[key[0]].append((old_d, new_d))
    if not groups:
        raise ValueError("R is undefined: no positions have matched files 3 and 7")
    totals = [(math.fsum(a for a, _ in values), math.fsum(b for _, b in values), len(values))
              for _, values in sorted(groups.items())]

    def estimates(sample):
        n = sum(count for _, _, count in sample)
        old_r = math.fsum(a for a, _, _ in sample) / n
        new_r = math.fsum(b for _, b, _ in sample) / n
        return old_r, new_r, new_r - 0.5 * old_r

    point = estimates(totals)
    rng = random.Random(1)
    draws = [estimates(rng.choices(totals, k=len(totals))) for _ in range(1000)]
    intervals = [[percentile([draw[i] for draw in draws], p) for p in (0.025, 0.975)]
                 for i in range(3)]
    if intervals[0][0] <= 0:
        decision = "判定不能（R_curが正と確かめられない）"
    elif intervals[2][0] > 0:
        decision = "半分より多くが残る"
    elif intervals[2][1] < 0:
        decision = "半分より多くは残らない"
    else:
        decision = "判定不能"
    result = {name: {"estimate": value, "ci95": interval}
              for name, value, interval in zip(("R_cur", "R_new", "D"), point, intervals)}
    result.update(decision=decision, games=len(totals),
                  positions=sum(n for _, _, n in totals), bootstrap_replicates=1000,
                  bootstrap_seed=1, percentile_method="linear interpolation at (n-1)*p (type 7)")
    return result


def read_games(path):
    with path.open(encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            try:
                yield json.loads(line)
            except json.JSONDecodeError as error:
                raise ValueError(f"invalid NDJSON at line {line_number}: {error}") from error


def write_json(path, result):
    with path.open("w", encoding="utf-8") as destination:
        json.dump(result, destination, ensure_ascii=False, indent=2, allow_nan=False)
        destination.write("\n")


def print_evaluation(result):
    c = result["counts"]
    print(f"局面数 {c['positions']:,}、対局数 {c['games']:,}、Rの対象 {c['r_positions']:,}局面 / {c['r_games']:,}局")
    print(f"除外局面 {c['excluded_positions']:,}（麒麟なし {c['excluded_positions_missing_kirin']:,}、"
          f"比較可能な段なし {c['excluded_positions_no_pair']:,}）、再生失敗 {c['replay_failed_games']:,}局")
    print("R = 未定義" if result["R"] is None else f"R = {result['R']:.6f} cp")
    table = result["file_table"]
    print("| 持ち主から見た筋 | " + " | ".join(str(row["file"]) for row in table) + " |")
    print("|---|" + "---|" * 12)
    for label, key in (("評価値（cp）", "evaluation_cp"), ("非線形部分（cp）", "nonlinear_cp"),
                       ("線形経路（cp）", "linear_cp"), ("標本数", "n")):
        values = [(str(row[key]) if key == "n" else f"{row[key]:+.2f}")
                  if row[key] is not None else "." for row in table]
        print(f"| {label} | " + " | ".join(values) + " |")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    evaluate_parser = commands.add_parser("evaluate", help="棋譜を再生し、麒麟を置き直して評価する")
    evaluate_parser.add_argument("binary", type=Path)
    evaluate_parser.add_argument("games", type=Path)
    evaluate_parser.add_argument("output", type=Path)
    judge_parser = commands.add_parser("judge", help="同一局面の2本のネットを比較する")
    judge_parser.add_argument("current", type=Path)
    judge_parser.add_argument("new", type=Path)
    judge_parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "evaluate":
            started = time.monotonic()
            binary = args.binary.resolve(strict=True)
            with binary.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
            result = evaluate_games(read_games(args.games), lambda sfens: usi_evaluate(binary, sfens))
            result.update(binary=str(binary), binary_sha256=digest,
                          games_path=str(args.games.resolve()), elapsed_seconds=time.monotonic() - started)
            write_json(args.output, result)
            print_evaluation(result)
            print(f"所要時間 {result['elapsed_seconds']:.3f}秒（JSONの書き込みを除く）")
        else:
            current = json.loads(args.current.read_text(encoding="utf-8"))
            new = json.loads(args.new.read_text(encoding="utf-8"))
            result = judge(current, new)
            for name in ("R_cur", "R_new", "D"):
                value = result[name]
                lo, hi = value["ci95"]
                print(f"{name} = {value['estimate']:.6f} cp、95%区間 [{lo:.6f}, {hi:.6f}] cp")
            print(result["decision"])
            if args.output is not None:
                write_json(args.output, result)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, f"error: {error}\n")


if __name__ == "__main__":
    main()
