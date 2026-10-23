"""Lishogi-Botのログから、ByoyomiMarginを変えた後の秒読みつき対局の記録を集計する。

使い方:
    python3 -I byoyomi_record.py <lishogi-bot.log> "<切替時刻 YYYY-MM-DD HH:MM:SS>"

切替時刻より後に始まって終局した秒読みつきの対局を対象にし、対局ごとの表と全体の集計を
標準出力へ書く。時刻はログの時刻（UTC）である。

エンジンの`go`、`ponderhit`、`bestmove`の行は対局IDを持たないので、探索の開始から
`bestmove`までの間に進行中の対局が1つだけだった手に限って対局と結び付け、秒読み期の
思考時間を求める。着手の数と送信の時間は、対局IDを持つ`POST .../move/`の行から数える。
"""

import ast
import re
import statistics
import sys
from datetime import datetime

TS_FMT = "%Y-%m-%d %H:%M:%S,%f"
RE_GO = re.compile(r"engine_ctrl\.usi DEBUG << go (.*)$")
RE_BESTMOVE = re.compile(r"engine_ctrl\.usi DEBUG >> bestmove (\S+)")
RE_VERSION = re.compile(r"engine_ctrl\.usi DEBUG >> id name minase (\S+)")
RE_MARGIN = re.compile(r"engine_ctrl\.usi DEBUG << setoption name ByoyomiMargin value (\d+)")
RE_PLAYING = re.compile(r"__main__ INFO \+\+\+ Playing https://lishogi\.org/(\w{8})/")
# 通信対局は終局を待たずに対局の処理を抜けるので、終局の通知のほかに処理の終了でも外す。
RE_LEAVE = re.compile(
    r"__main__ INFO --- (?:https://lishogi\.org/(\w{8})/\S+ Game over"
    r"|Disconnecting from https://lishogi\.org/(\w{8})/)"
)
RE_POST_REQ = re.compile(r"lishogi DEBUG POST https://lishogi\.org/api/bot/game/(\w{8})/move/(\S+)$")
RE_POST_RES = re.compile(r'"POST /api/bot/game/(\w{8})/move/(\S+) HTTP/1\.1" (\d+)')

# bestmoveと着手の送信を結び付ける時間の幅。
MATCH_WINDOW_MS = 5000
# 締切まで考えたとみなす幅。思考時間が「秒読み − 余裕 − この値」以上の手を数える。
NEAR_DEADLINE_MS = 100


def timestamp(line):
    return datetime.strptime(line[:23], TS_FMT)


def ms(delta):
    return delta.total_seconds() * 1000.0


def go_fields(args):
    tokens = args.split()
    fields = {"ponder": "ponder" in tokens}
    for key in ("btime", "wtime", "byoyomi", "movetime"):
        if key in tokens:
            fields[key] = int(tokens[tokens.index(key) + 1])
    return fields


def parse_dict(line):
    return ast.literal_eval(line[line.index("{"):])


def read_log(path, cutoff):
    """ログを1回読み、切替時刻の前に始まった対局と、切替後の出来事を集める。"""
    before_cutoff_games = set()
    margins = set()
    games = {}
    in_progress = set()
    pending_version = None
    last_go_fields = None
    # 直近の探索の開始。(時刻, goの引数, 種類) で、種類は "go"、"ponder"、"ponderhit"。
    last_start = None
    # 探索の開始から現在まで、進行中の対局が1つだけだったか。
    solo = False
    bestmoves = []
    posts = []
    responses = []
    after = False

    with open(path, errors="replace") as f:
        for line in f:
            if not after:
                if line[:1].isdigit() and line[:19] >= cutoff:
                    after = True
                else:
                    if "'type': 'gameFull'" in line and "__main__ DEBUG {'id': '" in line:
                        before_cutoff_games.add(parse_dict(line)["id"])
                    continue
            if "Welcome minase-bot" in line:
                in_progress.clear()
                last_go_fields = last_start = None
                continue
            m = RE_MARGIN.search(line)
            if m:
                margins.add(int(m.group(1)))
                continue
            m = RE_VERSION.search(line)
            if m:
                pending_version = m.group(1)
                continue
            m = RE_LEAVE.search(line)
            if m:
                in_progress.discard(m.group(1) or m.group(2))
                continue
            m = RE_PLAYING.search(line)
            if m:
                gid = m.group(1)
                in_progress.add(gid)
                versions = games.setdefault(gid, {}).setdefault("versions", [])
                if pending_version and pending_version not in versions:
                    versions.append(pending_version)
                continue
            if "'type': 'gameFull'" in line and "__main__ DEBUG {'id': '" in line:
                d = parse_dict(line)
                g = games.setdefault(d["id"], {})
                if "full" not in g:
                    g["full"] = d
                    g["start"] = timestamp(line)
                continue
            if "Event: {'type': 'gameFinish'" in line:
                d = parse_dict(line)["game"]
                games.setdefault(d["gameId"], {})["finish"] = d
                in_progress.discard(d["gameId"])
                continue
            m = RE_GO.search(line)
            if m:
                fields = go_fields(m.group(1))
                last_go_fields = fields
                last_start = (timestamp(line), fields, "ponder" if fields["ponder"] else "go")
                solo = len(in_progress) == 1
                continue
            if "engine_ctrl.usi DEBUG << ponderhit" in line:
                if last_go_fields is not None:
                    last_start = (timestamp(line), last_go_fields, "ponderhit")
                    solo = solo and len(in_progress) == 1
                continue
            m = RE_BESTMOVE.search(line)
            if m:
                if last_start is not None:
                    bestmoves.append({
                        "time": timestamp(line),
                        "move": m.group(1),
                        "solo": solo and len(in_progress) == 1,
                        "start": last_start[0],
                        "fields": last_start[1],
                        "kind": last_start[2],
                    })
                last_start = None
                solo = False
                continue
            m = RE_POST_REQ.search(line)
            if m:
                posts.append({"time": timestamp(line), "game": m.group(1), "move": m.group(2)})
                continue
            m = RE_POST_RES.search(line)
            if m:
                responses.append((timestamp(line), m.group(1), m.group(2), int(m.group(3))))
                continue

    return before_cutoff_games, margins, games, bestmoves, posts, responses


def link_moves(bestmoves, posts, responses):
    """送信をその直前のbestmoveと応答に結び付ける。結び付かない送信も残す。"""
    used = set()
    first = 0
    for post in posts:
        while first < len(bestmoves) and ms(post["time"] - bestmoves[first]["time"]) > MATCH_WINDOW_MS:
            first += 1
        match = None
        for i in range(first, len(bestmoves)):
            if bestmoves[i]["time"] > post["time"]:
                break
            if i not in used and bestmoves[i]["move"] == post["move"]:
                match = i
        if match is not None:
            used.add(match)
            post["bestmove"] = bestmoves[match]
    by_key = {}
    for t, gid, move, code in responses:
        by_key.setdefault((gid, move), []).append((t, code))
    for post in posts:
        later = [r for r in by_key.get((post["game"], post["move"]), []) if r[0] >= post["time"]]
        if later:
            post["response"], post["code"] = min(later)


def byoyomi_phase_think(post, game, margin):
    """秒読み期の手で対局と結び付けられた手なら (思考時間, 種類) を返す。"""
    bm = post.get("bestmove")
    if bm is None or not bm["solo"]:
        return None
    clock = game["full"]["clock"]
    own = "btime" if game["finish"]["color"] == "sente" else "wtime"
    f = bm["fields"]
    if "movetime" in f or f.get(own) != 0 or f.get("byoyomi") != clock["byoyomi"]:
        return None
    return ms(bm["time"] - bm["start"]), bm["kind"]


def main(path, cutoff):
    before_cutoff_games, margins, games, bestmoves, posts, responses = read_log(path, cutoff)
    link_moves(bestmoves, posts, responses)
    if len(margins) != 1:
        sys.exit(f"切替後のByoyomiMarginが1つの値ではない: {sorted(margins)}")
    margin = margins.pop()

    byoyomi_games = [
        gid for gid, g in games.items()
        if "full" in g and gid not in before_cutoff_games
        and (g["full"].get("clock") or {}).get("byoyomi")
    ]
    unfinished = [gid for gid in byoyomi_games if "finish" not in games[gid]]
    aborted = [gid for gid in byoyomi_games if gid not in unfinished and (
        games[gid]["finish"]["status"]["name"] == "aborted" or not games[gid]["finish"].get("hasMoved"))]
    targets = sorted(
        (gid for gid in byoyomi_games if gid not in unfinished and gid not in aborted),
        key=lambda gid: games[gid]["start"],
    )
    target_set = set(targets)
    posts_by_game = {}
    for post in posts:
        posts_by_game.setdefault(post["game"], []).append(post)

    print(f"切替時刻: {cutoff}、切替後のByoyomiMargin: {margin} ms")
    print(f"切替後に始まった秒読みつきの対局: {len(byoyomi_games)}局"
          f"（中止 {len(aborted)}局、終局の通知なし {len(unfinished)}局、対象 {len(targets)}局）")
    print()
    print("| 番号 | 対局 | 開始（UTC） | 版 | 持ち時間 | 秒読み | 手番 | 結果 | 終局理由 "
          "| Botの着手 | 秒読み期の手（解析） | 送信の最大 |")
    print("|---:|---|---|---|---:|---:|---|---|---|---:|---:|---:|")
    by_byoyomi = {}
    slack = []
    results = {}
    versions = {}
    forfeits = {"Bot": 0, "相手": 0}
    games_with_phase = 0
    for n, gid in enumerate(targets, 1):
        g = games[gid]
        clock, fin = g["full"]["clock"], g["finish"]
        winner = fin.get("winner")
        result = "引き分け" if winner is None else ("勝ち" if winner == fin["color"] else "負け")
        results[result] = results.get(result, 0) + 1
        version = "/".join(g.get("versions", []))
        versions[version] = versions.get(version, 0) + 1
        status = fin["status"]["name"]
        if status == "outoftime":
            forfeits["相手" if result == "勝ち" else "Bot"] += 1
        phase = 0
        for post in posts_by_game.get(gid, []):
            think = byoyomi_phase_think(post, g, margin)
            if think is None:
                continue
            phase += 1
            by_byoyomi.setdefault(clock["byoyomi"], []).append(think)
            if "response" in post:
                total = ms(post["response"] - post["bestmove"]["start"])
                slack.append((clock["byoyomi"] - total, gid, think[1], think[0],
                              ms(post["response"] - post["bestmove"]["time"])))
        games_with_phase += phase > 0
        sends = [ms(p["response"] - p["bestmove"]["time"])
                 for p in posts_by_game.get(gid, []) if "bestmove" in p and "response" in p]
        max_send = f"{max(sends):,.0f} ms" if sends else "-"
        print(f"| {n} | {gid} | {g['start']:%m-%d %H:%M} | {version} | {clock['initial'] // 60000}分 "
              f"| {clock['byoyomi'] // 1000}秒 | {'先手' if fin['color'] == 'sente' else '後手'} | {result} "
              f"| {status} | {len(posts_by_game.get(gid, []))} | {phase} | {max_send} |")

    print()
    print(f"版ごとの対局数: {versions}")
    print(f"結果: {results}")
    print(f"時間切れ: Bot {forfeits['Bot']}局、相手 {forfeits['相手']}局")
    print()
    total = sum(len(v) for v in by_byoyomi.values())
    ponderhit = sum(1 for v in by_byoyomi.values() for t in v if t[1] == "ponderhit")
    near = sum(1 for b, v in by_byoyomi.items() for t in v if t[0] >= b - margin - NEAR_DEADLINE_MS)
    print(f"秒読み期の手を解析できた対局: {games_with_phase}局、手: {total}手（うち先読みの的中 {ponderhit}手）")
    print(f"締切（秒読み − {margin} ms）の{NEAR_DEADLINE_MS} ms手前以降まで考えた手: {near}手")
    print()
    print("| 秒読み | 手数 | 思考時間の中央値 | 最小 | 最大 | 中央値/秒読み |")
    print("|---:|---:|---:|---:|---:|---:|")
    for b in sorted(by_byoyomi):
        v = [t[0] for t in by_byoyomi[b]]
        print(f"| {b // 1000}秒 | {len(v)} | {statistics.median(v):,.0f} ms | {min(v):,.0f} ms "
              f"| {max(v):,.0f} ms | {statistics.median(v) / b:.3f} |")
    print()
    print("締切までの残り（秒読み − 探索の開始から送信の応答まで）が小さい5手:")
    for s in sorted(slack)[:5]:
        print(f"  残り {s[0]:,.0f} ms、対局 {s[1]}、{s[2]}、思考 {s[3]:,.0f} ms、送信 {s[4]:,.0f} ms")

    target_posts = [p for gid in targets for p in posts_by_game.get(gid, [])]
    sends = sorted(ms(p["response"] - p["bestmove"]["time"])
                   for p in target_posts if "bestmove" in p and "response" in p)
    codes = {}
    for p in target_posts:
        codes[p.get("code")] = codes.get(p.get("code"), 0) + 1
    print()
    print(f"対象の対局のBotの着手: {len(target_posts)}手、送信の応答コード: {codes}")
    print(f"bestmoveから送信の応答まで（{len(sends)}手）: 中央値 {statistics.median(sends):,.0f} ms、"
          f"99パーセンタイル {statistics.quantiles(sends, n=100)[-1]:,.0f} ms、最大 {sends[-1]:,.0f} ms、"
          f"0.6秒超 {sum(v > 600 for v in sends)}手、2秒超 {sum(v > 2000 for v in sends)}手、"
          f"3秒超 {sum(v > 3000 for v in sends)}手")
    all_sends = [p for p in posts if "bestmove" in p and "response" in p]
    slow = [p for p in all_sends if ms(p["response"] - p["bestmove"]["time"]) > 3000]
    print(f"切替後の全対局の着手（{len(all_sends)}手）で送信が3秒を超えた手: {len(slow)}手")
    for p in slow:
        clock = games.get(p["game"], {}).get("full", {}).get("clock")
        kind = "通信対局" if clock is None else f"持ち時間{clock['initial'] // 60000}分・秒読み{clock['byoyomi'] // 1000}秒"
        print(f"  {p['bestmove']['time']:%Y-%m-%d %H:%M:%S}、対局 {p['game']}（{kind}）、"
              f"送信 {ms(p['response'] - p['bestmove']['time']):,.0f} ms")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
