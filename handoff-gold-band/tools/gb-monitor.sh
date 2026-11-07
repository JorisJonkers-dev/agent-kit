#!/bin/zsh
# Samples Gold Band and all descendants every 4s for 30 min: new network peers, new cwds, new child commands.
OUT=$1; : > $OUT; typeset -A seen
end=$((SECONDS+1800))
while (( SECONDS < end )); do
  roots=$(pgrep -f "Gold Band.app/Contents/MacOS/gold-band-desktop")
  [[ -z $roots ]] && { sleep 4; continue }
  ps -A -o pid=,ppid= > $OUT.ps
  pids=$(python3 -I - $OUT.ps $roots <<'PY'
import sys
ppid={}
for l in open(sys.argv[1]):
    a=l.split()
    if len(a)==2: ppid.setdefault(a[1],[]).append(a[0])
seen=set(sys.argv[2:]); todo=list(seen)
while todo:
    p=todo.pop()
    for c in ppid.get(p,[]):
        if c not in seen: seen.add(c); todo.append(c)
print(",".join(sorted(seen)))
PY
)
  now=$(date +%H:%M:%S)
  for p in ${(s:,:)pids}; do
    cmd=$(ps -o command= -p $p 2>/dev/null | cut -c1-150)
    k="CMD $p $cmd"; [[ -z ${seen[$k]} && -n $cmd ]] && { seen[$k]=1; echo "$now $k" >> $OUT }
  done
  lsof -nP -a -p $pids -i 2>/dev/null | awk 'NR>1{print $1,$2,$8,$9,$10}' | sort -u | while read l; do
    k="NET $l"; [[ -z ${seen[$k]} ]] && { seen[$k]=1; echo "$now $k" >> $OUT }
  done
  lsof -nP -a -p $pids -d cwd 2>/dev/null | awk 'NR>1{print $1,$2,$9}' | sort -u | while read l; do
    k="CWD $l"; [[ -z ${seen[$k]} ]] && { seen[$k]=1; echo "$now $k" >> $OUT }
  done
  sleep 4
done
