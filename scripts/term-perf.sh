#!/bin/sh
# Generates ~5.3 MiB of mixed plain/ANSI-colored output over ~30s.
# Run with NANOMD_TERM_STATS=1 set on nanomd to read p95/max off the pane's stats line.
i=0
start=$(date +%s)
while [ $i -lt 90000 ]; do
  printf '\033[3%dmline %06d plain text with \033[1;4%dmANSI\033[0m tail padding padding pad\n' $((i % 8)) $i $((i % 8))
  i=$((i + 1))
  [ $((i % 3000)) -eq 0 ] && sleep 1
done
echo "done: $(($(date +%s) - start))s"
