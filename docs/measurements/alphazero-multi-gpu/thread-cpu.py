# プロセスのスレッドごとのCPUの使用率を指定の秒数で測ってスレッド名ごとにまとめ、待ちの場所（wchan）を数える。
# 使い方: python3 thread-cpu.py <PID> <秒数>
import collections, os, sys, time
pid = sys.argv[1]; interval = float(sys.argv[2])
def snap():
    out = {}
    for tid in os.listdir(f"/proc/{pid}/task"):
        try:
            stat = open(f"/proc/{pid}/task/{tid}/stat").read()
            comm = open(f"/proc/{pid}/task/{tid}/comm").read().strip()
            fields = stat.rsplit(")", 1)[1].split()
            out[tid] = (comm, int(fields[11]) + int(fields[12]), fields[0])
        except FileNotFoundError:
            pass
    return out
a = snap(); time.sleep(interval); b = snap()
hz = os.sysconf("SC_CLK_TCK")
groups = collections.defaultdict(list)
for tid, (comm, t, state) in b.items():
    if tid in a:
        groups[comm].append((t - a[tid][1]) / hz / interval)
for comm, shares in sorted(groups.items(), key=lambda x: -sum(x[1])):
    shares.sort(reverse=True)
    print(f"{comm:18} threads={len(shares):3} total={sum(shares):6.2f} cores  top={[round(s, 2) for s in shares[:6]]}")
wchan = collections.Counter()
for tid in os.listdir(f"/proc/{pid}/task"):
    try:
        wchan[(open(f"/proc/{pid}/task/{tid}/comm").read().strip(), open(f"/proc/{pid}/task/{tid}/wchan").read().strip())] += 1
    except (FileNotFoundError, PermissionError):
        pass
for (comm, w), n in wchan.most_common(12):
    print("wchan", comm, w, n)
