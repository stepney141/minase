import json, math, collections, datetime as dt
JST = dt.timezone(dt.timedelta(hours=9))
T = lambda y,m,d,H=0,M=0: dt.datetime(y,m,d,H,M,tzinfo=JST).timestamp()*1000
c200,c24 = T(2026,10,4,17,36),T(2026,10,12,12,3)
gs = [json.loads(l) for l in open("bot.ndjson")]
gs = sorted((g for g in gs if g.get("moves") and "initialSfen" not in g and len(g["moves"].split())>=30), key=lambda g:g["createdAt"])
rows=[]
for g in gs:
    ps=g["players"]; me="sente" if ps["sente"].get("user",{}).get("id")=="minase-bot" else "gote"; o="gote" if me=="sente" else "sente"
    if "user" not in ps[o] or ps[o]["user"].get("title")=="BOT": continue
    if not g.get("winner"): continue
    c=g.get("clock") or {}
    rows.append(dict(o=ps[o]["user"]["id"],w=g["winner"]==me,t=g["createdAt"],clock=(c.get("initial"),c.get("increment"),c.get("byoyomi")),days=g.get("daysPerTurn"),r=ps[o].get("rating",0)))
def mh(sel0,sel1,exclude=()):
    S=collections.defaultdict(lambda:[0,0,0,0])
    for r in rows:
        if r["o"] in exclude: continue
        if sel0(r): S[r["o"]][0 if r["w"] else 1]+=1
        elif sel1(r): S[r["o"]][2 if r["w"] else 3]+=1
    R=Q=0; PR=PSQ=QS=0; used=0;n0=n1=0
    for a,b,c,d in S.values():  # a,b=old win/loss; c,d=new win/loss ; OR=(c*b)/(d*a)
        n=a+b+c+d
        if a+b==0 or c+d==0: continue
        used+=1;n0+=a+b;n1+=c+d
        r=c*b/n; s=d*a/n
        R+=r;Q+=s
        P=(c+b)/n; QQ=(d+a)/n
        PR+=P*r; PSQ+=P*s+QQ*r; QS+=QQ*s
    if R==0 or Q==0: return used,n0,n1,None
    l=math.log(R/Q); se=math.sqrt(PR/(2*R*R)+PSQ/(2*R*Q)+QS/(2*Q*Q)); k=400/math.log(10)
    return used,n0,n1,round(l*k),round((l-1.96*se)*k),round((l+1.96*se)*k)
pst=lambda r:r["t"]<c24; nn=lambda r:r["t"]>=c24; sep=lambda r:r["t"]<c200; mid=lambda r:c200<=r["t"]<c24
print("相手ごとに層別したMantel-Haenszel（相手数, 旧局数, 新局数, Elo差, 95%下, 95%上）")
print("PST+FM全体→NNUE全体", mh(pst,nn))
print("9月〜10/4版→2.0.0〜2.3.0", mh(sep,mid))
print("2.0.0〜2.3.0→NNUE", mh(mid,nn))
print("9月〜10/4版→NNUE", mh(sep,nn))
top=collections.Counter(r["o"] for r in rows).most_common(2)
names=list(dict.fromkeys(r["o"] for r in rows))
print("最多の相手を除く PST+FM→NNUE", mh(pst,nn,exclude=(top[0][0],)))
strong=[o for o,c in collections.Counter(r["o"] for r in rows).items() if max(x["r"] for x in rows if x["o"]==o)>=2400]
print("R2400以上の相手に限定した局数", sum(1 for r in rows if r["o"] in strong), "相手数", len(strong))
print("PST+FM→NNUE R2400以上のみ", mh(lambda r:pst(r) and r["o"] in strong, lambda r:nn(r) and r["o"] in strong))
print("PST+FM→NNUE R2400未満のみ", mh(lambda r:pst(r) and r["o"] not in strong, lambda r:nn(r) and r["o"] not in strong))
for o in strong:
    x=[r for r in rows if r["o"]==o]
    a=[r for r in x if pst(r)];b=[r for r in x if nn(r)]
    print(" 強豪",names.index(o),"R",x[0]["r"],"→",x[-1]["r"],f"PST+FM {sum(r['w'] for r in a)}/{len(a)}  NNUE {sum(r['w'] for r in b)}/{len(b)}",collections.Counter(r["clock"] for r in a).most_common(2),collections.Counter(r["clock"] for r in b).most_common(2))
for lbl,sel in (("9月〜10/4版",sep),("2.0.0〜2.3.0",mid),("NNUE",nn)):
    x=[r for r in rows if sel(r)]
    print(lbl,"局数",len(x),"R2400以上の相手の局数",sum(r["o"] in strong for r in x))
