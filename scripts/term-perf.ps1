# Generates ~5.3 MiB of mixed plain/ANSI-colored output over ~30s.
# Run with NANOMD_TERM_STATS=1 set on nanomd to read p95/max off the pane's stats line.
$e = [char]27
$sw = [Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 90000; $i++) {
  [Console]::WriteLine("$e[3$($i % 8)mline $i plain text with $e[1;4$($i % 8)mANSI$e[0m tail padding padding pad")
  if ($i % 3000 -eq 2999) { Start-Sleep -Milliseconds 950 }
}
"done: $([math]::Round($sw.Elapsed.TotalSeconds,1))s"
