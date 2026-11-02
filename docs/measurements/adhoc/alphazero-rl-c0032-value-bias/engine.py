"""USIでminaseのMCTSを起動して、局面ごとの評価を読む。"""

import re
import subprocess


class Engine:
    def __init__(self, binary, net, batch):
        self.process = subprocess.Popen(
            [binary, "--protocol", "usi", "--rules", "engine-default", "--search", "mcts", "--nn-model", net],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
        )
        self.send("usi")
        self.until("usiok")
        self.send(f"setoption name NnBatchSize value {batch}")
        # ハーネスと同じく投了を無効にして、評価値が低くても手を返させる。
        self.send("setoption name ResignValue value 99999")
        self.send("isready")
        self.until("readyok")

    def send(self, line):
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def until(self, token):
        lines = []
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError("the engine stopped: " + "".join(lines[-5:]))
            lines.append(line)
            if line.startswith(token):
                return lines

    def search(self, moves, go):
        """木を持ち越さずに探索し、最後のinfoの(cp, nodes, time)を返す。cpのないinfoならcpはNoneである。"""
        self.send("usinewgame")
        self.send("position startpos moves " + " ".join(moves))
        self.send(go)
        info = [line for line in self.until("bestmove") if " nodes " in line][-1]
        cp = re.search(r"score cp ([+-]?\d+)", info)
        nodes = re.search(r"nodes (\d+)", info)
        elapsed = re.search(r"time (\d+)", info)
        return (int(cp[1]) if cp else None), int(nodes[1]), int(elapsed[1])

    def quit(self):
        self.send("quit")
        self.process.wait()
