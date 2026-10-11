import json, sys
D = sys.argv[1]
L = json.load(open(f"{D}/out/lishogi.json"))
M = json.load(open(f"{D}/out/matches.json"))

def pct(a, b):
    return f"{100.0*a/b:.2f}%" if b else "n/a"

out = []
w = out.append
w("# 王駒2枚（王将/玉将＋太子）の出現頻度と終局理由の集計\n")
w("対象は、lishogiの中将棋棋譜 23,567局（`data/lishogi/games.ndjson`、2026年9月22日取得。集計ツールはこのファイルを全件読み終えた後に出力を書いたので、第7節に記す上書き事故は集計値に影響しない）と、"
  "minaseの自己対局 1,816局（`data/matches/move-ordering-all-ltc` 599ペア＝1,198局、`data/matches/move-ordering-all-stc` 309ペア＝618局。いずれも候補ea0c75a対基準cd081f4、規則L0,P0,R1,E0、時間制御60000+200 msと10000+100 ms）である。"
  "自己対局は全自己対局からの無作為標本ではなく、直近2測定の全局である。\n")
w("再生はminase-core（`Game::play`）で行った。lishogi棋譜は`Rules::LISHOGI`（L1,L2,P0,P3,R1,E1,E3）で再生し、同じ着手列を`e1: false`（詰みで終局）にした双子対局にも与えて、盤上に詰みが成立した手数を記録した。自己対局はmanifestの`canonical_rules`から作った規則で再生し、保存された終局理由と再生の裁定が全1,790局で一致した（不一致0件）。王駒数は`Position::royal_pieces(color).popcount()`、太子の有無は`pieces_of_kind(color, PieceKind::CrownPrince)`で数えた。\n")

w("## 1. 対象局数と除外\n")
w("| 情報源 | 総局数 | 再生局数 | 除外の内訳 |")
w("|---|---:|---:|---|")
ex = ", ".join(f"{k} {v}" for k, v in L["excluded"].items())
w(f"| lishogi | {L['games_total']:,} | {L['games_replayed']:,} | {ex} |")
ex = ", ".join(f"{k} {v}" for k, v in M["excluded"].items())
w(f"| 自己対局 | {M['games_total']:,} | {M['games_replayed']:,} | {ex} |")
w("")
w("lishogiの除外は、途中局面から始まる`initialSfen`つき 459局（太子の出現を局単位で数えられない）、着手なし 247局、`noStart` 5局、`Rules::LISHOGI`で不合法手が現れた 1局（id iKnVosIP、39手目）である。自己対局の除外は手数上限4096に達した打ち切り 26局である。lishogiの`status`が`stalemate`の1局（id EOTgK5TS）は`initialSfen`つきで除外側に入ったため、再生した局に合法手なしによる終局はない。lishogiで終局後に着手が残っていた局は0、lishogiの勝者とminaseの裁定の勝者が食い違った局は1（`draw`と記録された局をR1が反復による勝ちと裁定）であった。\n")

w("## 2. 終局理由\n")
w("### 2a. lishogi（lishogiのstatus / minase再生の裁定）\n")
w("| status / 裁定 | 局数 | 割合 |")
w("|---|---:|---:|")
tot = L["games_replayed"]
for k, v in sorted(L["end_reasons"].items(), key=lambda kv: -kv[1]):
    w(f"| {k} | {v:,} | {pct(v, tot)} |")
w(f"| 合計 | {tot:,} | 100% |")
w("")
w("E1（王駒の実捕獲まで続行）のため、minaseの裁定で`Mate`は出ない。双子対局（詰みで終局）で見た、盤上の詰みと実際の終わり方の関係は次のとおりである。\n")
w("| status | 詰みなし | 最終手で詰み | 詰み後1〜2手続行 | 詰み後3〜10手続行 | 詰み後11手以上続行 |")
w("|---|---:|---:|---:|---:|---:|")
tm = L["twin_mate"]
for st in ["royalsLost", "resign", "outoftime", "timeout", "bareKing", "draw", "perpetualCheck", "repetition"]:
    nm = sum(v for k, v in tm.items() if k.startswith(st + " / twin:no-mate"))
    fin = tm.get(f"{st} / twin:mate-at-final-ply", 0)
    c12 = tm.get(f"{st} / twin:mate-then-continued-1-2-plies", 0)
    c310 = tm.get(f"{st} / twin:mate-then-continued-3-10-plies", 0)
    c10 = tm.get(f"{st} / twin:mate-then-continued->10-plies", 0)
    w(f"| {st} | {nm:,} | {fin:,} | {c12:,} | {c310:,} | {c10:,} |")
w("")
rl = {k: v for k, v in tm.items() if k.startswith("royalsLost")}
rl_tot = sum(rl.values()); rl_nomate = sum(v for k, v in rl.items() if "no-mate" in k)
rs_fin = tm.get("resign / twin:mate-at-final-ply", 0); rs_tot = sum(v for k, v in tm.items() if k.startswith("resign"))
w(f"王駒捕獲（royalsLost）{rl_tot:,}局のうち、捕獲の前に詰みが成立していなかった局は{rl_nomate:,}局（{pct(rl_nomate, rl_tot)}）である。これは主に王手を放置した結果の捕獲（第8条第4項）であり、獅子が1手で王将と太子を両取りした捕獲（第21条第5項）も含み得る。投了{rs_tot:,}局のうち、投了時点で盤上に詰みが成立していた局は{rs_fin:,}局（{pct(rs_fin, rs_tot)}）である。\n")

w("### 2b. 自己対局（審判層の終局理由）\n")
w("| 終局理由 | 局数 | 割合 |")
w("|---|---:|---:|")
tot = M["games_replayed"]
for k, v in sorted(M["end_reasons"].items(), key=lambda kv: -kv[1]):
    w(f"| {k} | {v:,} | {pct(v, tot)} |")
w(f"| 合計 | {tot:,} | 100% |")
w("")
w(f"`RoyalCapture` 6局は、いずれも開始手順（11手、11手、9手の無作為着手）の直後、エンジンの初手（10手目または12手目）で王将を取られた局である（3ペア×2局、各局のturnsは1手、`royal_capture_dumps`に最終手直前のSFENを保存した）。無作為開始手順が王将を獅子の利きに置いた人工的な事例であり、王駒2枚の事例ではない（捕獲直前の敗者の王駒数は全6局で1）。詰み以外の決着は反復による勝ち92局、反復引き分け56局、駒枯れ7局で、`Stalemate`（合法手なし）は0局である（E0のため`BareKing`は裁定対象外）。\n")

w("## 3. 終局付近の王駒数（敗者側, 勝者側）\n")
w("勝敗の決まった局について、終局局面（0）、終局の1プライ前、10プライ前、20プライ前での（敗者の王駒数, 勝者の王駒数）の度数を示す。`too_short`は、その手数に達する前に局が始まっていた局数である。終局局面で敗者が0枚なのは王駒捕獲で終わった局である。\n")
for name, A in [("lishogi", L), ("自己対局", M)]:
    w(f"### 3{'a' if name=='lishogi' else 'b'}. {name}\n")
    keys = ["loser0_winner1", "loser0_winner2", "loser1_winner1", "loser1_winner2", "loser2_winner1", "loser2_winner2", "too_short"]
    w("| 時点 | " + " | ".join(k.replace("loser", "敗者").replace("_winner", "/勝者").replace("too_short", "手数不足") for k in keys) + " | 敗者2枚の割合 | 勝者2枚の割合 | どちらかが2枚の割合 |")
    w("|---|" + "---:|" * (len(keys) + 3))
    for off in ["minus0", "minus1", "minus10", "minus20"]:
        c = A["royals_at_offset"][off]
        n = sum(v for k, v in c.items() if k != "too_short")
        l2 = c.get("loser2_winner1", 0) + c.get("loser2_winner2", 0)
        w2 = c.get("loser0_winner2", 0) + c.get("loser1_winner2", 0) + c.get("loser2_winner2", 0)
        any2 = l2 + w2 - c.get("loser2_winner2", 0)
        lab = {"minus0": "終局局面", "minus1": "1プライ前", "minus10": "10プライ前", "minus20": "20プライ前"}[off]
        w(f"| {lab} | " + " | ".join(f"{c.get(k,0):,}" for k in keys) + f" | {pct(l2, n)} | {pct(w2, n)} | {pct(any2, n)} |")
    w("")
    nw = A["royals_final_no_winner"]
    w("引き分けまたは勝者不明の局の終局局面（先手王駒, 後手王駒）: " + ", ".join(f"{k} {v}" for k, v in nw.items()) + f"（計{sum(nw.values())}局）。\n")

w("## 4. 全局面に占める王駒2枚の局面\n")
w("| 情報源 | 局面数（初期局面を含む） | 手番の相手が王駒2枚 | 割合 | 手番側が王駒2枚 | 割合 |")
w("|---|---:|---:|---:|---:|---:|")
for name, A in [("lishogi", L), ("自己対局", M)]:
    w(f"| {name} | {A['positions_total']:,} | {A['positions_opp_two_royals']:,} | {pct(A['positions_opp_two_royals'], A['positions_total'])} | {A['positions_stm_two_royals']:,} | {pct(A['positions_stm_two_royals'], A['positions_total'])} |")
w("")

w("## 5. 太子が現れた局\n")
w("| 情報源 | 再生局数 | 太子が一度でも現れた局 | 割合 |")
w("|---|---:|---:|---:|")
for name, A in [("lishogi", L), ("自己対局", M)]:
    w(f"| {name} | {A['games_replayed']:,} | {A['games_with_prince']:,} | {pct(A['games_with_prince'], A['games_replayed'])} |")
w("")
w("太子が現れた局の終局理由の内訳は次のとおりである。\n")
w("| 情報源 | 終局理由 | 太子が現れた局 | その理由の全局 | 割合 |")
w("|---|---|---:|---:|---:|")
for name, A in [("lishogi", L), ("自己対局", M)]:
    for k, v in sorted(A["games_with_prince_by_reason"].items(), key=lambda kv: -kv[1]):
        w(f"| {name} | {k} | {v:,} | {A['end_reasons'][k]:,} | {pct(v, A['end_reasons'][k])} |")
w("")

w("## 6. 詰み探索への含意\n")
l0 = L["royals_at_offset"]["minus0"]; m0 = M["royals_at_offset"]["minus0"]
l_n = sum(v for k, v in l0.items() if k != "too_short"); m_n = sum(v for k, v in m0.items() if k != "too_short")
l_l2 = l0.get("loser2_winner1", 0) + l0.get("loser2_winner2", 0)
l_m1 = L["royals_at_offset"]["minus1"]; l_m1n = sum(v for k, v in l_m1.items() if k != "too_short"); l_m1l2 = l_m1.get("loser2_winner1",0)+l_m1.get("loser2_winner2",0)
m_m1 = M["royals_at_offset"]["minus1"]; m_m1n = sum(v for k, v in m_m1.items() if k != "too_short"); m_m1l2 = m_m1.get("loser2_winner1",0)+m_m1.get("loser2_winner2",0)
w(f"- 敗者（詰まされる側）が王駒2枚のまま終局した局は、lishogiで{l_l2}局（勝敗の決まった{l_n:,}局の{pct(l_l2,l_n)}）、自己対局で0局である。終局の1プライ前でも、lishogi {l_m1l2}局（{pct(l_m1l2,l_m1n)}）、自己対局 {m_m1l2}局（{pct(m_m1l2,m_m1n)}）にとどまる。"
  "詰み探索が「守る側に王駒が2枚ある」局面を終局付近で扱う頻度は1%未満である。")
l_w2 = l0.get("loser0_winner2",0)+l0.get("loser1_winner2",0)+l0.get("loser2_winner2",0)
m_w2 = m0.get("loser0_winner2",0)+m0.get("loser1_winner2",0)+m0.get("loser2_winner2",0)
w(f"- 勝者（詰ます側）が王駒2枚で終局した局は、lishogi {l_w2}局（{pct(l_w2,l_n)}）、自己対局 {m_w2}局（{pct(m_w2,m_n)}）で、敗者側の2枚より1桁多い。詰みの判定そのものには影響しないが、攻め側の王駒の安全を見る処理では2枚を前提にする必要がある。")
w(f"- 全局面で見ると、手番の相手が王駒2枚である局面は lishogi {pct(L['positions_opp_two_royals'], L['positions_total'])}、自己対局 {pct(M['positions_opp_two_royals'], M['positions_total'])} である。太子が一度でも現れる局は lishogi {pct(L['games_with_prince'], L['games_replayed'])}、自己対局 {pct(M['games_with_prince'], M['games_replayed'])} で、自己対局の方が醉象を成らせる頻度が高い。")
w(f"- 自己対局（E0、詰みで終局）では決着{sum(v for k,v in M['end_reasons'].items() if k.startswith('Win')):,}局のうち詰みが{M['end_reasons']['Win:Mate']:,}局（{pct(M['end_reasons']['Win:Mate'], M['games_replayed'])}）を占める。lishogi（E1）では王駒捕獲{rl_tot:,}局のうち{rl_tot-rl_nomate:,}局（{pct(rl_tot-rl_nomate, rl_tot)}）は捕獲前に詰みが成立しており、残り{rl_nomate:,}局は詰みを経ない捕獲（主に王手放置）である。")
w("")
w("## 7. 実行したコマンド\n")
w("```console")
w("pgrep -af '[m]inase (match run|spsa)|[m]atch_runner|[s]psa_runner'   # 出力なし（測定は走っていない）")
w(f"cd {D} && CARGO_TARGET_DIR={D}/target nice -n 19 cargo build --release --offline -j 1")
w(f"cd /home/stepney141/board-games/minase && nice -n 19 {D}/target/release/royal-diag lishogi data/lishogi/games.ndjson {D}/out/lishogi.json   # 87.26 s（単一スレッド）。初版の引数順（入力, 出力）で実行した")
w(f"cd /home/stepney141/board-games/minase && nice -n 19 {D}/target/release/royal-diag matches {D}/out/matches.json data/matches/move-ordering-all-ltc data/matches/move-ordering-all-stc   # 16.25 s（単一スレッド）")
w(f"python3 {D}/format_report.py {D}")
w("```")
w("")
w("初版のツールには出力先を第2引数から取る誤りがあり、lishogiモードでは入力ファイル `data/lishogi/games.ndjson` に集計JSON（3.5 kB）を上書きした。集計結果はその書き込まれたJSONを `out/lishogi.json` へ退避して保存した。"
  "`data/strength-stage9/human/games-final.ndjson` が同一内容（31,796,374バイト、23,567局、`data/lishogi/state.json` の `game_ids` と一致）であるため、`cp -p data/strength-stage9/human/games-final.ndjson data/lishogi/games.ndjson` で復元できる。復元はこの集計の実行者（エージェント）には許可されなかったので、利用者が実行する。"
  "現在のソースは引数順を `lishogi <out.json> <ndjson>` に直し、出力先が `.json` でなければ停止するが、修正版での再実行はしていない（集計値は初版の実行による）。")
w("")
w(f"ソースは `{D}/src/main.rs`、集計の生データは `{D}/out/lishogi.json` と `{D}/out/matches.json` にある。")
open(f"{D}/out/report.md", "w").write("\n".join(out) + "\n")
print("\n".join(out))
