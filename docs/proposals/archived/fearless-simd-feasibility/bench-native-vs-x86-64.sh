#!/bin/bash
# Alternate the native and x86-64 builds of the same commit 5 rounds pinned to P-core 3 (3 measured reps each),
# count instructions with perf stat, then dump the FM kernels of an inline(never) x86-64 diagnostic build.
# Expects $W/target/release/minase (native) and $W/target-x86-64/release/minase (x86-64) to exist.
W=${W:-$(git rev-parse --show-toplevel)}
O=${O:-$W/target/fearless-simd-bench}
mkdir -p $O
R=$O/bench-native-vs-x86-64.txt
: > $R
echo "start $(date '+%F %T') load=$(cut -d' ' -f1-3 /proc/loadavg)" >> $R
for round in 1 2 3 4 5; do
  for cfg in native x86-64; do
    case $cfg in native) B=$W/target/release/minase;; x86-64) B=$W/target-x86-64/release/minase;; esac
    echo "== round=$round config=$cfg $(date +%T)" >> $R
    taskset -c 3 $B dev bench --depth 9 --repetitions 3 2>&1 | grep -E "^median|^position=initial" >> $R
  done
done
echo "== perf stat (1 rep each, instructions are load-insensitive)" >> $R
for cfg in native x86-64; do
  case $cfg in native) B=$W/target/release/minase;; x86-64) B=$W/target-x86-64/release/minase;; esac
  echo "-- $cfg" >> $R
  taskset -c 3 perf stat -x, -e instructions:u,cycles:u -o $O/perf-$cfg.txt $B dev bench --depth 9 --repetitions 1 2>&1 | grep -E "^median" >> $R
  grep -E "instructions|cycles" $O/perf-$cfg.txt >> $R
done
echo "end $(date '+%F %T') load=$(cut -d' ' -f1-3 /proc/loadavg)" >> $R
# Diagnostic x86-64 build with inline(never) on the FM kernels, to read the portable codegen.
cd $W
sed -i -e 's|    pub(super) fn add(&self, accumulator|    #[inline(never)]\n    pub(super) fn add(\&self, accumulator|' \
       -e 's|    pub(super) fn remove(&self, accumulator|    #[inline(never)]\n    pub(super) fn remove(\&self, accumulator|' \
       -e 's|    pub(super) fn replace(|    #[inline(never)]\n    pub(super) fn replace(|' \
       -e 's|    pub(super) fn correction(&self, accumulator|    #[inline(never)]\n    pub(super) fn correction(\&self, accumulator|' \
       crates/minase/src/eval/pst/fm.rs
RUSTFLAGS="-C target-cpu=x86-64" CARGO_TARGET_DIR=target-asm-x86-64 nice -n 19 cargo build --release -j1 --bin minase 2>&1 | tail -1 >> $R
git checkout -- crates/minase/src/eval/pst/fm.rs
B=$W/target-asm-x86-64/release/minase
for f in 3add 6remove 7replace 10correction; do
  ADDR=$(objdump -d --no-show-raw-insn $B | grep -E "^[0-9a-f]+ <.*2fm.*2Fm.*$f>:" | head -1 | cut -d' ' -f1)
  echo "== x86-64 codegen $f" >> $R
  objdump -d --no-show-raw-insn $B --start-address=0x$ADDR --stop-address=$((0x$ADDR + 0x800)) | awk '/^[0-9a-f]+ </{n++} n<=1' | sed -n '2,400p' | sed -E 's/^\s*[0-9a-f]+:\s*//' > $O/fm-x86-64-$f.asm
  echo "lines: $(wc -l < $O/fm-x86-64-$f.asm)" >> $R
  awk '{print $1}' $O/fm-x86-64-$f.asm | sort | uniq -c | sort -rn | head -14 | tr '\n' ';' >> $R; echo >> $R
done
echo "all done $(date +%T)" >> $R
