import json, math, datetime as dt, collections, sys
JST = dt.timezone(dt.timedelta(hours=9))
cuts = [("master(〜10-04) PST+FM", "2026-09-01 00:00"), ("2.0.0 PST+FM", "2026-10-04 17:36"), ("2.2.0 PST+FM", "2026-10-08 09:43"),
        ("2.3.0 PST+FM", "2026-10-09 00:53"), ("2.4.0 NNUE", "2026-10-12 12:03"), ("2.5.0 NNUE", "2026-10-23 20:50"), ("end", "2100-01-01 00:00")]
cuts = [(n, dt.datetime.strptime(t, "%Y-%m-%d %H:%M").replace(tzinfo=JST).timestamp() * 1000) for n, t in cuts]
gs = [json.loads(l) for l in open("bot.ndjson")]
gs = [g for g in gs if g.get("moves") and "initialSfen" not in g]
print(len(gs), "games from", dt.datetime.fromtimestamp(min(g["createdAt"] for g in gs)/1000, JST), "to", dt.datetime.fromtimestamp(max(g["createdAt"] for g in gs)/1000, JST))
def row(name, sel):
    w = l = d = 0; opp = []; me = []; regs = collections.Counter()
    for g in sel:
        ps = g["players"]; side = "sente" if ps["sente"].get("user", {}).get("id") == "minase-bot" else "gote"; o = "gote" if side == "sente" else "sente"
        if "user" not in ps[o] or ps[o].get("user", {}).get("title") == "BOT": continue
        if len(g["moves"].split()) < 30: continue
        r = g.get("winner")
        if r == side: w += 1
        elif r == o: l += 1
        else: d += 1
        opp.append(ps[o].get("rating", 0)); me.append(ps[side].get("rating", 0))
    n = w + l + d
    if not n: print(f"{name:26} 0"); return
    s = (w + d / 2) / n; a = sum(opp) / n
    sc = min(max(s, 0.5 / n), 1 - 0.5 / n)
    print(f"{name:26} {n:4} {w:3}勝{l:3}敗{d:2}分 勝率{s:6.1%} 相手平均R {a:6.0f} 当時R平均 {sum(me)/n:6.0f} perf {a + 400 * math.log10(sc / (1 - sc)):6.0f}")
for (name, lo), (_, hi) in zip(cuts, cuts[1:]):
    row(name, [g for g in gs if lo <= g["createdAt"] < hi])
row("PST+FM全体", [g for g in gs if cuts[0][1] <= g["createdAt"] < cuts[4][1]])
row("NNUE全体", [g for g in gs if cuts[4][1] <= g["createdAt"]])
print('---- 2.6.0 を分離')
cut26 = dt.datetime(2026,10,25,18,26,tzinfo=JST).timestamp()*1000
row("2.5.0 NNUE", [g for g in gs if cuts[5][1] <= g["createdAt"] < cut26])
row("2.6.0 NNUE", [g for g in gs if cut26 <= g["createdAt"]])
def perf(sel):
    w=l=d=0;opp=[]
    for g in sel:
        ps=g["players"];side="sente" if ps["sente"].get("user",{}).get("id")=="minase-bot" else "gote";o="gote" if side=="sente" else "sente"
        if "user" not in ps[o] or ps[o].get("user",{}).get("title")=="BOT" or len(g["moves"].split())<30: continue
        r=g.get("winner");w+=r==side;l+=r==o;d+=r not in(side,o);opp.append(ps[o].get("rating",0))
    n=w+l+d;s=(w+d/2)/n;e=400*math.log10(s/(1-s));se=math.sqrt(s*(1-s)/n)*400/math.log(10)/(s*(1-s))
    return n,s,sum(opp)/n,e,se
a=perf([g for g in gs if cuts[0][1]<=g["createdAt"]<cuts[4][1]]);b=perf([g for g in gs if cuts[4][1]<=g["createdAt"]])
print(a,b);pa=a[2]+a[3];pb=b[2]+b[3];print("perf差",pb-pa,"±",1.96*math.hypot(a[4],b[4]))
print("自己対局 NNUE初代対PST+FM: STC", 400*math.log10(.885/.115), "LTC", 400*math.log10(.918/.082))
tc=collections.Counter()
for g in gs:
    c=g.get("clock") or {}
    tc[(g["createdAt"]>=cuts[4][1], g.get("speed") or g.get("perf"), g.get("rated"))]+=1
print(tc.most_common(12))
