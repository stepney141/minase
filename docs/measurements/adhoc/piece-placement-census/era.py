import json, datetime as dt
exec(open("census.py").read().split("humans()\nharness")[0])
JST = dt.timezone(dt.timedelta(hours=9))
T = lambda y,m,d,H=0,M=0: dt.datetime(y,m,d,H,M,tzinfo=JST).timestamp()*1000
c200,c24 = T(2026,10,4,17,36),T(2026,10,12,12,3)
n={}
for l in open("bot.ndjson"):
    g=json.loads(l)
    if not g.get("moves") or "initialSfen" in g: continue
    ps=g["players"]; me=0 if ps["sente"].get("user",{}).get("id")=="minase-bot" else 1
    e="9月〜10/4" if g["createdAt"]<c200 else "2.0.0〜2.3.0" if g["createdAt"]<c24 else "NNUE"
    n[e]=n.get(e,0)+1
    try: play(g["moves"].split(),{me:[e]})
    except Exception: pass
items=[("奔王1〜2段","q",lambda f,r:r<=2),("龍馬1〜2段","h",lambda f,r:r<=2),("醉象3〜5筋","e",lambda f,r:3<=f<=5),("金将3段以上","g",lambda f,r:r>=3),("麒麟右端2筋","o",lambda f,r:f<=2),("王将8筋以上","k",lambda f,r:f>=8)]
def sh(g,k,p):
    t=s=0
    for ph in (0,1):
        a=counts[g].get(ph,{}).get(k)
        if not a: continue
        for i,x in enumerate(a):
            t+=x; s+= x if p(i//12+1,i%12+1) else 0
    return s/t if t else float('nan')
print(n)
for nm,k,p in items: print(nm.ljust(8)," ".join(f"{e}:{sh(e,k,p):6.1%}" for e in n))
