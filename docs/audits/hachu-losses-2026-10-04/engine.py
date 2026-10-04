import subprocess, re
import os
BIN=os.environ.get("MINASE_BIN","/home/stepney141/board-games/minase/target/match-cache/9027662d8d1761be02337eece76421f27f05cc38/minase")
RULES="L1,L3,P0,P5,P6,R2,E1,E2"
class Engine:
    def __init__(self, threads=2, hash_mb=1024):
        self.p=subprocess.Popen([BIN,"--protocol","usi","--rules",RULES],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True,bufsize=1)
        self.send("usi"); self.until("usiok")
        self.send(f"setoption name Threads value {threads}"); self.send(f"setoption name USI_Hash value {hash_mb}")
        self.send("setoption name ResignValue value 99999")
        self.send("isready"); self.until("readyok")
    def send(self,s): self.p.stdin.write(s+"\n"); self.p.stdin.flush()
    def until(self,tok):
        out=[]
        while True:
            l=self.p.stdout.readline()
            if not l: raise RuntimeError("engine died")
            out.append(l.rstrip("\n"))
            if l.startswith(tok): return out
    def pos(self,moves): self.send("position startpos"+(" moves "+" ".join(moves) if moves else ""))
    def d(self,moves):
        self.pos(moves); self.send("d"); lines=self.until("rights-zobrist")
        sfen=next(l for l in lines if l.startswith("sfen ")); z=next(l.split()[1] for l in lines if l.startswith("zobrist "))
        board=[l for l in lines if l.startswith("|")]
        return sfen[5:], z, "\n".join(board)
    def search(self,moves,movetime):
        self.pos(moves); self.send(f"go movetime {movetime}"); lines=self.until("bestmove")
        info=[l for l in lines if l.startswith("info depth") and " pv " in l]
        last=info[-1] if info else ""
        m=re.search(r"score (cp|mate) (-?\d+)",last); dd=re.search(r"depth (\d+)",last); pv=re.search(r" pv (.*)$",last)
        return {"best":lines[-1].split()[1],"score":(m.group(1),int(m.group(2))) if m else None,"depth":int(dd.group(1)) if dd else None,"pv":pv.group(1).split() if pv else []}
    def evaluate(self,moves):
        self.pos(moves); self.send("eval"); lines=self.until("info string end")
        return int(next(l.split()[-1] for l in lines if l.startswith("info string evaluation")))
    def quit(self): self.send("quit"); self.p.wait(timeout=10)
