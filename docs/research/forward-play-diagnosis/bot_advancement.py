"""lishogiの棋譜から、minase-botと人間の相手の非王駒の平均前進段数を版ごとに数える。

使い方: python3 bot_advancement.py <games.ndjson>
入力は /api/games/user/minase-bot?perfType=chushogi の書き出し（1行1局）。
"""
import collections, datetime, json, re, statistics, sys

START = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
# Botログの`>> id name`から読んだ版の切替時刻（UTC）。
SWITCHES = [("2026-09-22 01:57", "1.1.0"), ("2026-10-03 06:10", "1.3.0"), ("2026-10-04 08:36", "2.0.0"),
            ("2026-10-08 00:43", "2.2.0"), ("2026-10-08 15:52", "2.3.0")]
SWITCHES = [(datetime.datetime.strptime(t, "%Y-%m-%d %H:%M").replace(tzinfo=datetime.timezone.utc), v) for t, v in SWITCHES]
CHECKPOINTS = (60, 100, 140)


def parse(sfen):
    board = {}
    for rank, row in enumerate(sfen.split("/")):
        file, i = 12, 0
        while i < len(row):
            digits = re.match(r"\d+", row[i:])
            if digits:
                file -= int(digits.group()); i += len(digits.group()); continue
            piece = row[i]; i += 1
            if piece == "+":
                piece += row[i]; i += 1
            board[(file, rank)] = piece; file -= 1
    return board


def square(text):
    return int(text[:-1]), ord(text[-1]) - 97


acc = collections.defaultdict(lambda: collections.defaultdict(list))
for line in open(sys.argv[1]):
    game = json.loads(line); players = game["players"]
    if game["status"] == "aborted" or game.get("initialSfen"):
        continue
    created = datetime.datetime.fromtimestamp(game["createdAt"] / 1000, datetime.timezone.utc)
    versions = [v for t, v in SWITCHES if t <= created]
    if not versions:
        continue
    bot_is_sente = players["sente"].get("user", {}).get("id") == "minase-bot"
    board = parse(START)
    for ply, move in enumerate(game["moves"].split(), 1):
        squares = re.findall(r"\d{1,2}[a-l]", move)
        origin = square(squares[0]); piece = board.pop(origin)
        for middle in map(square, squares[1:-1]):
            if middle != origin:
                board.pop(middle, None)
        board[square(squares[-1])] = piece
        if ply not in CHECKPOINTS:
            continue
        for side in ("bot", "opp"):
            sente = bot_is_sente if side == "bot" else not bot_is_sente
            depths = [11 - r if sente else r for (f, r), p in board.items()
                      if p.lstrip("+").upper() != "K" and p.lstrip("+").isupper() == sente]
            acc[(versions[-1], side)][ply].append((statistics.mean(depths), sum(d >= 6 for d in depths)))

for key in sorted(acc):
    print(key, {ply: (len(rows), round(statistics.mean(a for a, _ in rows), 2), round(statistics.mean(c for _, c in rows), 1))
                for ply, rows in sorted(acc[key].items())})
