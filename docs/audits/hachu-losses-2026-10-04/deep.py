import json, sys
from concurrent.futures import ThreadPoolExecutor
from games import lost_games, drops
from engine import Engine
MT=int(sys.argv[1])
jobs=[]
for g in lost_games():
    _,ds=drops(g)
    for a,b in ds: jobs.append((g,a,b))
def run(j):
    g,a,b=j; k=a["ply"]; mv=g["moves"]; e=Engine()
    r=dict(pair=g["pair"],color=g["color"],ply=k,eval_before=a["v"],depth_before=a["depth"],eval_after=b["v"],depth_after=b["depth"],minase_move=mv[k],hachu_reply=mv[k+1])
    for name,n in (("p0",k),("p1",k+1),("p2",k+2)): r[name]=e.search(mv[:n],MT)
    r["static_p0"]=e.evaluate(mv[:k]); r["static_p2"]=e.evaluate(mv[:k+2])
    e.quit(); return r
with ThreadPoolExecutor(max_workers=8) as ex: out=list(ex.map(run,jobs))
json.dump(out,open(sys.argv[2],"w"),ensure_ascii=False,indent=1)
