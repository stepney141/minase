import glob, json, os, re, statistics as st
SQ=re.compile(r"(\d{1,2})([a-l])")
exec(open("census.py").read().split("counts = {}")[0])
def first_capture(moves):
    b=parse(START)
    for n,mv in enumerate(moves,1):
        sq=[(int(a),RANKS.index(c)) for a,c in SQ.findall(mv)]
        frm,to=sq[0],sq[-1]
        pc=b.pop(frm)
        cap=any(m in b for m in sq[1:-1] if m!=frm) or to in b
        if cap: return n
        b[to]=pc
    return None
for run in ("nnue-material-linear-init-noninferiority-ltc","alphazero-rl-c0032-ab-ladder-m3200-a1000"):
    L=[];op=[]
    for f in sorted(glob.glob(os.path.join("../../../../data/matches",run,"pairs","*.json")))[:600]:
        p=json.load(open(f)); o=p["opening"]["moves"]; op.append(len(o))
        for g in p["games"]:
            mv=o+[t["response"]["usi"] for t in g["turns"] if t["response"].get("kind")=="move"]
            x=first_capture(mv)
            if x: L.append(x)
    L.sort(); print(run,"局",len(L),"最初の捕獲の手数 中央値",st.median(L),"10%",L[len(L)//10],"90%",L[len(L)*9//10],"序盤の手数平均",sum(op)/len(op))
