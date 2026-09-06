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

use crate::ansi::escape_len;
use crate::color::{color_for, write_sgr};
use crate::engine::{set_mode, Engine};
use crate::options::Options;
use crate::stream::paint_stream;

/// Ask the terminal for its size (rows, columns). The block path may only
/// run when the whole text fits: no more lines than the height and every
/// line strictly narrower than the width. When the size cannot be queried
/// (stdout is not a terminal) we take the per-line path, which needs no
/// geometry.
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
            Some((w.rows as usize, w.cols as usize))
        } else {
            None
        }
    }
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

/// One buffered text row: its characters (visual cells) plus, for each
/// cell, the ANSI SGR prefix (background colours, styles, …) that must be
/// emitted before it so formatting from coloured input (e.g. fastfetch's
/// palette blocks, which are background-coloured spaces) survives.
#[derive(Clone)]
struct Row {
    /// The final characters (empty for a blank line).
    chars: Vec<char>,
    /// Per-cell input SGR prefix bytes (may be empty).
    fmts: Vec<Vec<u8>>,
}

impl Row {
    fn empty() -> Row {
        Row {
            chars: Vec::new(),
            fmts: Vec::new(),
        }
    }
}

/// Parse one raw input line into a `Row`: escape sequences are dropped, but
/// each SGR (`ESC [ … m`) is remembered and attached to every following
/// visible character as its formatting prefix (attributes accumulate until
/// a new SGR code replaces them, which mirrors terminal behaviour closely
/// enough for re-colouring). With `keep` disabled no prefixes are recorded,
/// so the reveal paints plain rainbow text over any input styling.
fn parse_line(line: &[u8], keep: bool) -> Row {
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
                    // Only SGR sequences (…m) style the following cells.
                    if keep && seq.len() >= 2 && seq[seq.len() - 1] == b'm' {
                        fmt = seq.to_vec();
                    }
                    i += l;
                }
                None => i += 1,
            }
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
                row.chars.push(c);
                row.fmts.push(fmt.clone());
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
    c: usize,
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
    if c > 0 {
        out.write_all(b"\x1b[")?;
        out.write_all(c.to_string().as_bytes())?;
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
    let mut all: Vec<Row> = raw
        .split(|&b| b == b'\n')
        .map(|line| parse_line(line, opts.keep))
        .collect();
    if all.last().map(|l| l.chars.is_empty()) == Some(true) {
        all.pop(); // trailing newline
    }
    if all.is_empty() {
        return Ok(());
    }
    // A line at least as wide as the terminal would wrap: no geometry.
    let too_wide = all.iter().any(|l| l.chars.len() >= cols);
    if too_wide {
        let mut input: &[u8] = &raw;
        return paint_stream(&mut input, opts, eng, out);
    }
    // Chunk the input by screen height and reveal each chunk in place.
    let chunk_h = rows.saturating_sub(1).max(1);
    for (i, chunk) in all.chunks(chunk_h).enumerate() {
        if i > 0 {
            out.write_all(b"\n")?;
        }
        animate_block(chunk, opts, eng, out)?;
    }
    // The reveal parks the cursor at column 0 of the last row; when the
    // input ended with a newline, move to a fresh line below it so the next
    // output (shell prompt, …) does not overwrite the final row.
    if raw.ends_with(b"\n") {
        out.write_all(b"\n")?;
        out.flush()?;
    }
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
        for _ in 0..row.chars.len() {
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
    let (max_c, max_r) = (
        rows.iter().map(|r| r.chars.len()).max().unwrap_or(0),
        height,
    );
    let max_tau = if max_c == 0 {
        0
    } else {
        (max_r as f64 + max_c as f64 * cost).floor() as usize
    };

    // Stable per-cell colour (stream style: hue = os + row·dy + col·dx).
    let stable_rgb =
        |r: usize, c: usize| color_for(eng.os + r as f64 * dy + c as f64 * dx, opts.pure);

    let write_final = |out: &mut dyn Write, r: usize, c: usize| -> io::Result<()> {
        let mut cb = [0u8; 4];
        let enc = rows[r].chars[c].encode_utf8(&mut cb).as_bytes().to_vec();
        let mut code = [0u8; 24];
        let n = write_sgr(&mut code, eng.mode, opts.invert, stable_rgb(r, c));
        put_cell(out, height, r, c, &rows[r].fmts[c], &code[..n], &enc)?;
        Ok(())
    };

    // Reveal step loop: one band per `duration` frames.
    let (d, speed) = (opts.duration.max(1), opts.speed.max(0.1));
    let frame = Duration::from_secs_f64(1.0 / speed);

    // cell list per reveal step
    let mut by_step: Vec<Vec<(usize, usize)>> = vec![Vec::new(); max_tau + 1];
    for (r, row) in rows.iter().enumerate() {
        for (c, _) in row.chars.iter().enumerate() {
            let t = tau_of(r, c, cost).min(max_tau);
            by_step[t].push((r, c));
        }
    }

    for step in 0..=max_tau {
        // Freeze the previous band first (same frame the next is born).
        if step > 0 {
            for &(r, c) in &by_step[step - 1] {
                write_final(out, r, c)?;
            }
        }
        // Dwell: flicker the current band for `d` frames.
        let band = &by_step[step];
        for f in 0..d {
            if f > 0 {
                thread::sleep(frame);
            }
            for &(r, c) in band {
                let hue = random_hue();
                let rgb = color_for(hue, opts.pure);
                let code = {
                    let mut code = [0u8; 24];
                    let n = write_sgr(&mut code, eng.mode, opts.invert, rgb);
                    code[..n].to_vec()
                };
                put_cell(out, height, r, c, &[], &code, &[random_glyph()])?;
            }
            out.flush()?;
        }
    }
    // Freeze the last band.
    for &(r, c) in &by_step[max_tau] {
        write_final(out, r, c)?;
    }
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
        let row = parse_line(b"\x1b[40mAB\x1b[41mC\x1b[42mDEF", true);
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
    fn parse_line_no_keep_drops_fmt() {
        let row = parse_line(b"\x1b[40mAB\x1b[41mCD", false);
        let s: String = row.chars.iter().collect();
        assert_eq!(s, "ABCD");
        assert!(row.fmts.iter().all(|f| f.is_empty()));
    }
}
