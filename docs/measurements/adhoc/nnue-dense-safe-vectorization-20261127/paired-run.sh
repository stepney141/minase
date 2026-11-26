#!/usr/bin/env bash
# Paired cycle comparison: base vs safe, alternating order per pair.
set -euo pipefail
N_BENCH=$1; N_TEACH=$2
B=/tmp/nps/target-base/release/minase; S=/tmp/nps/target-safe/release/minase
printf 'setoption name RuleSet value JCSA\nsetoption name EvalFile value /tmp/nps/netA.bin\nisready\nbench depth 16 threads 1 repetitions 1\nquit\n' > bench16.usi
EV='cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/branch-misses/u'
one_bench() { taskset -c 3 perf stat -x, -e $EV -o $3.stat -- $1 --protocol usi < bench16.usi > $3.out; echo "$2 bench $(awk -F, '{printf "%s ", $1}' $3.stat)" >> results.txt; }
one_teach() { rm -rf $3.dir; (cd /tmp/nps/base && taskset -c 3 perf stat -x, -e $EV -o /tmp/nps/pair/$3.stat -- $1 data selfplay generate --eval-file /tmp/nps/netA.bin --output-dir /tmp/nps/pair/$3.dir --chunk-games 2 --games 2 --seed 9010200000 --nodes 20000 --random-moves 0 --concurrency 1 --hash-mb 16 > /tmp/nps/pair/$3.log 2>&1); echo "$2 teach $(awk -F, '{printf "%s ", $1}' $3.stat) sha=$(sha256sum $3.dir/teacher.mnsd | cut -c1-16) $(grep searched_nodes_total $3.log)" >> results.txt; }
: > results.txt
for i in $(seq 1 $N_BENCH); do if (( i % 2 )); then one_bench $B base b$i-base; one_bench $S safe b$i-safe; else one_bench $S safe b$i-safe; one_bench $B base b$i-base; fi; diff <(grep position= b$i-base.out | sed 's/elapsed=[^ ]*//') <(grep position= b$i-safe.out | sed 's/elapsed=[^ ]*//') >/dev/null && echo "pair $i bench identical" >> results.txt; done
for i in $(seq 1 $N_TEACH); do if (( i % 2 )); then one_teach $B base t$i-base; one_teach $S safe t$i-safe; else one_teach $S safe t$i-safe; one_teach $B base t$i-base; fi; done
