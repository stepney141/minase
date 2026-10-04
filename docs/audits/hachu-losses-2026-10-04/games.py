import json
RUN="/home/stepney141/board-games/minase/data/matches/search-revival-hachu-elo200/pairs"
def lost_games():
    for n in (29,70,75,142):
        p=json.load(open(f"{RUN}/{n:020d}.json")); op=list(p["opening"]["moves"])
        for gi,g in enumerate(p["games"]):
            t=g["termination"]; c=g["candidate_color"]
            if t["kind"]!="adjudicated_win" or t["winner"]==c: continue
            moves=op+[tu["response"]["usi"] for tu in g["turns"] if tu["response"]["kind"]=="move"]
            evs=[]
            for i,tu in enumerate(g["turns"]):
                ev=tu.get("evaluation")
                if tu["side"]==c and ev:
                    s=ev["score"]
                    if s["kind"]=="cp": v=s["value"]
                    elif s["kind"]=="mate_in": v=30000-s["moves"]
                    else: v=-30000+s["moves"]
                    if ev["perspective"]!=c: v=-v
                    evs.append(dict(ply=len(op)+i,v=v,depth=ev["depth"],move=tu["response"].get("usi"),t=tu["think_time_ns"]/1e9))
            yield dict(pair=n,game=gi,color=c,moves=moves,evs=evs)
def drops(g, thr=400):
    evs=g["evs"]; peak=max(range(len(evs)),key=lambda i:evs[i]["v"] if abs(evs[i]["v"])<20000 else -99999)
    out=[]
    for a,b in zip(evs[peak:],evs[peak+1:]):
        if b["ply"]==a["ply"]+2 and a["v"]-b["v"]>=thr and a["v"]>-1500: out.append((a,b))
    return evs[peak],out
if __name__=="__main__":
    for g in lost_games():
        pk,ds=drops(g)
        print(g["pair"],g["color"],"peak",pk["ply"],pk["v"],"drops",len(ds),[(a["ply"],a["v"],b["v"]) for a,b in ds])
