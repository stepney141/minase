#!/usr/bin/env bash
# Usage: run.sh <worktree> <out-dir>
# Reproduces random-net-output-check.md: writes the random-initialized networks, evaluates the
# fixed positions with each execution provider and batch size, computes the PyTorch reference,
# and compares every output with it. Exit codes are printed rather than stopping at the first
# failure, so that the abnormal exits of the CUDA execution provider are recorded.
set -u
WT=$(realpath "$1")
OUT=$(realpath -m "$2")
source "$WT/tools/train/zero/env.sh"
mkdir -p "$OUT/nets" "$OUT/eval" "$OUT/trt-cache"
B="$WT/target/release/minase"
NET="$OUT/nets/zero-b10c128.onnx"

cd "$WT/tools/train/zero"
uv run --frozen python feasibility/make_nets.py --out-dir "$OUT/nets"
sha256sum "$OUT"/nets/*.onnx

run() {
  local name=$1
  shift
  "$B" dev nn-eval --nn-model "$NET" "$@" --out "$OUT/eval/$name.json" 2> "$OUT/eval/$name.stderr"
  echo "$name exit=$? $(tr '\n' ' ' < "$OUT/eval/$name.stderr")"
}
for b in 1 7 256; do run "cpu-b$b" --backend cpu --batch "$b"; done
for b in 1 7 256; do run "trt-b$b" --backend tensorrt --cache-dir "$OUT/trt-cache" --batch "$b"; done
for b in 1 7 256; do run "trt-b$b-cached" --backend tensorrt --cache-dir "$OUT/trt-cache" --batch "$b"; done
for i in 1 2 3 4; do run "cuda-b7-run$i" --backend cuda --batch 7; done

uv run --frozen python nn_torch_reference.py --onnx "$NET" --blocks 10 --channels 128 \
  --fixture fixtures/nn-positions.jsonl --out "$OUT/eval/pytorch.json"
echo "pytorch exit=$?"
compare() {
  echo -n "$1 tolerance=$2: "
  uv run --frozen python nn_compare.py --reference "$OUT/eval/pytorch.json" \
    --candidate "$OUT/eval/$1.json" --tolerance "$2"
  echo "exit=$?"
}
for name in cpu-b1 cpu-b7 cpu-b256; do compare "$name" 1e-4; done
for name in trt-b1 trt-b7 trt-b256 trt-b1-cached trt-b7-cached trt-b256-cached cuda-b7-run1; do
  compare "$name" 1e-2
done
