import json
from games import lost_games
from engine import Engine
G={g["pair"]:g for g in lost_games()}
e=Engine(threads=1,hash_mb=16)
def keys(moves):
    return [e.d(moves[:i])[1] for i in range(len(moves)+1)]
cases=[]
for r in json.load(open("deep30.json")):
    for name,off in (("p0",0),("p1",1),("p2",2)):
        s=r[name]["score"]
        if s and s[0]=="cp" and s[1]==0: cases.append((r["pair"],r["ply"]+off,r[name]["pv"],name))
for pair,n,pv,name in cases:
    mv=G[pair]["moves"]; hist=keys(mv[:n]); 
    path=[]; hit=None
    for j in range(len(pv)):
        k=e.d(mv[:n]+pv[:j+1])[1]
        if k in hist: hit=(j+1,"history",len(hist)-1-hist.index(k)); break
        if k in path: hit=(j+1,"path"); break
        path.append(k)
    print(pair,n,name,"pvlen",len(pv),"repetition at",hit, " ".join(pv[:12]))
e.quit()
