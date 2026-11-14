import sys, time, tempfile
from pathlib import Path
sys.path.insert(0, sys.argv[1] + "/tests")
import setup_harness as h
args = tuple(sys.argv[2:])
root = Path(tempfile.mkdtemp())
orig = h.subprocess.run
def run(cmd, **kw):
    pass
    kw["env"]["PS4"] = '+ $EPOCHREALTIME ${LINENO} '
    cmd = ["bash", "-x"] + cmd[1:]
    return orig(cmd, **kw)
h.subprocess.run = run
t = time.time()
r = h.setup_run(root, "Linux", args=args)
print("rc", r.returncode, "secs", round(time.time() - t, 1))
open("/tmp/trace-" + "-".join(args or ("normal",)).strip("-") + ".txt", "w").write(r.stderr)
lines = [l for l in r.stderr.splitlines() if l.startswith("+") and len(l.split()) > 2]
prev = None
gaps = []
for l in lines:
    try:
        t = float(l.split()[1])
    except ValueError:
        continue
    if prev is not None and t - prev[0] > 0.3:
        gaps.append((round(t - prev[0], 2), prev[1][:220]))
    prev = (t, l)
for g in sorted(gaps, reverse=True)[:25]:
    print(*g)
