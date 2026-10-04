import re, json
from games import lost_games
from engine import Engine
def parse(sfen):
    rows=sfen.split()[0].split("/"); cells={}
    for r,row in enumerate(rows):
        f=12; toks=re.findall(r"\+?[A-Za-z]|\d+",row)
        for t in toks:
            if t.isdigit(): f-=int(t)
            else: cells[(f,r)]=t; f-=1
    return cells

res={}
for g in lost_games():
    e=Engine(threads=1,hash_mb=16); me=g["color"]; mv=g["moves"]; prev=parse(e.d([])[0]); log=[]
    for i in range(len(mv)):
        cur=parse(e.d(mv[:i+1])[0])
        mover="black" if i%2==0 else "white"
        own=lambda t:"black" if t.lstrip("+")[0].isupper() else "white"
        caps=[t for sq,t in prev.items() if own(t)!=mover and (sq not in cur or own(cur[sq])==mover)]
        if caps: log.append((i, "M" if mover==me else "H", mv[i], caps))
        prev=cur
    e.quit(); res[g["pair"]]=log
    print(g["pair"], me)
    for x in log: print("  ",x)

json.dump(res,open("caps.json","w"))
