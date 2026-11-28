#!/usr/bin/env python3
"""Measure NNUE trainer per-kernel CUDA event timings using NVRTC.

Compiles the LayerStack and Simple CUDA kernels directly with NVRTC using the
same arithmetic flags as minase-trainer (`--fmad=false --prec-div=true
--prec-sqrt=true --ftz=false`), builds reverse indices from real MNSD positions,
and records average per-kernel elapsed times in milliseconds via CUDA events.
"""

from __future__ import annotations

import argparse
import ctypes
import json
from pathlib import Path
import sys

import numpy as np
import torch


SENTINEL = 0xFFFFFFFF
COLS = 145
HIDDEN = 256
LS_ROWS = 6841
SM_KING_BUCKETS = 72
SM_BASE_FEATURES = 13392
SM_ROWS = SM_KING_BUCKETS * SM_BASE_FEATURES


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Measure per-kernel CUDA event timings for minase-trainer NNUE kernels."
    )
    parser.add_argument("--mnsd", type=Path, required=True, help="Path to input .mnsd teacher file.")
    parser.add_argument(
        "--kernels-dir",
        type=Path,
        required=True,
        help="Path to crates/minase-trainer/kernels.",
    )
    parser.add_argument(
        "--zero-py-dir",
        type=Path,
        required=True,
        help="Path to tools/train/zero (provides minase_zero.encoding).",
    )
    parser.add_argument("--out-json", type=Path, required=True, help="Destination JSON file.")
    parser.add_argument("--arch", type=str, required=True, help="CUDA arch target, e.g. sm_86.")
    parser.add_argument("--batch-size", type=int, required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--warmup", type=int, required=True)
    parser.add_argument("--repeats", type=int, required=True)
    return parser.parse_args()


def check_nvrtc(code: int) -> None:
    if code != 0:
        raise RuntimeError(f"NVRTC failed with error code {code}")


def check_cuda(code: int) -> None:
    if code != 0:
        raise RuntimeError(f"CUDA driver API failed with error code {code}")


def compile_mod(nvrtc: ctypes.CDLL, cuda: ctypes.CDLL, paths: list[Path], arch: str) -> ctypes.c_void_p:
    src = "\n".join(path.read_text(encoding="utf-8") for path in paths).encode("utf-8")
    prog = ctypes.c_void_p()
    check_nvrtc(nvrtc.nvrtcCreateProgram(ctypes.byref(prog), src, b"minase-trainer.cu", 0, None, None))
    opts = [
        f"--gpu-architecture={arch}".encode("utf-8"),
        b"--fmad=false",
        b"--prec-div=true",
        b"--prec-sqrt=true",
        b"--ftz=false",
    ]
    arr = (ctypes.c_char_p * len(opts))(*opts)
    rc = nvrtc.nvrtcCompileProgram(prog, len(opts), arr)
    if rc != 0:
        log_sz = ctypes.c_size_t()
        nvrtc.nvrtcGetProgramLogSize(prog, ctypes.byref(log_sz))
        buf = ctypes.create_string_buffer(log_sz.value)
        nvrtc.nvrtcGetProgramLog(prog, buf)
        raise RuntimeError(f"NVRTC compile failed ({rc}):\n{buf.value.decode('utf-8', errors='replace')}")
    ptx_sz = ctypes.c_size_t()
    check_nvrtc(nvrtc.nvrtcGetPTXSize(prog, ctypes.byref(ptx_sz)))
    ptx = ctypes.create_string_buffer(ptx_sz.value)
    check_nvrtc(nvrtc.nvrtcGetPTX(prog, ptx))
    nvrtc.nvrtcDestroyProgram(ctypes.byref(prog))
    mod = ctypes.c_void_p()
    check_cuda(cuda.cuModuleLoadData(ctypes.byref(mod), ptx))
    return mod


def get_fn(cuda: ctypes.CDLL, mod: ctypes.c_void_p, name: str) -> ctypes.c_void_p:
    fn = ctypes.c_void_p()
    check_cuda(cuda.cuModuleGetFunction(ctypes.byref(fn), mod, name.encode("utf-8")))
    return fn


def launch(cuda: ctypes.CDLL, fn: ctypes.c_void_p, gx: int, gy: int, args: list[object]) -> None:
    c_args = []
    ptrs = []
    for arg in args:
        val = ctypes.c_uint64(arg.data_ptr()) if isinstance(arg, torch.Tensor) else arg
        c_args.append(val)
        ptrs.append(ctypes.cast(ctypes.byref(val), ctypes.c_void_p))
    arr = (ctypes.c_void_p * len(ptrs))(*ptrs)
    stream = ctypes.c_void_p(torch.cuda.current_stream().cuda_stream)
    check_cuda(cuda.cuLaunchKernel(fn, int(gx), int(gy), 1, 128, 1, 1, 0, stream, arr, None))


def time_fn(fn_call, warmup: int, reps: int) -> float:
    for _ in range(warmup):
        fn_call()
    torch.cuda.synchronize()
    start = torch.cuda.Event(enable_timing=True)
    end = torch.cuda.Event(enable_timing=True)
    start.record()
    for _ in range(reps):
        fn_call()
    end.record()
    torch.cuda.synchronize()
    return float(start.elapsed_time(end) / reps)


def build_features(
    boards: np.ndarray, sides: np.ndarray, piece_code_planes: np.ndarray
) -> tuple[np.ndarray, np.ndarray]:
    batch_size = boards.shape[0]
    feat_a = np.full((batch_size, 2, COLS), SENTINEL, dtype=np.uint32)
    feat_hk = np.full((batch_size, 2, COLS), SENTINEL, dtype=np.uint32)
    for b in range(batch_size):
        occ = np.flatnonzero(boards[b])
        pc = boards[b, occ]
        planes0 = piece_code_planes[pc]
        for v in range(2):
            view = int(sides[b]) ^ v
            rel_plane = (planes0 + view * 47) % 94
            sq = occ if view == 0 else (143 - occ)
            base = rel_plane * 144 + sq
            f_file = sq % 12
            crow = (base // 144) * 72 + (sq // 12) * 6 + np.minimum(f_file, 11 - f_file)
            feat_a[b, v, occ] = crow * 2 + (f_file >= 6).astype(np.uint32)

            own_kings = occ[rel_plane == 11]
            anchor = int(own_kings[0]) if len(own_kings) else int(occ[0])
            af, ar = anchor % 12, (anchor // 12 if view == 0 else 11 - anchor // 12)
            mir = af < 6
            af_m = (11 - af) if mir else af
            bucket = 6 * ar + (11 - af_m)
            keep = (rel_plane % 47) != 11
            rp = rel_plane[keep]
            rc = rp // 47
            st = rp % 47
            st = np.where(st < 11, st, st - 1)
            osq = occ[keep]
            rr = (osq // 12) if view == 0 else (11 - osq // 12)
            rf = (11 - (osq % 12)) if mir else (osq % 12)
            rows_hk = bucket * SM_BASE_FEATURES + (rc * 46 + st) * 144 + rr * 12 + rf
            feat_hk[b, v, : len(rows_hk)] = rows_hk.astype(np.uint32)
    return feat_a, feat_hk


def build_reverse_index(
    feat: np.ndarray, rows: int, a_encoded: bool
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    flat = feat.reshape(-1)
    valid_pos = np.flatnonzero(flat != SENTINEL)
    vals = flat[valid_pos]
    row_ids = (vals // 2) if a_encoded else vals
    order = np.argsort(row_ids, kind="stable")
    sorted_pos = valid_pos[order].astype(np.uint32)
    sorted_vals = vals[order]
    sorted_rows = row_ids[order]
    counts = np.bincount(sorted_rows, minlength=rows).astype(np.uint32)
    starts = np.zeros(rows + 1, dtype=np.uint32)
    np.cumsum(counts, out=starts[1:])
    pos_idx = sorted_pos // (2 * COLS)
    view_idx = (sorted_pos // COLS) % 2
    col_idx = sorted_pos % COLS
    rot = (sorted_vals & 1) if a_encoded else np.zeros_like(sorted_vals)
    entries = (pos_idx << 10) | (view_idx << 9) | (col_idx << 1) | rot
    seg_starts = [0]
    seg_offsets = [0]
    for r in range(rows):
        s0, s1 = int(starts[r]), int(starts[r + 1])
        off = s0
        while off < s1:
            off = min(off + 128, s1)
            seg_offsets.append(off)
        seg_starts.append(len(seg_offsets) - 1)
    return (
        np.array(seg_starts, dtype=np.uint32),
        np.array(seg_offsets, dtype=np.uint32),
        entries.astype(np.uint32),
        counts,
    )


def u32_gpu(arr: np.ndarray) -> torch.Tensor:
    return torch.from_numpy(arr.view(np.int32)).cuda()


def main() -> None:
    args = parse_args()
    if args.batch_size <= 0 or args.batch_size % 128 != 0 or args.repeats <= 0 or args.warmup < 0:
        raise ValueError("batch-size must be positive multiple of 128; repeats > 0; warmup >= 0")

    sys.path.insert(0, str(args.zero_py_dir))
    import minase_mnsd_reader
    from minase_zero.encoding import PIECE_CODE_PLANES

    torch.cuda.init()
    _ = torch.zeros(1, device="cuda")
    nvrtc = ctypes.CDLL("libnvrtc.so")
    cuda = ctypes.CDLL("libcuda.so")

    raw = minase_mnsd_reader.read(str(args.mnsd))
    n_all = len(np.frombuffer(raw["sides"], dtype=np.uint8))
    rng = np.random.default_rng(args.seed)
    idx = rng.choice(n_all, size=args.batch_size, replace=False)
    boards = np.frombuffer(raw["boards"], dtype=np.uint8).reshape(n_all, 144)[idx] & 0x7F
    sides = np.frombuffer(raw["sides"], dtype=np.uint8)[idx]
    outcomes_np = np.frombuffer(raw["outcomes"], dtype=np.uint8)[idx]

    piece_table = np.array(PIECE_CODE_PLANES, dtype=np.int32)
    feat_a, feat_hk = build_features(boards, sides, piece_table)
    ss_a, so_a, ent_a, cnt_a = build_reverse_index(feat_a, LS_ROWS, True)
    ss_hk, so_hk, ent_hk, cnt_hk = build_reverse_index(feat_hk, SM_ROWS, False)
    nz_hk = cnt_hk[cnt_hk > 0]

    # Measure LayerStack
    mod_ls = compile_mod(
        nvrtc,
        cuda,
        [
            args.kernels_dir / "layerstack" / "forward.cu",
            args.kernels_dir / "layerstack" / "backward.cu",
            args.kernels_dir / "layerstack" / "optimizer.cu",
        ],
        args.arch,
    )
    B = args.batch_size
    H = HIDDEN
    head_ls = 128 + 16 * H + 16 + 960 + 32 + 32 + 1
    n_param_ls = LS_ROWS * H + H // 2 + LS_ROWS + 16 * H + 16 + 16 * H + 16 + 960 + 32 + 32 + 1
    w_ls = torch.randn(n_param_ls, device="cuda", dtype=torch.float32) * 0.01
    g_ls = torch.zeros_like(w_ls)
    m_ls = torch.zeros_like(w_ls)
    v_ls = torch.zeros_like(w_ls)
    fwd_ls = torch.zeros(B * (3 * H + 82), device="cuda", dtype=torch.float32)
    bwd_ls = torch.zeros(B * (2 * H + 49), device="cuda", dtype=torch.float32)
    part_stat_ls = torch.zeros((B // 128) * 14, device="cuda", dtype=torch.float64)
    sums_ls = torch.zeros(14, device="cuda", dtype=torch.float64)
    head_part_ls = torch.zeros((B // 128) * head_ls, device="cuda", dtype=torch.float64)
    sparse_part_ls = torch.zeros((len(so_a) - 1) * (H + 1), device="cuda", dtype=torch.float64)
    targets_gpu = torch.full((B,), 0.5, device="cuda", dtype=torch.float32)
    outcomes_gpu = torch.from_numpy(outcomes_np).cuda()
    nonfinite = torch.zeros(1, device="cuda", dtype=torch.int32)
    d_feat_a = u32_gpu(feat_a.reshape(-1))
    d_ss_a, d_so_a, d_ent_a = u32_gpu(ss_a), u32_gpu(so_a), u32_gpu(ent_a)
    fn_ls = {
        k: get_fn(cuda, mod_ls, k)
        for k in (
            "ls_forward",
            "ls_statistics",
            "ls_statistics_reduce",
            "ls_backward",
            "ls_head_partials",
            "ls_head_reduce",
            "ls_sparse_partials",
            "ls_sparse_reduce",
            "ls_adamw",
        )
    }

    ls_ms = {
        "ls_forward": time_fn(lambda: launch(cuda, fn_ls["ls_forward"], B, 1, [d_feat_a, w_ls, fwd_ls, ctypes.c_uint32(H), ctypes.c_uint32(LS_ROWS), ctypes.c_uint32(COLS), ctypes.c_uint32(1), nonfinite]), args.warmup, args.repeats),
        "ls_statistics": time_fn(lambda: launch(cuda, fn_ls["ls_statistics"], B // 128, 14, [fwd_ls, targets_gpu, outcomes_gpu, part_stat_ls, ctypes.c_uint32(B), ctypes.c_uint32(H), ctypes.c_double(1.0), ctypes.c_double(733.0), nonfinite]), args.warmup, args.repeats),
        "ls_statistics_reduce": time_fn(lambda: launch(cuda, fn_ls["ls_statistics_reduce"], 14, 1, [part_stat_ls, sums_ls, ctypes.c_uint32(B // 128)]), args.warmup, args.repeats),
        "ls_backward": time_fn(lambda: launch(cuda, fn_ls["ls_backward"], B, 1, [w_ls, fwd_ls, targets_gpu, bwd_ls, ctypes.c_uint32(B), ctypes.c_uint32(H), ctypes.c_uint32(LS_ROWS), ctypes.c_double(1.0), nonfinite]), args.warmup, args.repeats),
        "ls_head_partials": time_fn(lambda: launch(cuda, fn_ls["ls_head_partials"], (head_ls + 31) // 32, B // 128, [fwd_ls, bwd_ls, head_part_ls, ctypes.c_uint32(B), ctypes.c_uint32(H), ctypes.c_uint32(head_ls)]), args.warmup, args.repeats),
        "ls_head_reduce": time_fn(lambda: launch(cuda, fn_ls["ls_head_reduce"], head_ls, 1, [head_part_ls, g_ls, ctypes.c_uint32(B // 128), ctypes.c_uint32(H), ctypes.c_uint32(LS_ROWS), ctypes.c_uint32(head_ls), nonfinite]), args.warmup, args.repeats),
        "ls_sparse_partials": time_fn(lambda: launch(cuda, fn_ls["ls_sparse_partials"], len(so_a) - 1, 1, [d_so_a, d_ent_a, bwd_ls, sparse_part_ls, ctypes.c_uint32(H), ctypes.c_uint32(1)]), args.warmup, args.repeats),
        "ls_sparse_reduce": time_fn(lambda: launch(cuda, fn_ls["ls_sparse_reduce"], LS_ROWS, 1, [d_ss_a, sparse_part_ls, g_ls, ctypes.c_uint32(H), ctypes.c_uint32(LS_ROWS), nonfinite]), args.warmup, args.repeats),
        "ls_adamw": time_fn(lambda: launch(cuda, fn_ls["ls_adamw"], (n_param_ls + 127) // 128, 1, [w_ls, g_ls, m_ls, v_ls, ctypes.c_uint32(0), ctypes.c_uint32(n_param_ls), ctypes.c_float(1.98), ctypes.c_uint32(1), ctypes.c_double(0.1), ctypes.c_double(0.001), ctypes.c_double(0.001), ctypes.c_float(1.0), nonfinite]), args.warmup, args.repeats),
    }

    del w_ls, g_ls, m_ls, v_ls, fwd_ls, bwd_ls, part_stat_ls, sums_ls, head_part_ls, sparse_part_ls, d_feat_a, d_ss_a, d_so_a, d_ent_a
    torch.cuda.empty_cache()

    # Measure Simple (HalfKP_vm)
    mod_sm = compile_mod(
        nvrtc,
        cuda,
        [
            args.kernels_dir / "simple" / "forward.cu",
            args.kernels_dir / "simple" / "backward.cu",
            args.kernels_dir / "simple" / "optimizer.cu",
        ],
        args.arch,
    )
    head_sm = 65 * H + 1121
    n_param_sm = SM_ROWS * H + SM_BASE_FEATURES * H + H + 64 * H + 32 + 1024 + 32 + 32 + 1
    w_sm = torch.zeros(n_param_sm, device="cuda", dtype=torch.float32)
    g_sm = torch.zeros_like(w_sm)
    m_sm = torch.zeros_like(w_sm)
    v_sm = torch.zeros_like(w_sm)
    fwd_sm = torch.zeros(B * (4 * H + 129), device="cuda", dtype=torch.float32)
    bwd_sm = torch.zeros(B * (2 * H + 65), device="cuda", dtype=torch.float32)
    part_stat_sm = torch.zeros((B // 128) * 7, device="cuda", dtype=torch.float64)
    sums_sm = torch.zeros(7, device="cuda", dtype=torch.float64)
    head_part_sm = torch.zeros((B // 128) * head_sm, device="cuda", dtype=torch.float64)
    d_feat_hk = u32_gpu(feat_hk.reshape(-1))
    d_ss_hk, d_so_hk, d_ent_hk = u32_gpu(ss_hk), u32_gpu(so_hk), u32_gpu(ent_hk)
    fn_sm = {
        k: get_fn(cuda, mod_sm, k)
        for k in (
            "simple_forward",
            "simple_statistics",
            "simple_statistics_reduce",
            "simple_backward",
            "simple_head_partials",
            "simple_head_reduce",
            "simple_sparse_reduce",
            "simple_virtual_reduce",
            "simple_adamw",
        )
    }

    sm_ms = {
        "simple_forward": time_fn(lambda: launch(cuda, fn_sm["simple_forward"], B, 1, [d_feat_hk, w_sm, fwd_sm, ctypes.c_uint32(H), ctypes.c_uint32(SM_ROWS), ctypes.c_uint32(SM_BASE_FEATURES), ctypes.c_uint32(COLS), nonfinite]), args.warmup, args.repeats),
        "simple_statistics": time_fn(lambda: launch(cuda, fn_sm["simple_statistics"], B // 128, 7, [fwd_sm, targets_gpu, part_stat_sm, ctypes.c_uint32(B), ctypes.c_uint32(H), ctypes.c_double(1.0), nonfinite]), args.warmup, args.repeats),
        "simple_statistics_reduce": time_fn(lambda: launch(cuda, fn_sm["simple_statistics_reduce"], 7, 1, [part_stat_sm, sums_sm, ctypes.c_uint32(B // 128)]), args.warmup, args.repeats),
        "simple_backward": time_fn(lambda: launch(cuda, fn_sm["simple_backward"], B, 1, [w_sm, fwd_sm, targets_gpu, bwd_sm, ctypes.c_uint32(B), ctypes.c_uint32(H), ctypes.c_uint32(SM_ROWS), ctypes.c_uint32(SM_BASE_FEATURES), ctypes.c_double(1.0), nonfinite]), args.warmup, args.repeats),
        "simple_head_partials": time_fn(lambda: launch(cuda, fn_sm["simple_head_partials"], (head_sm + 31) // 32, B // 128, [fwd_sm, bwd_sm, head_part_sm, ctypes.c_uint32(B), ctypes.c_uint32(H), ctypes.c_uint32(head_sm)]), args.warmup, args.repeats),
        "simple_head_reduce": time_fn(lambda: launch(cuda, fn_sm["simple_head_reduce"], (head_sm + 31) // 32, 1, [head_part_sm, g_sm, ctypes.c_uint32(B // 128), ctypes.c_uint32(H), ctypes.c_uint32(SM_ROWS), ctypes.c_uint32(SM_BASE_FEATURES), ctypes.c_uint32(head_sm), nonfinite]), args.warmup, args.repeats),
        "memset_zeros": time_fn(lambda: g_sm[: SM_ROWS * H].zero_(), args.warmup, args.repeats),
        "simple_sparse_reduce": time_fn(lambda: launch(cuda, fn_sm["simple_sparse_reduce"], len(so_hk) - 1, 1, [d_ss_hk, d_so_hk, d_ent_hk, d_feat_hk, bwd_sm, g_sm, ctypes.c_uint32(H), ctypes.c_uint32(COLS), nonfinite]), args.warmup, args.repeats),
        "simple_virtual_reduce": time_fn(lambda: launch(cuda, fn_sm["simple_virtual_reduce"], SM_KING_BUCKETS * SM_BASE_FEATURES // SM_KING_BUCKETS, 1, [g_sm, ctypes.c_uint32(H), ctypes.c_uint32(SM_ROWS), ctypes.c_uint32(SM_BASE_FEATURES), nonfinite]), args.warmup, args.repeats),
        "simple_adamw": time_fn(lambda: launch(cuda, fn_sm["simple_adamw"], (n_param_sm + 127) // 128, 1, [w_sm, g_sm, m_sm, v_sm, ctypes.c_uint32(0), ctypes.c_uint32(n_param_sm), ctypes.c_float(1.98), ctypes.c_uint32(1), ctypes.c_float(0.1), ctypes.c_float(0.001), ctypes.c_float(0.001), ctypes.c_float(1.0), nonfinite]), args.warmup, args.repeats),
    }

    payload = {
        "batch_size": B,
        "seed": args.seed,
        "arch": args.arch,
        "sparsity": {
            "feature_set_a": {
                "entries": int(len(ent_a)),
                "nonempty_rows": int(np.sum(cnt_a > 0)),
                "allocated_rows": LS_ROWS,
                "segments": int(len(so_a) - 1),
            },
            "halfkp_vm": {
                "entries": int(len(ent_hk)),
                "nonempty_rows": int(len(nz_hk)),
                "total_real_rows": SM_ROWS,
                "king_buckets": SM_KING_BUCKETS,
                "base_features_per_king_bucket": SM_BASE_FEATURES,
                "segments": int(len(so_hk) - 1),
                "median_occurrences_per_active_row": float(np.median(nz_hk)),
                "le_4_occurrences_ratio": float(np.mean(nz_hk <= 4)),
            },
        },
        "layerstack_ms": ls_ms,
        "simple_ms": sm_ms,
    }
    args.out_json.parent.mkdir(parents=True, exist_ok=True)
    args.out_json.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
