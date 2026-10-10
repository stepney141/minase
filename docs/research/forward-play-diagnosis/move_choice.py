r"""Botの棋譜の局面で、指した手・静的評価の1位・合法な静かな手のうち前進手が占める割合を比べる。

手順（リポジトリのルートで実行する）:
  1. python3 move_choice.py prepare <games.ndjson> <bot.ndjson>
     取り込みツールはBotの対局と非レーティング戦を除くので、診断用の写しでBOTの称号を消し、
     レーティング欄を埋める。初期局面から始まり、投了または王駒の捕獲で終わった時間制の対局だけを残す。
  2. target/release/minase data lishogi games --input <bot.ndjson> --output <bot.bin> \
         --report <bot.json> --nodes 2000 --concurrency 4 --seed 99 --allow-dirty
  3. target/release/minase dev pst-probe --pst crates/minase/nets/pst.bin --positions <bot.bin> --moves \
         | perl -pe 's/,"after":\{[^}]*\}//g; s/,"after_same":.*?,"turn_pst":-?\d+//g' > <moves.json>
     局面ごとの着手後の盤面と分解成分を落とし、着手・捕獲・成り・終局・評価差だけを残す。
  4. uv run --project tools/train python move_choice.py analyze <bot.ndjson> <bot.bin> <bot.json> <moves.json> [版]
     版を指定すると、その版のBotが指した対局の局面だけを集計する。手順3の重みをその版の
     crates/minase/nets/pst.bin にそろえれば、静的評価と探索が同じ評価関数による比較になる。
"""
import collections, datetime, json, re, statistics, sys
import numpy as np

SWITCHES = [("2026-09-18 05:01", "0.1.0"), ("2026-09-20 14:32", "1.0.0"), ("2026-09-22 01:57", "1.1.0"), ("2026-10-03 06:10", "1.3.0"), ("2026-10-04 08:36", "2.0.0"),
            ("2026-10-08 00:43", "2.2.0"), ("2026-10-08 15:52", "2.3.0")]
SWITCHES = [(datetime.datetime.strptime(t, "%Y-%m-%d %H:%M").replace(tzinfo=datetime.timezone.utc), v) for t, v in SWITCHES]
KING = 11  # PieceKind::King


def prepare(source, target):
    with open(target, "w") as out:
        for line in open(source):
            game = json.loads(line)
            if game.get("initialSfen") or not game.get("clock") or not game.get("winner") \
                    or game["status"] not in ("resign", "royalsLost"):
                continue
            if any("user" not in game["players"][c] for c in ("sente", "gote")):
                continue
            for color in ("sente", "gote"):
                player = game["players"][color]
                player["user"].pop("title", None)
                player["rating"] = player.get("rating") or 1500
                player["provisional"] = False
            game["rated"] = True
            out.write(json.dumps(game) + "\n")


def version_of(game):
    created = datetime.datetime.fromtimestamp(game["createdAt"] / 1000, datetime.timezone.utc)
    return [v for t, v in SWITCHES if t <= created][-1]


def squares(move):
    return [(int(s[:-1]), ord(s[-1]) - 97) for s in re.findall(r"\d{1,2}[a-l]", move)]


def analyze(games_path, records_path, report_path, moves_path, only_version=None):
    from minase_train.data.mnsd import map_records
    ids = json.load(open(report_path))["accepted_ids"]
    games = {json.loads(l)["id"]: json.loads(l) for l in open(games_path)}
    records = map_records(records_path)
    stats = collections.defaultdict(collections.Counter); gaps = collections.defaultdict(list)
    for position in json.load(open(moves_path)):
        record = records[position["index"]]
        game = games[ids[record["game"] - 1]]; ply = int(record["ply"]); stm = int(record["stm"])
        if only_version and version_of(game) != only_version:
            continue
        if not 20 <= ply < 150 or ply >= len(game["moves"].split()):
            continue
        played = game["moves"].split()[ply]
        legal = position["moves"]
        if played not in [m["move"] for m in legal]:
            continue
        board = record["board"]

        def kind(move):
            file, rank = squares(move)[0]
            return (int(board[(11 - rank) * 12 + (12 - file)]) - 1) % 64

        def forward(move):  # 成りを選ばずに、手番側から見て1段以上前へ進む手。
            (_, start), (_, end) = squares(move["move"])[0], squares(move["move"])[-1]
            return (start - end if stm == 0 else end - start) > 0 and not move["promote"]

        bot_sente = game["players"]["sente"]["user"]["id"] == "minase-bot"
        key = ("bot", version_of(game)) if (stm == 0) == bot_sente else ("human", only_version or "all")
        chosen = next(m for m in legal if m["move"] == played)
        if chosen["capture"]:
            continue
        quiet = [m for m in legal if not m["capture"] and not m["terminal"] and kind(m["move"]) != KING]
        if not quiet or kind(played) == KING:
            continue
        s = stats[key]; s["n"] += 1
        best = max(quiet, key=lambda m: m["delta"]); best_pst = max(quiet, key=lambda m: m["delta_pst"])
        s["played"] += forward(chosen); s["best"] += forward(best); s["best_pst"] += forward(best_pst)
        s["legal"] += sum(map(forward, quiet)) / len(quiet)
        s["best_f_played_not"] += forward(best) and not forward(chosen)
        s["best_not_played_f"] += forward(chosen) and not forward(best)
        f = [m for m in quiet if forward(m)]; nf = [m for m in quiet if not forward(m)]
        if f and nf:
            gaps[key].append((max(m["delta"] for m in f) - max(m["delta"] for m in nf),
                              max(m["delta_pst"] for m in f) - max(m["delta_pst"] for m in nf)))
    for key in sorted(stats):
        s = stats[key]; n = s["n"]; g = np.array(gaps[key])
        print(f"{key} 局面{n}: 指した手 {s['played']/n:.1%}, 静的評価1位 {s['best']/n:.1%}（PSTのみ {s['best_pst']/n:.1%}）, "
              f"合法な静かな手 {s['legal']/n:.1%}, 1位前進→非前進 {s['best_f_played_not']/n:.1%}, "
              f"1位非前進→前進 {s['best_not_played_f']/n:.1%}, 最良前進−最良非前進の中央値 "
              f"全体{np.median(g[:, 0]):+.0f}cp PSTのみ{np.median(g[:, 1]):+.0f}cp")


if sys.argv[1] == "prepare":
    prepare(*sys.argv[2:4])
else:
    analyze(*sys.argv[2:7])
