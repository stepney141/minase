"""測定ディレクトリの runs/*/out.fmo を読み、1問1行の結果CSVを書く。"""
import csv, re, sys
from pathlib import Path

B = Path(sys.argv[1])
T = re.compile(r"解析時間：(?:(\d+)時間\s*)?(?:(\d+)分\s*)?(?:(\d+)秒)?\s*展開局面数：([\d,]+)\s*(?:検出解数：(\d+)|到達深度：(\d+))\s*(\S*)")

def secs(m):
    h, mi, s = (int(x) if x else 0 for x in m.group(1, 2, 3))
    return h * 3600 + mi * 60 + s

rows = []
for p in csv.DictReader(open(B / "problems.csv")):
    d = B / "runs" / p["id"]
    fmo = d / "out.fmo"
    text = fmo.read_bytes().decode("cp932", "replace") if fmo.exists() else ""
    times = [m for m in (T.search(l) for l in text.splitlines()) if m]
    # 中断後に出力される「最初に検出した詰手順」は早詰確認を経ていないので、確定した解に数えない。
    first = next((m for m in times if m.group(5) and int(m.group(5)) > 0 and m.start() >= 0
                  and "中断" not in text[:text.find(m.group(0))]), None)
    last = times[-1] if times else None
    lens = [int(x) for x in re.findall(r"まで (\d+)手", text)]
    status = "no_output"
    if last:
        tail = last.group(7)
        status = {"検討完了": "complete", "余詰打切": "complete_cook_limit"}.get(tail, "interrupted" if "中断" in text else tail or "unknown")
    # 画面ログはエスケープ列を除いてから cp437→cp932 で復元する。根の詰みを最初に見つけた時刻は
    # 「N手詰を検出しました」の直前の進捗行の時刻であり、out.fmo の最初の解の行（早詰確認の後）より早い。
    det = det_len = None
    scr = d / "screen.raw.log"
    if scr.exists():
        t = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07", "", scr.read_text("utf-8", "replace"))
        t = t.encode("cp437", "replace").decode("cp932", "replace")
        m = re.search(r"(\d+)手詰を検出しました", t)
        if m:
            ts = list(T.finditer(t[:m.start()]))
            det, det_len = (secs(ts[-1]) if ts else 0), int(m.group(1))
    st = (d / "status.txt").read_text() if (d / "status.txt").exists() else ""
    tm = (d / "time.txt").read_text() if (d / "time.txt").exists() else ""
    rss = re.search(r"Maximum resident set size \(kbytes\): (\d+)", tm)
    rows.append({
        "id": p["id"], "title": p["title"].strip('"'), "limit": p["limit"],
        "noted_unsolvable": "不詰" in p["title"],
        "status": status, "fmcs_unsolvable": "不詰" in text.split("以上")[0] and not lens,
        "solution_len": lens[0] if lens else "", "n_solutions": int(last.group(5)) if last and last.group(5) else 0,
        "confirmed_s": secs(first) if first else "", "first_mate_s": "" if det is None else det, "first_mate_len": "" if det_len is None else det_len, "confirmed_nodes": first.group(4).replace(",", "") if first else "",
        "final_s": secs(last) if last else "", "final_nodes": last.group(4).replace(",", "") if last else "",
        "wall_s": re.search(r"wall=([\d.]+)", st).group(1) if "wall=" in st else "",
        "exit": re.search(r"exit=(\d+)", st).group(1) if "exit=" in st else "",
        "max_rss_mb": round(int(rss.group(1)) / 1024) if rss else "",
    })
with open(B / "results.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0])); w.writeheader(); w.writerows(rows)
