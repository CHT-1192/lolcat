// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! New `-a` reveal animation.
//!
//! Instead of the old per-line hue fade, the whole buffered block is
//! revealed by a diagonal front that sweeps from the top-left toward the
//! bottom-right / straight down (depending on `-A`). A cell at row `r`,
//! column `c` is born at reveal time `tau = r + c·C`, where `C` is the
//! per-character cost derived from the angle:
//!
//! - `-A 45` (grid-equivalent): `C = 1` — every step adds one anti-diagonal
//!   band, so adjacent rows differ by exactly one revealed column, and a
//!   single line advances rightwards one character per step;
//! - `-A 90` (equivalent): `C = 0` — each row appears whole at once (a
//!   single line therefore appears "directly"), and the sweep goes straight
//!   down row by row.
//!
//! Every `duration` frames (at `speed` frames per second) one new band is
//! born. While a band is alive its cells flicker each frame between random
//! printable non-space glyphs and random colours; on the frame the next
//! band is born the previous band freezes into its real characters and the
//! stable stream colour for that cell.

use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

use crate::ansi::{char_width, escape_len};
use crate::color::{color_for, write_sgr};
use crate::engine::{set_mode, Engine};
use crate::options::{Options, TabWidth};
use crate::stream::paint_stream;

/// Ask the terminal for its size (rows, columns). The block path may only
/// run when the whole text fits: no more lines than the height and every
/// line strictly narrower than the width. When stdout is not a terminal we
/// take the per-line path, which needs no geometry.
///
/// The size is sampled once per run (no SIGWINCH handling): the reveal
/// geometry is fixed for the whole input.
#[cfg(unix)]
fn terminal_size() -> Option<(usize, usize)> {
    #[repr(C)]
    struct WinSize {
        rows: u16,
        cols: u16,
        xpix: u16,
        ypix: u16,
    }
    // SAFETY: ioctl with a valid pointer to writable storage.
    unsafe {
        let mut w = std::mem::MaybeUninit::<WinSize>::uninit();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, w.as_mut_ptr()) == 0 {
            let w = w.assume_init();
            let (r, c) = (w.rows as usize, w.cols as usize);
            if r > 0 && c > 0 {
                return Some((r, c));
            }
        }
    }
    // ioctl failed or reported a zero size (e.g. a pty that was never
    // resized): fall back to $LINES/$COLUMNS, but only when stdout is still
    // a terminal — animation into a non-tty never makes sense.
    if unsafe { libc::isatty(libc::STDOUT_FILENO) } != 1 {
        return None;
    }
    let env = |k: &str| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
    };
    Some((env("LINES").unwrap_or(24), env("COLUMNS").unwrap_or(80)))
}

#[cfg(not(unix))]
fn terminal_size() -> Option<(usize, usize)> {
    None
}

/// Animation angle sector: directions from "down-right (but not too
/// vertical)" to "straight down". 18.4° = 90° - default angle (71.6°);
/// the opposite sector is 180° away.
const ANIM_ANGLE_MIN: f64 = 18.4;
const ANIM_ANGLE_MAX: f64 = 90.0;

/// Whether `-a` reveal geometry supports the given angle. Unsupported
/// angles degrade to plain (non-animated) colouring instead of failing.
pub(crate) fn angle_supported(angle: f64) -> bool {
    let a = angle.rem_euclid(360.0);
    (ANIM_ANGLE_MIN..=ANIM_ANGLE_MAX).contains(&a)
        || (ANIM_ANGLE_MIN + 180.0..=ANIM_ANGLE_MAX + 180.0).contains(&a)
}

/// Random printable glyph (0x21..=0x7e, no space).
fn random_glyph() -> u8 {
    (33 + rand::random::<u8>() % 94) as u8
}

/// Random hue in degrees.
fn random_hue() -> f64 {
    rand::random_range(0.0..360.0)
}

/// Per-character reveal cost `C` for the given angle.
///
/// Anchored on the two cases the design fixes: 45° → 1 (one anti-diagonal
/// band per step), 90° → 0 (whole row at once). `C = cot(angle)` has those
/// properties (cot 45° = 1, cot 90° = 0) and grows toward the slanted
/// "down-right" end of the allowed sector.
fn char_cost(angle: f64) -> f64 {
    let a = angle.rem_euclid(360.0);
    // Map mirror sectors back to 0..=90.
    let a = if a > 180.0 { a - 180.0 } else { a };
    let a = if a > 90.0 { 180.0 - a } else { a };
    let rad = a.max(1.0).to_radians(); // avoid cot 0°
    (1.0 / rad.tan()).clamp(0.0, 12.0)
}

/// One buffered text row: its cells plus, for each cell, the ANSI SGR prefix
/// (background colours, styles, …) that must be emitted before it so
/// formatting from coloured input (e.g. fastfetch's palette blocks, which
/// are background-coloured spaces) survives.
///
/// A cell is one *base character* plus any zero-width characters (combining
/// marks, variation selectors, ZWJ) that must be written in the same cell;
/// its display width is 1 or 2 terminal columns. Cells, not characters, are
/// what the reveal positions and colours.
#[derive(Clone)]
struct Row {
    /// Base character of each cell (empty for a blank line).
    chars: Vec<char>,
    /// Per-cell input SGR prefix bytes (may be empty).
    fmts: Vec<Vec<u8>>,
    /// Zero-width bytes glued to the base glyph of each cell.
    extras: Vec<String>,
    /// Display width of each cell in columns (≥ 1, so it can be positioned).
    widths: Vec<usize>,
}

impl Row {
    fn empty() -> Row {
        Row {
            chars: Vec::new(),
            fmts: Vec::new(),
            extras: Vec::new(),
            widths: Vec::new(),
        }
    }

    /// Add one visible character: a zero-width character joins the previous
    /// cell's glyph; anything else becomes a new cell.
    fn push(&mut self, c: char, fmt: &[u8]) {
        let w = char_width(c);
        if w == 0 {
            if let Some(last) = self.extras.last_mut() {
                last.push(c);
                return;
            }
            // A leading zero-width character with nothing to attach to: keep
            // it as its own (width-1) cell so it can still be positioned.
        }
        self.chars.push(c);
        self.fmts.push(fmt.to_vec());
        self.extras.push(String::new());
        self.widths.push(w.max(1));
    }

    /// Total display width of the row in terminal columns.
    fn width(&self) -> usize {
        self.widths.iter().sum()
    }
}

/// A C0 control byte that the row/column animation model cannot represent
/// faithfully: BEL, BS, VT/FF (which move the cursor inside a row), NUL, an
/// interior `\r` (overwrite), etc. Tabs are kept as literal glyph cells,
/// `ESC` starts escape sequences consumed by `parse_line`, and `\n`/`\r`
/// never appear here — rows are split on `\n` and CRLF endings are folded
/// by [`split_lines`] first.
fn is_unrepresentable_c0(b: u8) -> bool {
    b < 0x20 && b != b'\t' && b != 0x1b
}

/// Split raw input into rows on `\n`, folding CRLF endings: a `\r` directly
/// before the row end only rewinds to column 0, which the newline already
/// implies, so it is dropped (otherwise every CRLF line would carry a
/// phantom column and could trip the width gate). Rows that still contain
/// an unrepresentable control byte make the whole input `None`, so the
/// caller can degrade to the plain painter, which passes control bytes
/// through raw. The empty row left by a trailing `\n` is kept for the
/// caller to pop.
fn split_lines(raw: &[u8]) -> Option<Vec<&[u8]>> {
    let mut bad = false;
    let rows: Vec<&[u8]> = raw
        .split(|&b| b == b'\n')
        .map(|mut line| {
            while line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.iter().any(|&b| is_unrepresentable_c0(b)) {
                bad = true;
            }
            line
        })
        .collect();
    if bad {
        None
    } else {
        Some(rows)
    }
}

/// Parse one raw input line into a `Row`: escape sequences are dropped, but
/// each SGR (`ESC [ … m`) is remembered and attached to every following
/// visible character as its formatting prefix (attributes accumulate until
/// a new SGR code replaces them, which mirrors terminal behaviour closely
/// enough for re-colouring). With `keep` disabled no prefixes are recorded,
/// so the reveal paints plain rainbow text over any input styling.
fn parse_line(line: &[u8], keep: bool, tab: TabWidth) -> Row {
    let mut row = Row::empty();
    let mut fmt: Vec<u8> = Vec::new();
    let n = line.len();
    let mut i = 0;
    while i < n {
        let b = line[i];
        if b == 0x1b {
            match escape_len(&line[i..]) {
                Some(l) => {
                    let seq = &line[i..i + l];
                    let final_byte = seq[seq.len() - 1];
                    if keep && seq.len() >= 2 && final_byte == b'm' {
                        // SGR: style the following cells.
                        fmt = seq.to_vec();
                    } else if seq.len() >= 2 && seq[1] == b'[' {
                        // Column positioning used for alignment (fastfetch
                        // pads with `ESC[47G`, `ESC[nC`, …). Translate it
                        // into real space cells so the final text keeps its
                        // column layout after the escapes are gone.
                        let params = crate::ansi::csi_params(seq);
                        let p0 = |d: usize| {
                            params.first().copied().flatten().unwrap_or(d as u64) as usize
                        };
                        let pad = match final_byte {
                            b'G' => p0(1).saturating_sub(1).saturating_sub(row.width()),
                            b'C' => p0(1),
                            _ => 0,
                        };
                        for _ in 0..pad {
                            row.push(' ', &fmt);
                        }
                    }
                    i += l;
                }
                None => {
                    // Truncated escape (no final byte before the row ends):
                    // a terminal discards it together with its trailing
                    // parameter bytes, so do we. Resume at the next ESC if
                    // one appears later in the row, else the row is done —
                    // the incomplete tail must not leak out as "[31" text.
                    match line[i + 1..].iter().position(|&x| x == 0x1b) {
                        Some(k) => i += 1 + k,
                        None => i = n,
                    }
                }
            }
            continue;
        }
        if b == b'\t' {
            // Tabs expand into cells too (the `"tab"` config key), so the
            // frozen frame keeps the plain painter's column layout instead
            // of writing a raw TAB mid-row and leaving a flash glyph behind.
            for _ in 0..tab.cells(row.width()) {
                row.push(' ', &fmt);
            }
            i += 1;
            continue;
        }
        let len = if b < 0x80 {
            1
        } else if (0xc2..=0xdf).contains(&b) {
            2
        } else if (0xe0..=0xef).contains(&b) {
            3
        } else if (0xf0..=0xf4).contains(&b) {
            4
        } else {
            1
        };
        let end = (i + len).min(n);
        let seg = &line[i..end];
        if let Ok(s) = std::str::from_utf8(seg) {
            for c in s.chars() {
                row.push(c, &fmt);
            }
        }
        i = end;
    }
    row
}

/// Draw one cell (row `r`, column `c`) of the already-printed blank block.
/// Cursor is parked at column 0 of the last row; we move up to the row,
/// right to the column, write, and park back at the block end. The cell is
/// written as: input SGR prefix (background/style), our foreground colour
/// code, the glyph, then a full reset so attributes never leak into the
/// blank cells around it.
fn put_cell(
    out: &mut dyn Write,
    rows: usize,
    r: usize,
    col: usize,
    fmt: &[u8],
    code: &[u8],
    glyph: &[u8],
) -> io::Result<()> {
    let up = rows - 1 - r;
    if up > 0 {
        out.write_all(b"\x1b[")?;
        out.write_all(up.to_string().as_bytes())?;
        out.write_all(b"A")?;
    }
    if col > 0 {
        out.write_all(b"\x1b[")?;
        out.write_all(col.to_string().as_bytes())?;
        out.write_all(b"C")?;
    }
    if !fmt.is_empty() {
        out.write_all(fmt)?;
    }
    out.write_all(code)?;
    out.write_all(glyph)?;
    out.write_all(b"\x1b[0m")?;
    // Park back at column 0 of the last row.
    out.write_all(b"\r")?;
    if up > 0 {
        out.write_all(b"\x1b[")?;
        out.write_all(up.to_string().as_bytes())?;
        out.write_all(b"B")?;
    }
    Ok(())
}

/// Born-time of cell `(r, c)`: `tau = r + c·C`, rounded down to a step
/// index (bands are integer reveal steps).
fn tau_of(r: usize, c: usize, cost: f64) -> usize {
    (r as f64 + c as f64 * cost).floor() as usize
}

/// The main entry point. Buffers the whole input, then animates it in
/// screen-sized chunks (so arbitrarily tall input, e.g. an `apt list`,
/// keeps animating chunk after chunk). If stdout is not a terminal or any
/// single line is as wide as the terminal (the block geometry cannot
/// work), no animation runs at all: the input is painted plainly instead.
pub fn animate<R: BufRead + ?Sized>(
    fd: &mut R,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    eng.os = opts.os;
    if !eng.paint_init {
        set_mode(eng, opts.truecolor);
    }
    let mut raw = Vec::new();
    fd.read_to_end(&mut raw)?;
    if raw.is_empty() {
        return Ok(());
    }
    let (rows, cols) = match terminal_size() {
        Some(sz) => sz,
        None => {
            let mut input: &[u8] = &raw;
            return paint_stream(&mut input, opts, eng, out);
        }
    };
    // Parse the input into text rows. ANSI escapes are dropped from the
    // visible characters, but each SGR prefix is remembered per cell so
    // formatting from coloured input (fastfetch's background palette blocks)
    // survives underneath the reveal's own foreground colour. The plain
    // (non-animated) fallback keeps the raw bytes: `paint_stream` passes
    // ANSI through correctly.
    //
    // CRLF endings are folded first, and any remaining control byte that
    // the cell model cannot render (BEL, BS, VT/FF, an interior CR, NUL, …)
    // degrades the whole input to plain colouring — an animation that
    // flattened such a row would fabricate text that was never on screen.
    let lines = match split_lines(&raw) {
        Some(lines) => lines,
        None => {
            let mut input: &[u8] = &raw;
            return paint_stream(&mut input, opts, eng, out);
        }
    };
    let mut all: Vec<Row> = lines
        .iter()
        .map(|line| parse_line(line, opts.keep, opts.tab))
        .collect();
    if all.last().map(|l| l.chars.is_empty()) == Some(true) {
        all.pop(); // trailing newline
    }
    // Blank-only input (just newlines): there is nothing to reveal, and an
    // empty reveal would not even print the blank lines. Paint it plainly,
    // exactly like a non-animated run.
    if all.iter().all(|l| l.chars.is_empty()) {
        let mut input: &[u8] = &raw;
        return paint_stream(&mut input, opts, eng, out);
    }
    // A line at least as wide as the terminal would wrap: no geometry.
    // Display width, not cell count: a row of 40 CJK glyphs is 80 columns.
    let too_wide = all.iter().any(|l| l.width() >= cols);
    if too_wide {
        let mut input: &[u8] = &raw;
        return paint_stream(&mut input, opts, eng, out);
    }
    // Text that fits the window is revealed in place, exactly as before.
    // Taller text is scrolled through a fixed front instead of being split
    // into screen-sized chunks: the `"bottom"` key decides where the front
    // meets the window's bottom row.
    let window_h = rows.saturating_sub(1).max(1);
    if all.len() <= window_h {
        animate_block(&all, opts, eng, out)?;
    } else {
        animate_scroll(&all, window_h, opts.bottom.column(cols), opts, eng, out)?;
    }
    // The reveal parks the cursor at column 0 of the last row. When the
    // input ended with a newline, move to a fresh line below it so the next
    // output (shell prompt, …) does not overwrite the final row. Without a
    // trailing newline, mirror plain lolcat, which leaves the cursor just
    // past the final character: move right by the last row's cell count.
    if raw.ends_with(b"\n") {
        out.write_all(b"\n")?;
    } else if let Some(last) = all.last() {
        let w = last.width();
        if w > 0 {
            out.write_all(b"\x1b[")?;
            out.write_all(w.to_string().as_bytes())?;
            out.write_all(b"C")?;
        }
    }
    out.flush()?;
    Ok(())
}

/// In-place diagonal reveal over a block small enough to stay on screen.
fn animate_block(
    rows: &[Row],
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    // Pre-print a blank block (spaces) so the reveal can overwrite in place.
    out.write_all(b"\x1b[?25l")?;
    let height = rows.len();
    for (i, row) in rows.iter().enumerate() {
        for _ in 0..row.width() {
            out.write_all(b" ")?;
        }
        if i + 1 < height {
            out.write_all(b"\n")?;
        }
    }
    // The cursor is now at the end of the last (blank) row; park it at
    // column 0 — every cell write below assumes it starts from there, and
    // without this the very first write would land a stray glyph a whole
    // row-width to the right (the "ghost character").
    out.write_all(b"\r")?;
    out.flush()?;

    let cost = char_cost(opts.angle);
    let (dx, dy) = opts.phase_step();
    let (max_c, max_r) = (rows.iter().map(|r| r.width()).max().unwrap_or(0), height);
    let max_tau = if max_c == 0 {
        0
    } else {
        (max_r as f64 + max_c as f64 * cost).floor() as usize
    };

    // Stable per-cell colour (stream style: hue = os + row·dy + col·dx).
    let stable_rgb =
        |r: usize, col: usize| color_for(eng.os + r as f64 * dy + col as f64 * dx, opts.pure);

    let write_final = |out: &mut dyn Write, r: usize, c: usize, col: usize| -> io::Result<()> {
        let mut cb = [0u8; 4];
        let mut glyph = rows[r].chars[c].encode_utf8(&mut cb).as_bytes().to_vec();
        // Combining marks / ZWJ / selectors belong to this cell's glyph.
        glyph.extend_from_slice(rows[r].extras[c].as_bytes());
        let mut code = [0u8; 24];
        let n = write_sgr(
            &mut code,
            eng.mode,
            opts.invert,
            stable_rgb(r, col),
            opts.quantizer,
        );
        put_cell(out, height, r, col, &rows[r].fmts[c], &code[..n], &glyph)?;
        Ok(())
    };

    // Reveal step loop: one band per `duration` frames.
    let (d, speed) = (opts.duration.max(1), opts.speed.max(0.1));
    let frame = Duration::from_secs_f64(1.0 / speed);

    // cell list per reveal step
    let mut by_step: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); max_tau + 1];
    for (r, row) in rows.iter().enumerate() {
        let mut col = 0usize;
        for c in 0..row.chars.len() {
            let t = tau_of(r, col, cost).min(max_tau);
            by_step[t].push((r, c, col));
            col += row.widths[c];
        }
    }

    for step in 0..=max_tau {
        // Freeze the previous band first (same frame the next is born).
        if step > 0 {
            for &(r, c, col) in &by_step[step - 1] {
                write_final(out, r, c, col)?;
            }
            if d == 1 {
                // `-d 1` has no intra-band dwell, so the step boundary
                // itself must advance at `-s` fps — otherwise the whole
                // reveal would run as fast as the terminal can write and
                // `-s` would have no effect.
                thread::sleep(frame);
            }
        }
        // Dwell: flicker the current band for `d` frames.
        let band = &by_step[step];
        for f in 0..d {
            if f > 0 {
                thread::sleep(frame);
            }
            for &(r, _, col) in band {
                let hue = random_hue();
                let rgb = color_for(hue, opts.pure);
                let code = {
                    let mut code = [0u8; 24];
                    let n = write_sgr(&mut code, eng.mode, opts.invert, rgb, opts.quantizer);
                    code[..n].to_vec()
                };
                put_cell(out, height, r, col, &[], &code, &[random_glyph()])?;
            }
            out.flush()?;
        }
    }
    // Freeze the last band.
    for &(r, c, col) in &by_step[max_tau] {
        write_final(out, r, c, col)?;
    }
    out.flush()?;
    out.write_all(b"\x1b[?25h")?;
    out.flush()?;
    Ok(())
}

/// Reveal text that is taller than the window by scrolling it through a
/// front that keeps the `-A` slope. The front sweeps in text coordinates as
/// usual; as soon as its intersection with the window's bottom row has
/// reached `bottom_col` — and the row about to leave the top is fully
/// revealed — the text scrolls up one row and the next line enters at the
/// bottom. Nothing is ever scrolled off before it has been revealed, so the
/// whole text is shown exactly once, continuously (no per-screen restart).
fn animate_scroll(
    rows: &[Row],
    window_h: usize,
    bottom_col: usize,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    let h = window_h.max(1);
    let total = rows.len();
    let cost = char_cost(opts.angle);
    let tan = if cost <= f64::EPSILON {
        f64::INFINITY
    } else {
        1.0 / cost
    };
    let (dx, dy) = opts.phase_step();

    // Bands in text coordinates: tau(r, c) = r + c·cost, with the cell's
    // display column. `remaining[r]` counts row r's cells not yet frozen.
    let mut bands: Vec<Vec<(usize, usize, usize)>> = Vec::new();
    let mut remaining: Vec<usize> = rows.iter().map(|r| r.chars.len()).collect();
    for (r, row) in rows.iter().enumerate() {
        let mut col = 0usize;
        for c in 0..row.chars.len() {
            let t = tau_of(r, col, cost);
            if bands.len() <= t {
                bands.resize(t + 1, Vec::new());
            }
            bands[t].push((r, c, col));
            col += row.widths[c];
        }
    }
    let max_tau = bands.len().saturating_sub(1);

    // Blank window: one blank line per window row, parked at column 0.
    let blank_w = rows.iter().map(Row::width).max().unwrap_or(0);
    out.write_all(b"\x1b[?25l")?;
    for i in 0..h {
        for _ in 0..blank_w {
            out.write_all(b" ")?;
        }
        if i + 1 < h {
            out.write_all(b"\n")?;
        }
    }
    out.write_all(b"\r")?;
    out.flush()?;

    let stable_rgb =
        |r: usize, col: usize| color_for(eng.os + r as f64 * dy + col as f64 * dx, opts.pure);
    let write_final =
        |out: &mut dyn Write, r: usize, c: usize, col: usize, screen_r: usize| -> io::Result<()> {
            let mut cb = [0u8; 4];
            let mut glyph = rows[r].chars[c].encode_utf8(&mut cb).as_bytes().to_vec();
            glyph.extend_from_slice(rows[r].extras[c].as_bytes());
            let mut code = [0u8; 24];
            let n = write_sgr(
                &mut code,
                eng.mode,
                opts.invert,
                stable_rgb(r, col),
                opts.quantizer,
            );
            put_cell(out, h, screen_r, col, &rows[r].fmts[c], &code[..n], &glyph)
        };

    let d = opts.duration.max(1);
    let speed = opts.speed.max(0.1);
    let frame = Duration::from_secs_f64(1.0 / speed);

    let mut s = 0usize; // rows scrolled so far
    let mut pending: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); total];
    let mut revealed: Vec<Vec<bool>> = rows.iter().map(|r| vec![false; r.chars.len()]).collect();
    // Freeze a cell at most once (the tail passes below can overlap).
    macro_rules! freeze {
        ($r:expr, $c:expr, $col:expr, $sr:expr) => {{
            let (r, c, col, sr) = ($r, $c, $col, $sr);
            if !revealed[r][c] {
                revealed[r][c] = true;
                write_final(out, r, c, col, sr)?;
                remaining[r] = remaining[r].saturating_sub(1);
            }
        }};
    }

    // Bands are all revealed by `max_tau`, but scrolling the last rows into
    // the window costs one extra step per row, so keep stepping until every
    // row has entered.
    let mut t = 0usize;
    while t <= max_tau || s + h < total {
        if t > 0 {
            // Scroll at most one row per band, and only when it is safe: the
            // top row must be fully revealed. `bottom_col` is the earliest
            // point; if the whole window is already revealed we must scroll
            // anyway, otherwise waiting could never bring the tail into view.
            if s + h < total && remaining[s] == 0 {
                let inter = if cost <= f64::EPSILON {
                    f64::INFINITY
                } else {
                    (t as f64 - (s + h - 1) as f64) * tan
                };
                let window_done = (s..(s + h).min(total)).all(|r| remaining[r] == 0);
                if inter >= bottom_col as f64 || window_done {
                    out.write_all(b"\n")?;
                    s += 1;
                    // Anything that drained while its row was still below the
                    // window appears now, frozen.
                    let entered = s + h - 1;
                    if entered < total {
                        for &(r, c, col) in std::mem::take(&mut pending[entered]).iter() {
                            freeze!(r, c, col, entered - s);
                        }
                    }
                }
            }
            // Freeze the previous band. Cells below the window wait in
            // `pending` until their row scrolls in.
            if let Some(prev) = bands.get(t - 1) {
                for &(r, c, col) in prev {
                    if r < s {
                        // Cannot happen: a row is only scrolled past once frozen.
                        debug_assert!(false, "cell scrolled off before being revealed");
                    } else if r - s < h {
                        freeze!(r, c, col, r - s);
                    } else {
                        pending[r].push((r, c, col));
                    }
                }
            }
            if d == 1 {
                // Pace band boundaries at `-s` fps, as in the in-place path.
                thread::sleep(frame);
            }
        }
        // Dwell/flicker the current band (only the cells inside the window).
        for f in 0..d {
            if f > 0 {
                thread::sleep(frame);
            }
            for &(r, _, col) in bands.get(t).map(Vec::as_slice).unwrap_or(&[]) {
                if r < s || r - s >= h {
                    continue;
                }
                let hue = random_hue();
                let rgb = color_for(hue, opts.pure);
                let code = {
                    let mut code = [0u8; 24];
                    let n = write_sgr(&mut code, eng.mode, opts.invert, rgb, opts.quantizer);
                    code[..n].to_vec()
                };
                put_cell(out, h, r - s, col, &[], &code, &[random_glyph()])?;
            }
            out.flush()?;
        }
        t += 1;
    }
    // Freeze the last band, then any stragglers (defensive: with the scroll
    // rule above every row has entered the window by now).
    if let Some(last) = bands.get(max_tau) {
        for &(r, c, col) in last {
            if r >= s && r - s < h {
                freeze!(r, c, col, r - s);
            } else if r >= s {
                pending[r].push((r, c, col));
            }
        }
    }
    for row_pending in pending.iter() {
        for &(r, c, col) in row_pending {
            if r >= s && r - s < h {
                freeze!(r, c, col, r - s);
            }
        }
    }
    debug_assert!(remaining.iter().all(|&n| n == 0), "unrevealed cells left");
    out.flush()?;
    out.write_all(b"\x1b[?25h")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_sector() {
        assert!(angle_supported(45.0));
        assert!(angle_supported(71.6));
        assert!(angle_supported(90.0));
        assert!(angle_supported(198.4));
        assert!(angle_supported(225.0));
        assert!(angle_supported(270.0));
        assert!(angle_supported(-315.0)); // ≡ 45
        assert!(angle_supported(-90.0)); // ≡ 270
        assert!(!angle_supported(0.0));
        assert!(!angle_supported(10.0));
        assert!(!angle_supported(100.0));
        assert!(!angle_supported(180.0));
        assert!(!angle_supported(270.1));
        assert!(!angle_supported(-45.0)); // ≡ 315
    }

    #[test]
    fn char_cost_anchors() {
        // cot 45° = 1, cot 90° → 0.
        assert!((char_cost(45.0) - 1.0).abs() < 1e-6);
        assert!(char_cost(90.0) < 1e-9);
        assert!(char_cost(270.0) < 1e-9);
        assert!((char_cost(225.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tau_single_line_45() {
        // One row: tau = column.
        for c in 0..10 {
            assert_eq!(tau_of(0, c, 1.0), c);
        }
    }

    #[test]
    fn tau_diagonal_45() {
        // r + c: cells on one anti-diagonal share a step.
        assert_eq!(tau_of(1, 2, 1.0), 3);
        assert_eq!(tau_of(2, 1, 1.0), 3);
        assert_eq!(tau_of(0, 3, 1.0), 3);
    }

    #[test]
    fn tau_row_whole_at_90() {
        // cost 0: every column of row r is born together at tau = r.
        assert_eq!(tau_of(2, 0, 0.0), 2);
        assert_eq!(tau_of(2, 99, 0.0), 2);
    }

    #[test]
    fn parse_line_keeps_bg_fmt() {
        // fastfetch-style palette line: SGR background blocks survive as
        // per-cell prefixes, and the escapes themselves are not text.
        let row = parse_line(b"\x1b[40mAB\x1b[41mC\x1b[42mDEF", true, TabWidth::Spaces(8));
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "ABCDEF");
        assert_eq!(row.fmts[0], b"\x1b[40m".to_vec());
        assert_eq!(row.fmts[1], b"\x1b[40m".to_vec());
        assert_eq!(row.fmts[2], b"\x1b[41m".to_vec()); // C
        assert_eq!(row.fmts[3], b"\x1b[42m".to_vec()); // D
        assert_eq!(row.fmts[5], b"\x1b[42m".to_vec()); // F
        assert_eq!(row.chars.len(), row.fmts.len());
    }

    #[test]
    fn parse_line_abs_column_padding() {
        // fastfetch pads with ESC[47G; the value must land after 46 cells.
        let row = parse_line(b"CPU: \x1b[47GApple", true, TabWidth::Spaces(8));
        // 5 lead + 41 pad (target col 47 is 1-based → index 46) + 5 value
        assert_eq!(row.chars.len(), 51);
        assert_eq!(row.chars[0..5].iter().collect::<String>(), "CPU: ");
        assert_eq!(row.chars[46], 'A');
        assert!(row.chars[5..46].iter().all(|&c| c == ' '));
    }

    #[test]
    fn parse_line_no_keep_drops_fmt() {
        let row = parse_line(b"\x1b[40mAB\x1b[41mCD", false, TabWidth::Spaces(8));
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "ABCD");
        assert!(row.fmts.iter().all(|f| f.is_empty()));
    }

    /// Count frozen writes of `glyph` for a row whose input SGR prefix is
    /// `fmt`: flicker frames carry no prefix, so a match means the cell was
    /// really revealed (frozen) at least once.
    fn frozen_hits(out: &[u8], fmt: &[u8], glyph: u8) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i + fmt.len() <= out.len() {
            if &out[i..i + fmt.len()] == fmt {
                let mut j = i + fmt.len();
                if j + 1 < out.len() && out[j] == 0x1b && out[j + 1] == b'[' {
                    j += 2;
                    while j < out.len() && !out[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    if j < out.len() {
                        j += 1;
                    }
                }
                if j < out.len() && out[j] == glyph {
                    n += 1;
                }
            }
            i += 1;
        }
        n
    }

    #[test]
    fn animate_scroll_reveals_every_row_and_keeps_the_window() {
        // 12 rows of 3 cells, window height 6 → the text must scroll.
        let total = 12usize;
        let h = 6usize;
        let text: String = (0..total)
            .map(|i| {
                format!(
                    "\x1b[4{i}m{}{}{}\n",
                    (b'a' + i as u8) as char,
                    (b'a' + i as u8) as char,
                    (b'a' + i as u8) as char
                )
            })
            .collect();
        let lines = split_lines(text.as_bytes()).unwrap();
        let mut opts = Options::defaults();
        opts.angle = 60.0;
        opts.duration = 1;
        opts.speed = 1000.0;
        let rows: Vec<Row> = lines
            .iter()
            .map(|l| parse_line(l, true, opts.tab))
            .collect();
        let rows = &rows[..rows.len() - 1]; // drop the trailing empty row
        let mut eng = Engine::new();
        let mut out = Vec::new();
        animate_scroll(rows, h, 4, &opts, &mut eng, &mut out).unwrap();

        // Every row was revealed (3 frozen cells each): nothing lost.
        for i in 0..total {
            let fmt = format!("\x1b[4{i}m").into_bytes();
            let glyph = b'a' + i as u8;
            assert_eq!(
                frozen_hits(&out, &fmt, glyph),
                3,
                "row {i} not fully revealed"
            );
        }
        // One scroll per row beyond the window height.
        let newlines = out.iter().filter(|&&b| b == b'\n').count();
        assert_eq!(newlines, (h - 1) + (total - h));
        // The cursor never moves up beyond the window.
        let mut i = 0;
        while i + 2 < out.len() {
            if out[i] == 0x1b && out[i + 1] == b'[' {
                let mut j = i + 2;
                let mut n = 0usize;
                while j < out.len() && out[j].is_ascii_digit() {
                    n = n * 10 + (out[j] - b'0') as usize;
                    j += 1;
                }
                if j < out.len() && out[j] == b'A' {
                    assert!(n < h, "up move {n} leaves the window");
                }
                i = j;
            } else {
                i += 1;
            }
        }
    }

    #[test]
    fn animate_scroll_still_finishes_when_bottom_is_unreachable() {
        // A `bottom` column far beyond the front's reach must not stall the
        // animation: once the window is fully revealed we scroll anyway.
        let total = 10usize;
        let h = 4usize;
        let text: String = (0..total)
            .map(|i| {
                format!(
                    "\x1b[4{i}m{}{}\n",
                    (b'a' + i as u8) as char,
                    (b'a' + i as u8) as char
                )
            })
            .collect();
        let lines = split_lines(text.as_bytes()).unwrap();
        let mut opts = Options::defaults();
        opts.angle = 60.0;
        opts.duration = 1;
        opts.speed = 1000.0;
        let rows: Vec<Row> = lines
            .iter()
            .map(|l| parse_line(l, true, opts.tab))
            .collect();
        let rows = &rows[..rows.len() - 1];
        let mut eng = Engine::new();
        let mut out = Vec::new();
        animate_scroll(rows, h, 100_000, &opts, &mut eng, &mut out).unwrap();
        for i in 0..total {
            let fmt = format!("\x1b[4{i}m").into_bytes();
            let glyph = b'a' + i as u8;
            assert_eq!(frozen_hits(&out, &fmt, glyph), 2, "row {i} lost");
        }
    }

    #[test]
    fn animate_scroll_keeps_short_texts_in_place() {
        // The in-place path is chosen for anything that fits the window, so
        // `animate_scroll` is only ever called with more rows than `h`.
        let rows = vec![Row::empty(); 7];
        let mut opts = Options::defaults();
        opts.angle = 45.0;
        opts.duration = 1;
        opts.speed = 1000.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        // 7 empty rows in a 3-row window: every row is blank, so the window
        // empties immediately and the walk must still terminate.
        animate_scroll(&rows, 3, 2, &opts, &mut eng, &mut out).unwrap();
        assert!(out.ends_with(b"\x1b[?25h"));
    }

    #[test]
    fn animate_block_places_wide_cells_at_display_columns() {
        // angle 90 → cost 0, so the whole row is one band and every cell is
        // frozen in one pass. The running display column must be used for
        // positioning (你 at 0, 'a' at 2) and for the blank pre-print.
        let mut opts = Options::defaults();
        opts.angle = 90.0;
        opts.duration = 1;
        opts.speed = 1000.0;
        opts.tab = TabWidth::Spaces(8);
        let rows: Vec<Row> =
            std::iter::once(parse_line("你a".as_bytes(), true, opts.tab)).collect();
        let mut eng = Engine::new();
        let mut out = Vec::new();
        animate_block(&rows, &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains('你'), "wide glyph missing: {s:?}");
        assert!(s.contains("\x1b[2C"), "expected a two-column move: {s:?}");
        // Blank block pre-prints three columns (2 for 你 + 1 for a).
        assert!(s.matches(' ').count() >= 3, "blanks: {s:?}");

        // A combining mark is written inside its base cell, not positioned
        // separately.
        let rows: Vec<Row> =
            std::iter::once(parse_line("A\u{301}".as_bytes(), true, opts.tab)).collect();
        let mut out = Vec::new();
        animate_block(&rows, &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("A\u{301}"), "combined glyph missing: {s:?}");
    }

    #[test]
    fn parse_line_tracks_display_width() {
        // CJK: one cell, two columns.
        let row = parse_line("你ab".as_bytes(), true, TabWidth::Spaces(8));
        assert_eq!(row.chars, vec!['你', 'a', 'b']);
        assert_eq!(row.widths, vec![2, 1, 1]);
        assert_eq!(row.width(), 4);

        // Zero-width characters glue to the base cell instead of becoming
        // cells of their own (combining acute, VS16).
        let row = parse_line("A\u{301}\u{fe0f}x".as_bytes(), true, TabWidth::Spaces(8));
        assert_eq!(row.chars, vec!['A', 'x']);
        assert_eq!(row.extras[0], "\u{301}\u{fe0f}");
        assert_eq!(row.width(), 2);

        // Tab stops count display columns, not cells: 你 occupies 0-1, so a
        // tab must add 6 spaces to reach column 8.
        let row = parse_line("你\tz".as_bytes(), true, TabWidth::Stops);
        assert_eq!(row.width(), 9);
        assert_eq!(row.chars.last(), Some(&'z'));
        assert_eq!(row.chars.len(), 8); // 你 + 6 spaces + z

        // Absolute column moves (`ESC[10G`) also target display columns.
        let row = parse_line("你\x1b[10Gx".as_bytes(), true, TabWidth::Spaces(8));
        assert_eq!(row.width(), 10);
        assert_eq!(row.chars.last(), Some(&'x'));
        assert_eq!(row.widths[0], 2);
    }

    #[test]
    fn parse_line_expands_tabs() {
        // Fixed width: N spaces wherever the tab appears.
        let row = parse_line(b"a\tb", true, TabWidth::Spaces(4));
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "a    b");
        assert_eq!(row.chars.len(), row.fmts.len());
        // Tab stops: pad from the current column to the next multiple of 8.
        let row = parse_line(b"abc\tz", true, TabWidth::Stops);
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "abc     z"); // 3 + 5 pad = column 8
                                    // The default stays 8 fixed spaces (historical layout).
        let row = parse_line(b"a\tb", true, TabWidth::default());
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "a        b");
    }

    #[test]
    fn parse_line_truncated_escape_tail_dropped() {
        // A truncated escape (no final byte before the row end) is
        // discarded by terminals; it must not leak out as "[31" text.
        for tail in [&b"\x1b[31"[..], &b"\x1b["[..], &b"\x1b"[..]] {
            let mut line = b"ab".to_vec();
            line.extend_from_slice(tail);
            let row = parse_line(&line, true, TabWidth::Spaces(8));
            let s: String = row.chars.iter().collect();
            assert_eq!(s, "ab", "tail {:?}", tail);
        }
        // An incomplete sequence followed by a later complete one: parsing
        // resumes at the next ESC, which is handled normally.
        let row = parse_line(b"ab\x1b[31\x1b[32mX", true, TabWidth::Spaces(8));
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "abX");
        assert_eq!(row.fmts.last().unwrap(), b"\x1b[32m");
    }

    #[test]
    fn split_lines_folds_crlf() {
        // CRLF endings collapse to the content row (no phantom CR column).
        // A trailing newline still leaves a final empty row for the caller.
        assert_eq!(
            split_lines(b"a\r\nb\r\n").unwrap(),
            vec![b"a".as_slice(), b"b".as_slice(), b"".as_slice()]
        );
        // A lone trailing CR (no newline) is only a no-op column reset.
        assert_eq!(split_lines(b"ab\r").unwrap(), vec![b"ab".as_slice()]);
        // Trailing newline leaves a final empty row for the caller to pop.
        assert_eq!(
            split_lines(b"x\n").unwrap(),
            vec![b"x".as_slice(), b"".as_slice()]
        );
    }

    #[test]
    fn split_lines_rejects_interior_controls() {
        // Interior CR means overwrite semantics: unrepresentable.
        assert!(split_lines(b"a\rb\n").is_none());
        // BEL/BS/VT/FF/NUL and the rest of C0 degrade the input…
        for c in [0x00u8, 0x07, 0x08, 0x0b, 0x0c, 0x0e, 0x1f] {
            assert!(
                split_lines(&[b'a', c, b'b', b'\n']).is_none(),
                "C0 {:02x} not rejected",
                c
            );
        }
        // …while tabs and escape sequences are fine.
        assert_eq!(
            split_lines(b"a\tb\n").unwrap(),
            vec![b"a\tb".as_slice(), b"".as_slice()]
        );
        assert_eq!(
            split_lines(b"\x1b[40mAB\x1b(Bc\n").unwrap(),
            vec![b"\x1b[40mAB\x1b(Bc".as_slice(), b"".as_slice()]
        );
    }
}
