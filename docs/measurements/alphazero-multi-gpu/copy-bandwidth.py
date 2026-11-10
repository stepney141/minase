# ホストとGPUの間で、バッチ128の方策の出力（float32、約18 MB）と同じ大きさの領域を、ページング可能なメモリとピン留めメモリで50回ずつ転送し、1回の時間と帯域を表示する。
# 使い方: uv run --frozen python copy-bandwidth.py <GPUの番号>
import sys, time, torch
dev = torch.device(f"cuda:{int(sys.argv[1])}")
n = 128 * 35856  # float32 elements
for pinned in (False, True):
    host = torch.empty(n, dtype=torch.float32, pin_memory=pinned)
    gpu = torch.empty(n, dtype=torch.float32, device=dev)
    for direction in ("h2d", "d2h"):
        for _ in range(3):
            (gpu.copy_(host) if direction == "h2d" else host.copy_(gpu)); torch.cuda.synchronize(dev)
        reps = 50; t = time.perf_counter()
        for _ in range(reps):
            (gpu.copy_(host) if direction == "h2d" else host.copy_(gpu))
            torch.cuda.synchronize(dev)
        dt = (time.perf_counter() - t) / reps
        print(f"pinned={pinned} {direction} {dt*1e3:.2f} ms/copy {n*4/dt/1e9:.1f} GB/s")
