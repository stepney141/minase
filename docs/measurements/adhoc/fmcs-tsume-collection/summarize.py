"""results.csv を作者の判定と重複図で分類し、要約を出す。"""
import csv, re, statistics as st, sys
from pathlib import Path

B = Path(sys.argv[1])
# 作者が不詰とした作品。題名、直後のコメント行、または別作品集の同一出典の題名の注記による。
NOTED_UNSOLVABLE = {
    "sns05": "題名", "sns11": "題名", "sns26": "題名", "sns20": "直後のコメント（22玉で不詰）",
    "sm06": "題名（64獅で不詰）", "sm08": "題名の続き（14玉で不詰）",
    "sm21": "直後のコメント", "sm23": "直後のコメント", "sm26": "直後のコメント", "sm27": "直後のコメント",
    "sm42": "直後のコメント", "sm43": "直後のコメント", "sm50": "直後のコメント",
    "sm45": "絹篩23番（作物45）の題名", "kh42": "直後のコメント（a5横等で不詰）",
    "kh10": "題名", "kh24": "題名", "kh46": "題名", "kh49": "題名", "kh52": "題名", "kh61": "題名", "kh67": "題名",
}
probs = {p["id"]: p for p in csv.DictReader(open(B / "problems.csv"))}
rows = list(csv.DictReader(open(B / "results.csv")))
seen, first_of = {}, {}
for x in rows:
    key = (" ".join(sorted(re.findall(r"\S+", probs[x["id"]]["body"]))))
    first_of[x["id"]] = seen.setdefault(key, x["id"])
uniq = [x for x in rows if first_of[x["id"]] == x["id"]]
print("全問", len(rows), "重複除去後", len(uniq))
print("重複:", {k: v for k, v in first_of.items() if k != v})

def summarize(label, xs):
    sol = [x for x in xs if x["id"] not in NOTED_UNSOLVABLE]
    uns = [x for x in xs if x["id"] in NOTED_UNSOLVABLE]
    print(f"\n== {label}: 作者が詰とする {len(sol)} 問, 作者が不詰とする {len(uns)} 問")
    found = [x for x in sol if x["first_mate_s"] != ""]
    conf = [x for x in sol if x["confirmed_s"] != ""]
    done = [x for x in sol if x["status"].startswith("complete")]
    print("根の詰みを発見", len(found), "最短手数を確定", len(conf), "検討完了", len(done))
    print("手数一致", sum(x["solution_len"] == x["limit"] for x in conf), "/", len(conf))
    print("未発見", [(x["id"], x["limit"]) for x in sol if x["first_mate_s"] == ""])
    print("未確定", [(x["id"], x["limit"], x["first_mate_s"]) for x in sol if x["first_mate_s"] != "" and x["confirmed_s"] == ""])
    print("未完了", [(x["id"], x["limit"], x["confirmed_s"]) for x in sol if x["confirmed_s"] != "" and not x["status"].startswith("complete")])
    for name, col in (("発見", "first_mate_s"), ("確定", "confirmed_s"), ("完了", "final_s")):
        v = sorted(int(x[col]) for x in (found if name == "発見" else conf if name == "確定" else done))
        q = lambda f: v[min(len(v) - 1, int(f * len(v)))]
        print(f"{name}秒: 中央値 {st.median(v)}, 90% {q(0.9)}, 最大 {v[-1]}, 1秒以下 {sum(t<=1 for t in v)}, 10秒以下 {sum(t<=10 for t in v)}, 60秒以下 {sum(t<=60 for t in v)}")
    print("不詰判定", sum(x["fmcs_unsolvable"] == "True" for x in uns), "/", len(uns),
          "所要秒最大", max(int(x["final_s"]) for x in uns))
    print("作者が詰とするのに不詰と出た", [x["id"] for x in sol if x["fmcs_unsolvable"] == "True"])
    # 手数帯ごと
    for lo, hi in ((1, 15), (17, 25), (27, 39), (41, 99)):
        b = [x for x in sol if lo <= int(x["limit"]) <= hi]
        if not b: continue
        fm = sorted(int(x["first_mate_s"]) for x in b if x["first_mate_s"] != "")
        cf = sorted(int(x["confirmed_s"]) for x in b if x["confirmed_s"] != "")
        print(f"  {lo}-{hi}手: {len(b)}問, 発見 {len(fm)} (中央値 {st.median(fm) if fm else '-'}秒, 最大 {fm[-1] if fm else '-'}), 確定 {len(cf)} (中央値 {st.median(cf) if cf else '-'}秒, 最大 {cf[-1] if cf else '-'})")
    nps = [int(x["final_nodes"]) / int(x["final_s"]) for x in xs if x["final_s"] and int(x["final_s"]) >= 10]
    print("毎秒局面数（10秒以上の問題）中央値", round(st.median(nps)), "範囲", round(min(nps)), round(max(nps)))
    rss = [int(x["max_rss_mb"]) for x in xs if x["max_rss_mb"]]
    print("最大常駐メモリMB 中央値", st.median(rss), "最大", max(rss))

summarize("全150問", rows)
summarize("重複除去後", uniq)
