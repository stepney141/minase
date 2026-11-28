"""AlphaZero型の学習器で、1ミクロバッチ（256局面）の入力の組み立てと学習の1段の時間を比べる。

使い方: tools/train/zero を作業ディレクトリとし、env.shを読み込んだうえで
PYTHONPATH=. .venv/bin/python <このファイル> <自己対局のMNSDファイル>
学習の1段は、コンパイルしないbf16の順伝播、逆伝播、およびSGDの更新である。
"""
import sys, time
import numpy as np, torch
from minase_zero.data import read_file
from minase_zero.encoding import encode_planes
from minase_zero.net import ZeroNet

data, _ = read_file(sys.argv[1])
rng = np.random.default_rng(1)
dev = torch.device("cuda")
idx = rng.choice(len(data.sides), 256, replace=False)

def enc():
    b, s, l = (torch.from_numpy(a[idx]).to(dev) for a in (data.boards, data.sides, data.lions))
    return encode_planes(b, s, l)

def timed(fn, n=30):
    for _ in range(5): fn()
    torch.cuda.synchronize(); t = time.perf_counter()
    for _ in range(n): fn()
    torch.cuda.synchronize(); return (time.perf_counter() - t) / n * 1e3

print("encode_planes ms/microbatch256:", round(timed(enc), 2))
import minase_mnsd_reader as r
b = data.boards[idx]; s = data.sides[idx]
t = time.perf_counter()
for _ in range(30): r.control_planes(b, s)
print("control_planes CPU only ms:", round((time.perf_counter() - t) / 30 * 1e3, 2))

net = ZeroNet(20, 256).to(dev).train().to(memory_format=torch.channels_last)
x = enc().contiguous(memory_format=torch.channels_last)
opt = torch.optim.SGD(net.parameters(), lr=1e-3)
def step():
    opt.zero_grad(set_to_none=True)
    with torch.autocast("cuda", dtype=torch.bfloat16):
        p, v = net.forward_logits(x)
    (p.float().square().mean() + v.float().square().mean()).backward()
    opt.step()
print("eager bf16 fwd+bwd+sgd ms/microbatch256:", round(timed(step, 10), 1))
