# lolcat `-a` adversarial edge-case audit (archived)

Source: subagent audit requested by the maintainer, run against code at
commit `4961aee`. The original text is preserved verbatim in
[§ Original audit](#original-audit-verbatim); the status table below tracks
what has since been fixed. Line references in the original text refer to
the `4961aee` tree and may have drifted.

Suite size at audit time: 47 tests. Current: 61 tests.

## Status at a glance

| ID | Finding | Status |
|----|---------|--------|
| A1 | Tab kept as a glyph cell (no expansion) | Fixed — config-only `tab` key (N spaces, or `-1` = 8-column tab stops; default 8). Every paint path expands tabs now, including the `-a` reveal, so the leftover flash glyph at the tab column is gone. |
| A2 | `\r` as an ordinary cell / CRLF phantom column | Fixed `12f4d15` (CRLF endings folded in `split_lines`) |
| A3 | Interior C0 (BEL/BS/VT/FF/NUL) become cells | Fixed `12f4d15` — implemented as *any remaining interior C0 degrades the whole input to plain* (the "simplest, rule-1-compatible" option; not the drop-BEL/BS variant) |
| A4 | Non-`m`/`G`/`C` CSI silently dropped (content flattened) | **Open** (whitelist-degrade not implemented) |
| A5 | Backward/overwrite `G`/`C` unrepresentable | Folded into A4; **open** with it |
| A6 | Escape/OSC state lost; truncated-tail garbage | **Partial** — truncated-tail dropping fixed `12f4d15`; OSC 8/0 retention still open (documented limitation) |
| A7 | Wide/zero-width/ZWJ counted as width-1 | Fixed — `unicode-width` now drives the width gate, the blank pre-print, `put_cell` display columns and the reveal geometry; zero-width characters glue to their base cell; tab stops and `CSI n G` count display columns. ZWJ *grapheme clusters* are still per-cell (documented partial support: a family emoji is two wide cells, not one cluster). |
| A8 | Strict `< cols` gate can silently disable animation (fastfetch ≤ ~90 cols) | **Open** — needs real-pty verification + doc |
| A9 | Terminal size source / degenerate sizes | Fixed `12f4d15` (`$LINES`/`$COLUMNS` fallback, sampled once — documented) |
| A10 | No trailing `\n` → prompt overwrites the last animated row | Fixed `12f4d15` (cursor moved past final content) |
| A11 | Blank-only input collapses | Fixed `12f4d15` (blank-only goes through the plain painter) |
| A12 | First Ctrl-C swallowed during `-a` (stdin pipe) | **Open** |
| A13 | Interrupt mid-reveal leaves half-state | **Open** — accept + document |
| A14 | `-d 1` disables pacing entirely | Fixed `12f4d15` (band boundary sleeps at `-s`) |
| A15 | `-s nan` panics | Fixed `62678da` (`validate()` rejects non-finite `--speed`) |
| A16 | `csi_params` u64 overflow panic / unbounded pad OOM | **Open** |
| A17 | `-a -B` silently disables animation | Fixed `12f4d15` (help + README document `-B` overrides `-a`) |
| A18 | `-i` mode paints rainbow backgrounds on translated pad spaces | **Open** |
| A19 | Hue bookkeeping `-a` vs plain (one-row offset; per-chunk restart) | **Open** |
| A20 | Test suite not hermetic w.r.t. `COLORTERM` | Fixed `12f4d15` (render tests clear `COLORTERM`) |
| A21 | Block-chunk/tall-input interplay | Informational — no defect beyond A19/A8 |

Commits: `12f4d15` = "Degrade control-byte rows; fold CRLF; fix -a tail edge
cases"; `62678da` = "config: read lolcat.config.jsonc for option defaults"
(includes the `--speed` finiteness check).

## Remaining backlog

Priority order (likelihood × severity, per the original §B):

1. **A4 (+A5)** — whitelist-degrade: during `animate()`'s parse pass, if a
   line contains a CSI whose final byte ∉ {`m`,`G`,`C`} (or an OSC/DCS),
   route the whole input to `paint_stream`. Makes rule 1 uniform and
   future-proofs fastfetch-style vocabulary growth.
2. **A16** — saturating `csi_params` digit accumulation, `usize::try_from`
   with clamp, cap pad growth (≤ 2×cols).
3. **A8** — verify under a real pty at 80 / ≥100 cols; document the exact
   degrade condition.
4. **A12** — expose `interrupt_seen()` from `sigint.rs`, check it in the
   reveal loop, abort with `END_RESET` + exit 130 (no `?1049l`).
5. **A18 / A19 / A6-OSC8 / A13** — `-i` pad background reset; shared hue
   model + cumulative row offset across chunks; keep OSC 8 as a per-cell
   fmt prefix (or document); document the interrupt half-state.

---

## Original audit (verbatim)

Environment caveat (evidence rules): this sandbox cannot allocate a pty
(`script: openpty: Operation not permitted`), so live `-a` reveal frames
could not be captured. Everything below that touches the animated path was
verified by **code reading plus temporary in-module unit probes** of
`parse_line` (inserted, run, then reverted; `git status` clean, 47/47 tests
green) and by **live byte captures of the non-tty paths** (`lolcat -f`
streams, anchor path, fastfetch/cmatic/pipes.sh census). Facts are labeled
*measured* vs *read*.

Measured anchor points used below:

- `parse_line` cell model (temp unit test, then reverted): `"ab\tc"` →
  `a b TAB c`; `"ab\r"` → `a b CR`; `"a\x07b"` → `a b BEL b`;
  `"a\x0bb"` → `a b VT b`; `"a\xffb"` → `a b` (invalid byte dropped);
  `"ab\x1b[31"` → `a b [ 3 1`; `"ab\x1b[10Gcd"` → 10 cells;
  `"ab\x1b[1Gd"` → `a b d` (no reposition, no overwrite); `"你好"` → 2 cells;
  `"A\u0301x"` → 3 cells; OSC8/OSC0 stripped, `L`/`T` kept.
- Live: `lolcat -f` on `a\r\n` emits an SGR-wrapped colored CR; on tabs
  emits 8 plain spaces; spaces pass uncolored (render.rs:32-34).
- Live: fastfetch (non-tty census, TERM=dumb): 13× `CSI G` all with param
  `47`, 18× `CSI m`; zero CR/tab/BEL/`ESC7/8`/`?25`/any other CSI final.
  Two rows reach content width 90-91 (>80).
- Live: `printf 'a\x1b[18446744073709551616Gx\n' | target/debug/lolcat -B -f`
  → **panic rc=101** `attempt to add with overflow` at ansi.rs:196; 21-digit
  param → `attempt to multiply with overflow`.
- Live: `COLORTERM=truecolor cargo test` → **5 render tests fail**
  (engine.rs:31-40 honors env); with `COLORTERM=` unset, 47 pass.

---

## (A) Grouped findings

### Group 1 — The row model and the C0/CSI control plane (`parse_line`, areas 1/3/7/10)

**A1. Tab is not expanded; the raw `\t` becomes a glyph cell.** *Trigger:*
any tabbed text (`Makefile`/README rows, `column`, `ps`-style output)
through `-a`. *Behavior (measured):* `parse_line` pushes every ASCII byte as
a cell (animate.rs:173-193), so `"ab\tc"` yields cells `a b TAB c`. The
plain path expands to 8 spaces (stream.rs:71-81; render.rs:58). On freeze,
the TAB cell is written mid-block (put_cell, animate.rs:203-237), the
terminal jumps to the next stop, and subsequent cells of that row are drawn
from the parked origin at their own indices → final frame `"ab c"`-shaped,
**different from both plain `lolcat` (c at col 9) and a raw terminal (c at
col 8)**. *Impact:* core acceptance criterion ("final frozen frame ==
plain") broken for tabbed input. *Fix:* in `parse_line` expand `\t` to 8
spaces (byte-parity with `paint_stream`), or to spaces-to-next-tab-stop
(screen-parity with a terminal). *Effort:* S.

**A2. `\r` is an ordinary cell; CRLF text gets a phantom trailing column.**
*Trigger:* CRLF files (Windows/WSL/`unzip`-extracted text) or any producer
using `\r`. *Behavior (measured):* lines split only on `\n`
(animate.rs:279) keep the `\r`; parse keeps it as a cell (`"ab\r\n"` line →
`a b CR`, 3 cells). Freeze prints a real CR byte (cursor to col 0 mid-row —
harmless but the blank-block space under the CR cell is never repainted);
row width is +1, so a CRLF line with content `cols-1` wide silently trips
the width gate (animate.rs:289) and kills the animation, while plain lolcat
colors it (stream.rs:82-96 paints CR as an invisible colored byte, visually
content-only). *Fix:* strip one trailing `\r` per split line (CRLF = one row
end, matching terminal reality and plain's visual effect). *Effort:* S.

**A3. Interior C0 controls (BEL 0x07, BS 0x08, VT 0x0B, FF 0x0C, NUL)
become glyph cells.** *Behavior (measured):* all five land in `row.chars`
(animate.rs:173-193). Consequences: VT/FF bytes **move the cursor down a row
inside put_cell**, after which the trailing `\r`+`B` park
(animate.rs:229-235) lands one row too low and every subsequent cell in that
block drifts; BEL rings when frozen (during the reveal, not at the natural
point in the stream); BS/NUL are invisible but count toward width/gate.
Plain lolcat passes them raw (stream.rs `_` arm). *Impact:* silent
corruption/bells on control-bearing input. *Fix:* drop BEL/BS/NUL as no-op
cells, treat VT/FF as row breaks or (simplest, rule-1-compatible) make any
interior C0 → degrade whole input to plain. *Effort:* S.

**A4. Every cursor-move/erase CSI that isn't `m`, `G`, or forward `C` is
silently dropped — input gets flattened, not degraded.** *Trigger:* any
newline-delimited stream that repositions the cursor: `ESC[nA/B/E/F`,
`ESC[n;mH/f`, `ESC[nd`, `ESC[nG` *backwards*, `ESC[K/J/X/P/@`,
`ESC[?25l/h`, `ESC 7/8`, `ESC D/M/E`, spinners using `\r`+redraw, palette
rows using `ESC[1G`. *Behavior (read + measured):* parse_line:140-171 keeps
only SGR (`final=='m'`) and pads only `G`/`C` forward; everything else is
consumed and discarded (`"ab\x1b[2Acd"` → `a b c d`; `"ab\x1b[1Gd"` →
`a b d` — the real screen would show `d b`, i.e. overwrite semantics lost).
The block reveal then draws a *concatenated* line that never existed. This
violates rule 1: it is an animation geometry the code cannot represent, yet
instead of degrading to plain (which passes everything through faithfully,
stream.rs:52-58) it fabricates corrupted content. *Fix:* whitelist-degrade —
during `animate()`'s parse pass, if any line contains a CSI whose final byte
∉ {`m`,`G`,`C`}, or an OSC/DCS of any kind (except BEL/ST-terminated titles
that can be dropped), route the whole input to `paint_stream` (the existing
degrade slot, animate.rs:290-293). This makes rule-1 behavior uniform and
future-proofs fastfetch-style vocabulary growth. *Effort:* M.

**A5. Backward/overwrite semantics are inherently unrepresentable as
padding (even after A4's whitelist, `G`/`C` pointing behind the current
column are mis-translated).** *Behavior (measured):* `ESC[1G`/`ESC[0G`
produce 0 pad (`p0.saturating_sub(1).saturating_sub(len)`, animate.rs:158)
and the following text is appended — plain passes the move through and the
next chars overwrite. *Impact:* only matters for overwrite-style producers;
with A4's degrade these land in plain mode where they are correct.
*Verdict:* fold into A4; nothing more to fix. (fastfetch's actual usage —
only forward `ESC[47G` — measured in census — is exactly representable.)

**A6. Escape/OSC state is lost in `-a` vs plain: hyperlinks OSC 8, title
OSC 0/1/2, charset designations `ESC(B`/`ESC(0`, C1 controls, and truncated
escapes become visible garbage.** *Behavior (measured):* OSC8/OSC0 →
dropped, link text kept; `ESC(B`/`ESC(0` → dropped (their line-drawing
payload bytes, if any, are re-interpreted as ordinary chars); C1/invalid
UTF-8 bytes → silently dropped (`a\xffb` → `ab`, parse_line:186-191); a
*truncated* trailing escape becomes text — `"ab\x1b[31"` → `a b [ 3 1`
(parse_line:169 advances by 1 byte), while plain lolcat buffers it and emits
the incomplete CSI raw at EOF (stream.rs:138-142), which terminals discard.
*Impact:* `ls --hyperlink=always`-style pipes lose clickable links under
`-a`; a producer killed mid-escape shows a stray `[31` in the frozen frame.
*Fix:* drop a partial escape *tail* (don't turn it into `[`+digits) — S; keep
OSC 8 as a per-cell fmt prefix — L (or document as limitation). *Effort:* S
for the truncation part.

### Group 2 — Cell/column geometry vs real display width (areas 2/4/5)

**A7. Wide (CJK), zero-width combining, and ZWJ sequences are all counted as
width-1 cells.** *Trigger:* any CJK text, decomposed accents (`A`+U+0301),
or emoji ZWJ sequences. *Behavior (measured + read):* `"你好"` → 2 cells but
occupies 4 terminal columns. Every consumer assumes 1 cell = 1 column: width
gate (animate.rs:289, `chars.len() >= cols`), blank pre-print (322-329,
prints `chars.len()` spaces), `put_cell` right-move by cell index (218-222),
`tau_of`/`by_step` (241-243, 367-373). Freeze writes a 2-col glyph at col
`c`, then the next cell's `CSI c+1 C` lands on the second half of the
previous glyph → garbled reveals. Combining marks shift all following cells
right by their count; a family emoji (8 codepoints, ~2 columns) over-counts
and either mis-animates or trips the gate. *Impact:* total corruption of
those rows + wrong gate decisions; no crash. *Fix:* track per-cell display
width (e.g. `unicode-width` crate): gate on summed width, print
width-spaces in the blank block, and have `put_cell` right-move to the
*display column* (prefix sums); treat width-0 marks by writing them at the
current cell; accept that ZWJ grapheme clusters need cluster segmentation
(document partial support). *Effort:* M.

**A8. The strict `<` gate is computed on translated cell count and can
silently disable the flagship animation.** *Trigger:* any row whose
*translated* width ≥ terminal columns — including fastfetch rows on narrower
terminals (measured non-tty fastfetch rows reach col 90-91, and the pad-to-46
translation then adds the value width on top, so at an 80-col terminal the
translated rows exceed 80 unless fastfetch truncates values in tty mode).
*Behavior (read):* one such row routes the *whole* input to plain
(animate.rs:289-293). *Impact:* animation silently off for `fastfetch |
lolcat -a` on common 80-col terminals — sanctioned by the degrade rule, but
surprising and worth documenting/measuring; also the gate can be tripped by
pad translation itself (`ESC[999G` on an 80-col terminal). *Fix:* verify
under a real pty at 80 and ≥100 cols (see §C); document the exact degrade
condition in `--help`. *Effort:* verify+doc S.

**A9. Terminal size source and degenerate sizes.** *Behavior (read):* only
`TIOCGWINSZ` on stdout (animate.rs:42-60); no `COLUMNS`/`LINES` fallback;
`rows==0`/`cols==0` resolve safely (chunk_h = `rows-1`.max(1),
animate.rs:295; any non-empty row degrades at cols 0); `rows==1` animates
one-line blocks. *Impact:* benign; document as limitation (no
SIGWINCH/reactivity needed since the geometry is sampled once per run).
*Effort:* doc.

### Group 3 — Reveal mechanics, end-state, interrupts (areas 6/9)

**A10. Input without a trailing `\n`: the reveal parks the cursor at column
0 of the last row, so the shell prompt/next output overwrites the beginning
of the last frozen row.** *Behavior (read):* every cell write parks at col 0
of the block's last row (put_cell:229-235) and animate emits a newline *only*
when `raw.ends_with(b"\n")` (animate.rs:305-308). Plain lolcat leaves the
cursor just past the last character, so the prompt continues harmlessly.
*Impact:* `printf 'hi' | lolcat -a` (or a `$(...)` substitution stripped of
its newline) shows "hi", then the prompt types over it. *Fix:* after the
final block, when the input lacks a trailing `\n`, move the cursor to just
past the last content cell (`\x1b[{last_row_width}C`, cursor already at col
0) — exact plain-parity — or emit a newline. *Effort:* S.

**A11. Blank-only input collapses.** *Behavior (read):* `"\n"` → rows `[""]`
→ popped (animate.rs:282-284) → `all.is_empty()` → returns having printed
nothing (285-287); plain prints one blank line. N blank lines animate N-1
newlines + dummy dwells of `duration` frames over empty bands (383-399).
*Fix:* emit the popped trailing row count or skip dwell when the block has
zero cells. *Effort:* S.

**A12. First Ctrl-C is swallowed during `-a` when stdin is a pipe — the
animation runs to completion and exits 0.** *Behavior (read):* the drain
decision is made at startup from stdin type (main.rs:72-73); the handler
only flags the first interrupt when draining (sigint.rs:51-53, 72-73).
During `-a` stdin is already at EOF, so there is nothing to drain and
nothing aborts the frame loop (animate.rs:375-404 never consults the flag;
`INTERRUPTS` is private to sigint.rs:46). A second Ctrl-C force-exits with
TERM_RESET containing `ESC[?1049l` (sigint.rs:39, 79) — the exact
viewport-clearing sequence the comments say Terminal.app dislikes, and the
reset bytes race the animation's own writes. *Impact:* `fastfetch | lolcat
-a` Ctrl-C appears dead for the whole reveal (seconds); interactive Ctrl-C
(stdin a tty) works but leaves a half-revealed frame (see A13). *Fix:*
expose an `interrupt_seen()` from sigint.rs and check it in the reveal loop
to abort with END_RESET + exit 130 (no `?1049l`). *Effort:* M.

**A13. Interrupt mid-reveal leaves permanent half-state.** *Behavior
(read):* the blank pre-print has already overwritten the region (322-329);
Ctrl-C (interactive) exits immediately (sigint.rs:54-65) leaving frozen
cells plus random flash glyphs from the live band on screen; colors/cursor
are reset by END_RESET, but nothing can restore the overwritten area.
*Verdict:* inherent to in-place animation; accept, and document. (Also note
`process::exit(130)` skips BufWriter flush → the tail of the interrupted
stream is lost; acceptable.)

**A14. `-d 1` disables pacing entirely.** *Behavior (read):* sleeps occur
only inside the per-band dwell loop for frames `f>0` (animate.rs:384-387);
with `duration == 1` no sleep ever runs, so `-s` has no effect and the whole
reveal completes as fast as the terminal can write. *Fix:* pace the
inter-band transition with one frame sleep as well. *Effort:* S.

### Group 4 — Flags, validation, consistency (areas 7/8/10)

**A15. `-s nan` panics (rule-1 violation).** *Behavior (read):* validation
checks only `speed < 0.1` (cli.rs:97-99); NaN passes (NaN is not < 0.1).
`animate_block` computes `Duration::from_secs_f64(1.0 / speed)`
(animate.rs:363-364) which panics on NaN (std behavior; reachable whenever
`-a` actually runs — angle supported + real tty). Note `freq` and `angle`
get `is_finite` checks (cli.rs:91, 100) but `speed` does not. *Fix:* add
`!cli.speed.is_finite()` to the same check (or reuse the freq pattern).
*Effort:* S.

**A16. `csi_params` u64 overflow panics / OOMs.** *Trigger:* crafted input
with a 20+ digit numeric param, e.g.
`printf 'a\x1b[18446744073709551616Gx' | lolcat -B` (measured rc=101, debug
panic at ansi.rs:196 `cur.unwrap_or(0) * 10 + d`); in `-a`, a ≤u64::MAX
param (which parses without overflow) reaches the unbounded pad loop at
animate.rs:162-165 **before** the width gate (289) → memory blow-up in
release; debug additionally panics on ≥2⁶⁴ params. *Impact:* crash on
adversarial input (violates rule 1); not reachable from real tools. *Fix:*
saturating digit accumulation, cast to `usize::try_from` with a clamp, and
cap pad growth (e.g. ≤ 2×cols). *Effort:* S.

**A17. `-a` + `-B` silently disables animation; `-a` + `-C`
short-circuits.** *Behavior (read):* cat.rs:36 gates animation on
`!opts.anchor` — `-a -B` runs the anchor painter with no animation and no
warning; README/help (help.rs:37-71, README.md) don't state the precedence.
`-C` exits before any flag matters (main.rs:39-42). *Fix:* document;
optionally warn on `-a -B`. *Effort:* S (doc).

**A18. `-i` mode: pad translation paints rainbow backgrounds where plain
leaves terminal default.** *Behavior (read):* pad cells are frozen like real
cells with `fmt + colour-code + ' ' + reset` (put_cell:223-228), so under
`-i` the label→column-47 gap of a fastfetch line becomes a rainbow
background band; plain lolcat emits spaces uncolored (render.rs:32-34), so
the gap stays default background. *Impact:* visually large `-i` divergence
on pad-heavy producers. *Fix:* in invert mode write pad spaces with the
background reset (or emit them as plain spaces, as render.rs does).
*Effort:* S-M.

**A19. Hue bookkeeping diverges between `-a` and plain (cosmetic).**
*Behavior (read):* the stream painter advances `os` by `dy` before the first
character of *every* line (stream.rs:61-68, 83-85), so row `r` is painted at
`os+(r+1)·dy+c·dx`, while the animation's stable color is `os + r·dy + c·dx`
(animate.rs:349-351) — a constant one-row `dy` offset; and in multi-chunk
inputs `r` restarts at 0 per chunk (animate.rs:296-301, 368), so the hue
pattern restarts each screenful whereas plain continues. *Impact:* subtle
per-row color phase difference (obvious only at small `-F`/`-P`), and a
phase "jump" at each screenful of tall input. *Fix:* use one shared model
(S) and carry a cumulative row offset across chunks (S). *Effort:* S.

**A20. Test suite is not hermetic w.r.t. `COLORTERM`.** *Behavior
(measured):* `COLORTERM=truecolor` (typical on real desktops, and set in
this shell) makes 5 render tests fail because engine.rs:31-40 auto-selects
truecolor before render.rs tests assert 256-colour codes. *Fix:* tests must
`remove_var("COLORTERM")` or set it explicitly. *Effort:* S.

**A21. Block-chunk + fastfetch/tall-input interplay is fine, but verify
narrow-terminal behavior.** (see A8) — the frame-vs-plain comparison for
tall inputs needs a pty capture; no evidence of a defect beyond the hue
restart in A19.

---

## (B) Top 10 — ordered by likelihood × severity

| # | Finding | Tag |
|---|---------|-----|
| 1 | **A2/A1: CRLF/`\r` and TAB handling breaks -a-vs-plain frame parity on ordinary text** (trailing phantom cell; tab mismatch; CRLF width +1 can silently kill animation). | fix now |
| 2 | **A10: no trailing `\n` → prompt/next command overwrites the last animated row.** Common (`printf`, `$(...)`, files without final newline). | fix now |
| 3 | **A7: wide/combining/ZWJ treated as width-1 → garbled reveals and wrong gate.** High damage for CJK users. | fix now |
| 4 | **A4: non-`m`/`G`/`C` cursor & erase CSI silently flatten content instead of degrading.** Rule-1 violation; total corruption whenever hit; whitelist-degrade makes all future vocabulary safe. | fix now |
| 5 | **A12: first Ctrl-C swallowed during `-a` (pipe) — animation runs to end, exit 0; second Ctrl-C risks `?1049l` viewport clear.** | fix later |
| 6 | **A15: `-s nan` panics on a real tty** (crash, rule-1; also add `-d` sanity). | fix now |
| 7 | **A8: fastfetch can silently skip animation on ≤~90-col terminals (translated rows ≥ cols).** Verify with a pty; document the exact gate. | fix later (verify first) |
| 8 | **A16: `csi_params` overflow panic/OOM on crafted numeric params; unbounded pad loop before the width gate.** | fix later |
| 9 | **A18/A19: `-i` gap backgrounds and per-row hue offset make `-a` ≠ plain beyond timing.** | fix later |
| 10 | **A20: `cargo test` fails whenever `COLORTERM=truecolor` is exported (measured 5 failures).** | fix now (dev DX) |

Also worth doing cheaply: A6 truncation part, A11, A14 (S each); A3/A17/A5 as
documentation or bundled into A4's degrade; A13/A9
accept-as-documented-limitation; A21 hyperlink loss (OSC 8) — accept with a
doc note or an L-effort fmt fix.

---

## (C) Capture / verification commands (run on a real terminal; this sandbox has no pty)

```sh
# 1. Acceptance criterion: fastfetch frame parity at ≥100 cols AND the 80-col silent-degrade question (A8)
script -q /tmp/ff_a.txt sh -c 'stty cols 100 rows 40; fastfetch | lolcat -a -d 1 -s 60'
script -q /tmp/ff_p.txt sh -c 'stty cols 100 rows 40; fastfetch | lolcat'
grep -c $'\x1b\[?25l' /tmp/ff_a.txt            # >0 ⇒ animation ran
# repeat with `stty cols 80`; grep for ?25l to detect silent degrade; visually compare final frames by replaying both captures

# 2. A1/A2/A10: side-by-side frames
printf 'x\ty\r\nab\r\nlast-line-no-nl' > /tmp/t.txt
script -q /tmp/t_p.txt sh -c 'stty cols 80 rows 24; lolcat /tmp/t.txt'
script -q /tmp/t_a.txt sh -c 'stty cols 80 rows 24; lolcat -a -d 1 -s 60 /tmp/t.txt'
# compare: plain shows x + 8 spaces + y … ; -a shows a raw-tab row, phantom CR columns, and cursor parked at col 0 of "last-line-no-nl"

# 3. A7: wide text
script -q sh -c 'stty cols 80 rows 24; printf "你ab\n世界你好\n" | lolcat -a -d 1 -s 60'   # garbled reveal expected today

# 4. A15: NaN crash (needs a tty so animate_block runs)
script -q /dev/null sh -c 'printf "hello\n" | target/debug/lolcat -a -s nan'; echo rc=$?

# 5. A16: overflow (no tty needed; debug binary)
printf 'a\x1b[18446744073709551616Gx\n' | target/debug/lolcat -B -f; echo rc=$?
printf '\x1b[18446744073709551615Gx\n' | lolcat -a -f   # on a tty: watch memory (unbounded pad before the gate)

# 6. A20: hermeticity
COLORTERM=truecolor cargo test 2>&1 | tail -3          # expect 5 failures in render::tests

# 7. A12: Ctrl-C responsiveness during reveal
script -q /dev/null sh -c 'fastfetch | lolcat -a -d 9 -s 10'   # press Ctrl-C once mid-reveal; observe no abort, exit 0

# 8. A6: truncated escape at EOF
printf 'ab\x1b[31m' | script -q /dev/null sh -c 'lolcat -a'   # expect stray '[31' cells in the frozen frame vs inert raw bytes in plain mode

# 9. A11/A14 sanity
printf '\n' | script -q /dev/null sh -c 'lolcat -a'            # prints nothing vs plain's single blank line
```

*Files changed during the audit session: none (a temporary `parse_line`
probe battery in `src/animate.rs` was inserted, run, and reverted via
`git checkout`; `git status` clean, 47/47 tests green with `COLORTERM`
unset).*
