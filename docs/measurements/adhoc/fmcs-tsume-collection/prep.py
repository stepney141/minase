"""fmcs付属の作品集3本を1問1ファイルへ分割し、規則名と盤面を1行にまとめる。"""
import csv, re, sys
from pathlib import Path

src, out = Path(sys.argv[1]), Path(sys.argv[2])
(out / "problems").mkdir(parents=True, exist_ok=True)
rows = []
for name in ["sns", "sm", "kh"]:
    lines = (src / f"{name}.fmi").read_bytes().decode("cp932").replace("\x85", "\n").splitlines()
    opts = [l.strip() for l in lines if l.startswith("//") and "L" not in l]  # //XK, //XZ だけ残す
    title, title_line, body, idx = None, None, None, 0
    for no, raw in enumerate(lines, 1):
        line = raw.strip()
        if line.startswith(";") or not line:
            continue
        if body is None and line.startswith('"'):
            title, title_line = line, no
            continue
        if line.startswith("最善詰"):
            body = []
        if body is None:
            continue
        body.append(line.split(";", 1)[0].strip())
        m = re.search(r"#(\d+)\s*$", body[-1])
        if m:
            idx += 1
            pid = f"{name}{idx:02d}"
            text = " ".join(b for b in body if b)
            fmi = "\r\n".join(opts + [f'"{pid}"', text, ""])
            (out / "problems" / f"{pid}.fmi").write_bytes(fmi.encode("cp932"))
            rows.append({"id": pid, "file": name, "title_line": title_line, "title": title,
                         "limit": int(m.group(1)), "body": text})
            body = None
with open(out / "problems.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0]))
    w.writeheader(); w.writerows(rows)
print(len(rows))
