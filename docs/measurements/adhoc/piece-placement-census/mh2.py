import json, math, collections, datetime as dt
src=open("mh.py").read().split('pst=lambda')[0]
exec(src)
st=collections.defaultdict(collections.Counter)
rawg={}
for g in gs:
    ps=g["players"]; me="sente" if ps["sente"].get("user",{}).get("id")=="minase-bot" else "gote"; o="gote" if me=="sente" else "sente"
    if "user" not in ps[o] or ps[o]["user"].get("title")=="BOT" or not g.get("winner"): continue
    era="sep" if g["createdAt"]<c200 else "mid" if g["createdAt"]<c24 else "nnue"
    st[era][("勝" if g["winner"]==me else "負",g.get("status"))]+=1
    rawg[g["createdAt"]]=g.get("status")
for e,c in st.items(): print(e,sorted(c.items(),key=lambda kv:-kv[1]))
for r in rows: r["st"]=rawg[r["t"]]
keep=[r for r in rows if not (not r["w"] and r["st"] in ("outoftime","timeout"))]
rows[:]=keep
pst=lambda r:r["t"]<c24; nn=lambda r:r["t"]>=c24; sep=lambda r:r["t"]<c200; mid=lambda r:c200<=r["t"]<c24
print("Bot側の時間切れ負けを除く:")
print(" 9月〜10/4→2.0.0〜2.3.0",mh(sep,mid))
print(" 2.0.0〜2.3.0→NNUE",mh(mid,nn))
print(" 9月〜10/4→NNUE",mh(sep,nn))
