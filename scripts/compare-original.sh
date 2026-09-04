#!/usr/bin/env bash
#
# Compare our lolcat against the original (Homebrew Ruby) lolcat on the same
# text and seed: decode the per-character truecolor codes and fit each
# version's per-column/per-row hue phase under one shared sine model, so the
# rainbow direction (angle) and frequency are directly comparable.
#
# Usage: scripts/compare-original.sh [seed] [input-bytes]
set -euo pipefail

ORIG="${ORIG:-/opt/homebrew/bin/lolcat}"
OURS="${OURS:-$HOME/.local/bin/lolcat}"
SEED="${1:-12345}"
BYTES="${2:-1024}"

inp="$(mktemp)"; t1="$(mktemp)"; t2="$(mktemp)"
trap 'rm -f "$inp" "$t1" "$t2"' EXIT

head -c "$BYTES" /dev/random | base64 | fold -w 64 > "$inp"

"$ORIG" -t -f -S "$SEED" < "$inp" > "$t1"
"$OURS" -t -f -S "$SEED" < "$inp" > "$t2"

python3 - "$inp" "$t1" "$t2" <<'PY'
import re, sys, math

inp, f1, f2 = sys.argv[1:4]

def decode(path):
    rows = []
    for raw in open(path, 'rb').read().split(b'\n'):
        codes = re.findall(rb'\x1b\[38;2;(\d+);(\d+);(\d+)m(.)', raw)
        if codes:
            rows.append([(int(r), int(g), int(b)) for r, g, b, _c in codes])
    return rows

def col_phase(r, g, b):
    """Exact phase of the shared sine model from one colour."""
    cr, cg, cb = r - 128.0, g - 128.0, b - 128.0
    return math.atan2(cr, (cg - cb) / math.sqrt(3.0))

def linfit(xs, ys):
    n = len(xs)
    if n == 0:
        return 0.0, 0.0
    mx = sum(xs) / n
    my = sum(ys) / n
    den = sum((x - mx) ** 2 for x in xs)
    if den < 1e-12:
        return 0.0, my
    b = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den
    return b, my - b * mx

def unwrap(phases):
    out = []
    prev = None
    for p in phases:
        if prev is not None:
            while p - prev > math.pi:
                p -= 2 * math.pi
            while p - prev < -math.pi:
                p += 2 * math.pi
        out.append(p)
        prev = p
    return out

def analyse(rows):
    used = [r for r in rows if len(r) > 8]
    if not used:
        return None
    dxs, t0s = [], []
    for row in used:
        ps = unwrap([col_phase(r, g, b) for (r, g, b) in row])
        xs = list(range(len(ps)))
        dx, t0 = linfit(xs, ps)
        dxs.append(dx)
        t0s.append(t0)
    t0s = unwrap(t0s)
    dxs.sort()
    dx = dxs[len(dxs) // 2]
    dys = [b - a for a, b in zip(t0s, t0s[1:])]
    dy = sum(dys) / len(dys) if dys else 0.0
    return dx, dy

def fmt(res):
    dx, dy = res
    dxd = math.degrees(dx)
    dyd = math.degrees(dy)
    cycle = 360.0 / dxd if abs(dxd) > 1e-9 else float('inf')
    angle = math.degrees(math.atan2(dy, dx))
    return dxd, dyd, cycle, angle

a = analyse(decode(f1))
b = analyse(decode(f2))
for name, res in (("original", a), ("ours", b)):
    if res is None:
        print(f"{name:9}: no decodable rows")
        continue
    dxd, dyd, cycle, angle = fmt(res)
    print(f"{name:9}: dx={dxd:7.2f}°/col  cycle={cycle:7.1f} cols  dy={dyd:+7.2f}°/line  angle={angle:6.1f}°")
if a and b:
    _, _, ca, aa = fmt(a)
    _, _, cb, ab = fmt(b)
    print("\nours    : angle %6.1f°, cycle %6.1f cols" % (ab, cb))
    print("original: angle %6.1f°, cycle %6.1f cols" % (aa, ca))
    print("to match original try:  lolcat -A %.1f -F %.1f" % (aa, ca))
PY
