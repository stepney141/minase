import json, math, collections, datetime as dt
JST = dt.timezone(dt.timedelta(hours=9))
c24 = dt.datetime(2026,10,12,12,3,tzinfo=JST).timestamp()*1000
gs = [json.loads(l) for l in open("bot.ndjson")]
gs = sorted((g for g in gs if g.get("moves") and "initialSfen" not in g and len(g["moves"].split())>=30), key=lambda g:g["createdAt"])
by = collections.defaultdict(list)
for g in gs:
    ps=g["players"]; me="sente" if ps["sente"].get("user",{}).get("id")=="minase-bot" else "gote"; o="gote" if me=="sente" else "sente"
    if "user" not in ps[o] or ps[o]["user"].get("title")=="BOT": continue
    s=1.0 if g.get("winner")==me else 0.0 if g.get("winner")==o else 0.5
    exp=1/(1+10**((ps[o].get("rating",1500)-ps[me].get("rating",1500))/400))
    by[ps[o]["user"]["id"]].append((g["createdAt"],s,ps[o].get("rating",0),exp,g["createdAt"]>=c24))
print("対局相手数",len(by),"局数",sum(map(len,by.values())))
bins=[(1,3),(4,9),(10,24),(25,999)]
print("その相手との何局目か  局数  Botの得点率  相手平均R  Bot当時Rからの期待得点")
for lo,hi in bins:
    x=[t for v in by.values() for i,t in enumerate(v,1) if lo<=i<=hi]
    if x: print(f"{lo:3}〜{hi:3}局目 {len(x):5} {sum(t[1] for t in x)/len(x):8.1%} {sum(t[2] for t in x)/len(x):9.0f} {sum(t[3] for t in x)/len(x):9.1%}")
print("初対面の局のみ（版別）")
for era,f in (("PST+FM",False),("NNUE",True)):
    x=[v[0] for v in by.values() if v[0][4]==f]
    print(era,len(x),f"{sum(t[1] for t in x)/max(len(x),1):.1%}",f"相手R {sum(t[2] for t in x)/max(len(x),1):.0f}")
print("常連（10局以上）")
for k,v in sorted(by.items(),key=lambda kv:-len(kv[1]))[:10]:
    if len(v)<10: break
    p=[t for t in v if not t[4]]; n=[t for t in v if t[4]]
    f=lambda a:(f"{sum(t[1] for t in a):.0f}/{len(a)}" if a else "-")
    r0=v[0][2]; r1=v[-1][2]
    print(f"相手{list(by).index(k):3d} 計{len(v):3} PST+FM {f(p):>7} NNUE {f(n):>7} レーティング {r0}→{r1}")
