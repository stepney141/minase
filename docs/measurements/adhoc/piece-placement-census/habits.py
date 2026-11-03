import json, re, datetime as dt
exec(open("census.py").read().split("humans()\nharness")[0])
JST = dt.timezone(dt.timedelta(hours=9))
c24 = dt.datetime(2026,10,12,12,3,tzinfo=JST).timestamp()*1000
gs = [json.loads(l) for l in open("bot.ndjson")]
for g in gs:
    if not g.get("moves") or "initialSfen" in g: continue
    ps = g["players"]; me = 0 if ps["sente"].get("user",{}).get("id")=="minase-bot" else 1
    name = "bot_nnue" if g["createdAt"]>=c24 else "bot_pstfm"
    try: play(g["moves"].split(), {me:[name], 1-me:[name+"_opp"]})
    except Exception: pass
d = json.load(open("../../../../data/piece-placement-census/counts.json"))["counts"]
for k,v in counts.items(): d[k] = v
def share(g,k,pred):
    t=s=0
    for ph in ("0","1"):
        a=d.get(g,{}).get(ph,{}).get(k)
        if a is None:
            a=d.get(g,{}).get(int(ph),{}).get(k) if isinstance(d.get(g),dict) else None
        if not a: continue
        for i,x in enumerate(a):
            t+=x
            if pred(i//12+1,i%12+1): s+=x
    return s/t if t else float('nan')
items=[("奔王 自陣1〜2段","q",lambda f,r:r<=2),("龍馬 自陣1〜2段","h",lambda f,r:r<=2),("王将 初期升の外(8筋以上)","k",lambda f,r:f>=8),("醉象 3筋から5筋","e",lambda f,r:3<=f<=5),("金将 3段以上","g",lambda f,r:r>=3),("麒麟 右端2筋","o",lambda f,r:f<=2),("横行 両端の3段","m",lambda f,r:r==3 and f in(1,12)),("鳳凰 8筋4段","x",lambda f,r:f==8 and r==4),("歩兵 平均","p",lambda f,r:r>=6)]
cols=["human2000","humanlow","hachu","c0032","pst_gen0","pst_gen1","pst_fm","nnue_first","nnue_now","bot_pstfm","bot_nnue","bot_nnue_opp"]
print("項目".ljust(20)+" ".join(f"{c[:10]:>10}" for c in cols))
for n,k,p in items:
    print(n.ljust(18)+" ".join(f"{share(c,k,p):10.1%}" for c in cols))
