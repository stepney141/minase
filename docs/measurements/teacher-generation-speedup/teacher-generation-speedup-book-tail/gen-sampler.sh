#!/bin/bash
# Read-only sampling of the running generator: thread count and CPU ticks every 20 s for 40 minutes.
for i in $(seq 1 120); do
  timeout 25 ssh -i ~/.ssh/id_vastai -p <ポート> -o BatchMode=yes -o ConnectTimeout=15 root@<生成機のホスト> 'MP=$(pgrep -x minase | head -1); if [ -n "$MP" ]; then echo "$(date +%s) pid=$MP threads=$(ls /proc/$MP/task | wc -l) ticks=$(cut -d" " -f14,15 /proc/$MP/stat) rss_kb=$(grep VmRSS /proc/$MP/status | tr -s " \t" " " | cut -d" " -f2) load=$(cut -d" " -f1 /proc/loadavg)"; else echo "$(date +%s) none load=$(cut -d" " -f1 /proc/loadavg)"; fi' 2>/dev/null | grep -v "Welcome\|Have fun" >> /tmp/claude-1000/-home-stepney141-board-games-minase/063b77ac-3d3e-4aec-a692-b84a7d273cb6/scratchpad/gen-samples.txt
  sleep 20
done
