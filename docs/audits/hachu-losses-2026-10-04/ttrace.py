import re
from games import lost_games
from engine import Engine
G={g["pair"]:g for g in lost_games()}
cases=[(29,287),(29,288),(70,271)]
for pair,n in cases:
    e=Engine(); mv=G[pair]["moves"][:n]
    r=e.search(mv,30000)
    hist=[e.d(mv[:i])[1] for i in range(len(mv)+1)]
    line=[]; path=[]; hit=None
    for step in range(40):
        e.pos(mv+line); e.send("tt"); out=e.until("info string end")
        txt=" ".join(out); m=re.search(r"move (\S+)",txt)
        if step==0: print("   tt sample:", out[:4])
        if not m or m.group(1) in ("none","-"): break
        line.append(m.group(1)); k=e.d(mv+line)[1]
        if k in hist: hit=(len(line),"history"); break
        if k in path: hit=(len(line),"path"); break
        path.append(k)
    print(pair,n,r["score"],r["depth"],"pv",r["pv"],"| tt line",len(line),"repetition",hit," ".join(line[:16]))
    e.quit()
