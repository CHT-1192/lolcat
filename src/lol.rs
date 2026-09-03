// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Rainbow coloring engine — Rust port of lol.rb with an angle/cycle model.
//!
//! Streams input in 4096-byte blocks and colors it as it arrives, like the
//! Ruby original, so producers that never emit newlines (e.g.
//! `cmatrix | lolcat`) still get live output. ANSI escapes and UTF-8
//! characters split across read boundaries are carried over intact, and the
//! hue phase advances per newline exactly as the earlier line-based port
//! did. Uses standard xterm-256 nearest-color mapping (not the Paint gem
//! grayscale heuristic), with no partial-line phase tricks.
//!
//! Two palettes share the same hue model (`hue(x, y)` completes one
//! revolution per `freq` grid units):
//! - classic (default): the original lolcat sine mapping — pastel colors;
//! - pure (`-P`): a saturated HSV hue wheel, hue 0° = pure red.

use std::io::{self, BufRead, Read, Write};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    Truecolor,
    Pal256,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub freq: f64,
    pub seed: i64,
    pub os: f64,
    pub angle: f64,
    pub pure: bool,
    pub anchor: bool,
    pub animate: bool,
    pub duration: u64,
    pub speed: f64,
    pub invert: bool,
    pub truecolor: bool,
    pub force: bool,
}

impl Options {
    pub fn defaults() -> Options {
        Options {
            freq: 60.0,
            seed: 0,
            os: 0.0,
            angle: 71.6,
            pure: false,
            anchor: false,
            animate: false,
            duration: 12,
            speed: 20.0,
            invert: false,
            truecolor: false,
            force: false,
        }
    }

    /// Per-character (dx) and per-line (dy) hue increments, in degrees.
    ///
    /// The angle is the stripe direction: 0° = up (vertical stripes),
    /// clockwise positive (90° = right = horizontal stripes), measured in
    /// the character grid.  The hue completes one full revolution per
    /// `freq` grid units along that direction:
    ///
    ///     hue(x, y) = os + 360·(x·cosθ + y·sinθ) / freq
    ///
    /// `os` is the seed hue offset in degrees.
    pub fn phase_step(&self) -> (f64, f64) {
        let a = self.angle.rem_euclid(360.0).to_radians();
        let step = 360.0 / self.freq;
        (a.cos() * step, a.sin() * step)
    }
}

pub struct Engine {
    pub os: f64,
    paint_init: bool,
    pub mode: ColorMode,
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            os: 0.0,
            paint_init: false,
            mode: ColorMode::Pal256,
        }
    }
}

/// True hue wheel: hue 0° = pure red (ff0000), one full revolution around
/// the HSV circle (yellow → green → cyan → blue → magenta → red).
pub fn hue_to_rgb(hue: f64) -> [u8; 3] {
    let h = hue.rem_euclid(360.0);
    let h6 = h / 60.0;
    let sector = h6 as i32; // 0..=5
    let f = h6 - sector as f64;
    let (r, g, b) = match sector {
        0 => (1.0, f, 0.0),
        1 => (1.0 - f, 1.0, 0.0),
        2 => (0.0, 1.0, f),
        3 => (0.0, 1.0 - f, 1.0),
        4 => (f, 0.0, 1.0),
        _ => (1.0, 0.0, 1.0 - f),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

/// Map a hue (degrees) to RGB: the original lolcat sine mapping (pastel)
/// unless `pure` requests the saturated hue wheel.
pub fn color_for(hue: f64, pure: bool) -> [u8; 3] {
    if pure {
        return hue_to_rgb(hue);
    }
    let h = hue.rem_euclid(360.0).to_radians();
    let r = h.sin() * 127.0 + 128.0;
    let g = (h + 2.0 * std::f64::consts::PI / 3.0).sin() * 127.0 + 128.0;
    let b = (h + 4.0 * std::f64::consts::PI / 3.0).sin() * 127.0 + 128.0;
    [r as u8, g as u8, b as u8]
}

// ── Standard xterm-256 palette (nearest-neighbor) ──────────────────────

const XTERM256_PALETTE: [[u8; 3]; 256] = {
    let mut pal = [[0u8; 3]; 256];
    // 0-15: ANSI system colors
    let sys = [
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    let cube_levels = [0, 95, 135, 175, 215, 255];
    let mut i = 0;
    while i < 16 {
        pal[i] = sys[i];
        i += 1;
    }
    // 16-231: 6×6×6 cube
    let mut r = 0;
    while r < 6 {
        let mut g = 0;
        while g < 6 {
            let mut b = 0;
            while b < 6 {
                pal[16 + r * 36 + g * 6 + b] = [cube_levels[r], cube_levels[g], cube_levels[b]];
                b += 1;
            }
            g += 1;
        }
        r += 1;
    }
    // 232-255: grayscale
    let mut k = 0;
    while k < 24 {
        let v = (8 + 10 * k) as u8;
        pal[232 + k] = [v, v, v];
        k += 1;
    }
    pal
};

/// Level values of the 6×6×6 colour cube.
const CUBE_LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];

/// Standard xterm-256 nearest-neighbor (not the Paint gem's grayscale
/// heuristic). Ties go to the lower index.
///
/// Instead of scanning all 256 palette entries per colour, only the
/// genuinely competitive entries are measured: the 16 system colours, the
/// cube entries built from each channel's nearest level(s) (cube levels are
/// ≥ 40 apart, so the optimum per channel is always one of the two
/// neighbouring levels), and the grey levels nearest to the mean channel
/// value. `rgb_to_256_matches_brute` verifies this equals the naive full
/// scan for every one of the 2^24 possible inputs.
pub fn rgb_to_256(red: u8, green: u8, blue: u8) -> u8 {
    let (r, g, b) = (red as i32, green as i32, blue as i32);
    let mut best_d = i32::MAX;
    let mut best_i = 0u8;
    macro_rules! consider {
        ($i:expr, $pr:expr, $pg:expr, $pb:expr) => {{
            let dr = r - $pr;
            let dg = g - $pg;
            let db = b - $pb;
            let d = dr * dr + dg * dg + db * db;
            let i = $i;
            if d < best_d || (d == best_d && i < best_i) {
                best_d = d;
                best_i = i;
            }
        }};
    }
    // System colours 0..=15 (also covers exact blacks/whites/brights).
    for (i, p) in XTERM256_PALETTE.iter().enumerate().take(16) {
        consider!(i as u8, p[0] as i32, p[1] as i32, p[2] as i32);
    }
    // Colour cube 16..=231: nearest level index per channel plus ±1
    // neighbours (ties live at the midpoints between adjacent levels).
    let span = |c: i32| -> [i32; 3] {
        let mut lo = 0usize;
        while lo + 1 < 6 && CUBE_LEVELS[lo + 1] <= c {
            lo += 1;
        }
        let lo = lo as i32;
        [(lo - 1).max(0), lo, (lo + 1).min(5)]
    };
    let (lr, lg, lb) = (span(r), span(g), span(b));
    for &ri in &lr {
        for &gi in &lg {
            for &bi in &lb {
                let idx = (16 + ri * 36 + gi * 6 + bi) as u8;
                consider!(
                    idx,
                    CUBE_LEVELS[ri as usize],
                    CUBE_LEVELS[gi as usize],
                    CUBE_LEVELS[bi as usize]
                );
            }
        }
    }
    // Greys 232..=255: v = 8 + 10k; the best k sits near the mean channel.
    let m = (r + g + b) / 3;
    let k0 = ((m - 8 + 5) / 10).clamp(0, 23);
    for &k in &[(k0 - 1).max(0), k0, (k0 + 1).min(23)] {
        let v = 8 + 10 * k;
        consider!((232 + k) as u8, v, v, v);
    }
    best_i
}

/// Append the decimal digits of `v` (≤ 65535) at `buf[pos..]`; returns the
/// new position.
fn push_u(buf: &mut [u8], mut pos: usize, mut v: u32) -> usize {
    let mut any = false;
    if v >= 100 {
        buf[pos] = b'0' + (v / 100) as u8;
        pos += 1;
        v %= 100;
        any = true;
    }
    if any || v >= 10 {
        buf[pos] = b'0' + (v / 10) as u8;
        pos += 1;
        v %= 10;
    }
    buf[pos] = b'0' + v as u8;
    pos + 1
}

/// Write a foreground/background SGR colour code into `buf` (no heap
/// allocation). Returns the number of bytes written.
pub fn write_sgr(buf: &mut [u8], mode: ColorMode, invert: bool, rgb: [u8; 3]) -> usize {
    let prefix: &[u8] = match (mode, invert) {
        (ColorMode::Truecolor, false) => b"\x1b[38;2;",
        (ColorMode::Truecolor, true) => b"\x1b[48;2;",
        (ColorMode::Pal256, false) => b"\x1b[38;5;",
        (ColorMode::Pal256, true) => b"\x1b[48;5;",
    };
    let mut n = prefix.len();
    buf[..n].copy_from_slice(prefix);
    match mode {
        ColorMode::Truecolor => {
            n = push_u(buf, n, rgb[0] as u32);
            buf[n] = b';';
            n += 1;
            n = push_u(buf, n, rgb[1] as u32);
            buf[n] = b';';
            n += 1;
            n = push_u(buf, n, rgb[2] as u32);
        }
        ColorMode::Pal256 => {
            let k = rgb_to_256(rgb[0], rgb[1], rgb[2]);
            n = push_u(buf, n, k as u32);
        }
    }
    buf[n] = b'm';
    n + 1
}

/// Truecolor if `--truecolor` or `COLORTERM ∈ {truecolor, 24bit}`.
pub fn set_mode(eng: &mut Engine, truecolor: bool) {
    let detected = match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => ColorMode::Truecolor,
        _ => ColorMode::Pal256,
    };
    eng.mode = if truecolor {
        ColorMode::Truecolor
    } else {
        detected
    };
    if std::env::var_os("LOLCAT_DEBUG").is_some() {
        let num = |m: ColorMode| {
            if m == ColorMode::Truecolor {
                16777215
            } else {
                256
            }
        };
        eprintln!(
            "DEBUG: Paint.mode = {} (detected: {})",
            num(eng.mode),
            num(detected)
        );
    }
    eng.paint_init = true;
}

// ── ANSI escape scanning ───────────────────────────────────────────────

/// Scan a string into `(escape_run, char)` pairs — equivalent to Ruby's
/// `str.scan(ANSI_ESCAPE)`.
pub fn scan_pairs(s: &str) -> Vec<(String, Option<char>)> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < n {
        let mut esc = String::new();
        while i < n && chars[i] == '\x1b' {
            match parse_escape(&chars, i) {
                Some((seq, next)) => {
                    esc.push_str(&seq);
                    i = next;
                }
                None => break,
            }
        }
        let ch = if i < n {
            let c = chars[i];
            i += 1;
            Some(c)
        } else {
            None
        };
        pairs.push((esc, ch));
    }
    pairs
}

fn parse_escape(chars: &[char], i: usize) -> Option<(String, usize)> {
    let n = chars.len();
    if i + 1 >= n {
        return None;
    }
    let c = chars[i + 1];
    let is_fmt = |ch: char| (0x20..=0x2f).contains(&(ch as u32));
    let is_csi = |ch: char| (0x30..=0x3f).contains(&(ch as u32));
    let is_marker = |ch: char| matches!(ch, ']' | 'P' | 'X' | '^' | '_');

    if is_fmt(c) {
        let mut j = i + 1;
        while j < n && is_fmt(chars[j]) {
            j += 1;
        }
        if j < n {
            Some((chars[i..=j].iter().collect(), j + 1))
        } else if j - (i + 1) >= 2 {
            Some((chars[i..j].iter().collect(), j))
        } else {
            Some((chars[i..=i + 1].iter().collect(), i + 2))
        }
    } else if is_marker(c) {
        let mut j = i + 2;
        while j < n && chars[j] != '\x07' && chars[j] != '\x1b' {
            j += 1;
        }
        Some((chars[i..j].iter().collect(), j))
    } else if c == '[' {
        let mut j = i + 2;
        while j < n && is_csi(chars[j]) {
            j += 1;
        }
        if j < n {
            Some((chars[i..=j].iter().collect(), j + 1))
        } else if j - (i + 2) >= 1 {
            Some((chars[i..j].iter().collect(), j))
        } else {
            Some((chars[i..=i + 1].iter().collect(), i + 2))
        }
    } else {
        Some((chars[i..=i + 1].iter().collect(), i + 2))
    }
}

// ── Output ─────────────────────────────────────────────────────────────

fn expand_tabs(s: &str) -> String {
    s.replace('\t', "        ")
}

fn strip_csi_ops(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if chars[i] == '\x1b' && i + 1 < n && chars[i + 1] == '[' {
            let mut j = i + 2;
            while j < n && (0x30..=0x3f).contains(&(chars[j] as u32)) {
                j += 1;
            }
            if j < n && matches!(chars[j], '@' | 'J' | 'K' | 'P' | 'X') {
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn println(
    line: &str,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    let chomped = line.ends_with('\n');
    let body = if chomped {
        &line[..line.len() - 1]
    } else {
        line
    };
    let mut body = expand_tabs(body);
    if opts.animate {
        println_ani(&mut body, opts, eng, out, chomped)?;
    } else {
        println_plain(&body, opts, eng, out)?;
    }
    if chomped {
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn println_plain(
    str: &str,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    if !eng.paint_init {
        set_mode(eng, opts.truecolor);
    }
    let (dx, _) = opts.phase_step();
    let filtered = scan_pairs(str);
    let reset = if opts.invert {
        b"\x1b[49m".as_slice()
    } else {
        b"\x1b[39m".as_slice()
    };
    for (i, (esc, ch)) in filtered.iter().enumerate() {
        out.write_all(esc.as_bytes())?;
        if let Some(c) = ch {
            if *c == ' ' && !opts.invert {
                // Foreground-coloured spaces are invisible: emit plainly.
                out.write_all(b" ")?;
                continue;
            }
            let rgb = color_for(eng.os + (i as f64) * dx, opts.pure);
            let mut buf = [0u8; 48];
            let mut n = write_sgr(&mut buf, eng.mode, opts.invert, rgb);
            let mut cb = [0u8; 4];
            let enc = c.encode_utf8(&mut cb).as_bytes();
            buf[n..n + enc.len()].copy_from_slice(enc);
            n += enc.len();
            buf[n..n + reset.len()].copy_from_slice(reset);
            n += reset.len();
            out.write_all(&buf[..n])?;
        }
    }
    Ok(())
}

fn println_ani(
    str: &mut String,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
    _chomped: bool,
) -> io::Result<()> {
    if str.is_empty() {
        return Ok(());
    }
    out.write_all(b"\x1b7")?;
    let real_os = eng.os;
    // Slide the hue by ~3 cycle-steps per frame; with the default freq=60
    // that is 18°/frame, close to the original lolcat spin (freq*spread =
    // 0.3 rad ≈ 17.2°/frame).
    let slide = 3.0 * 360.0 / opts.freq;
    for _ in 1..=opts.duration {
        out.write_all(b"\x1b8")?;
        eng.os += slide;
        println_plain(str, opts, eng, out)?;
        *str = strip_csi_ops(str);
        std::thread::sleep(std::time::Duration::from_secs_f64(1.0 / opts.speed));
    }
    eng.os = real_os;
    Ok(())
}

// ── Streaming output ────────────────────────────────────────────────────

/// Length in bytes of a complete ANSI escape sequence starting at `b[0]`
/// (which must be ESC), or `None` when the sequence is truncated and more
/// input is needed to finish it.
fn escape_len(b: &[u8]) -> Option<usize> {
    debug_assert_eq!(b[0], 0x1b);
    if b.len() < 2 {
        return None;
    }
    match b[1] {
        b'[' => {
            // CSI: ESC [ params/intermediates (0x20..=0x3f) + final (0x40..=0x7e)
            let mut j = 2;
            while j < b.len() && (0x20..=0x3f).contains(&b[j]) {
                j += 1;
            }
            if j < b.len() && (0x40..=0x7e).contains(&b[j]) {
                Some(j + 1)
            } else {
                None
            }
        }
        // OSC/DCS/PM/APC/...: terminated by BEL, or by ESC \ (ST).
        b']' | b'P' | b'X' | b'^' | b'_' => {
            let mut j = 2;
            while j < b.len() && b[j] != 0x07 && b[j] != 0x1b {
                j += 1;
            }
            if j >= b.len() {
                return None;
            }
            if b[j] == 0x07 {
                Some(j + 1)
            } else if j + 1 < b.len() && b[j + 1] == b'\\' {
                Some(j + 2) // ESC \ (string terminator)
            } else {
                Some(j + 1)
            }
        }
        // ESC <intermediate> <final>: charset designation `ESC ( B` (ASCII),
        // DEC graphics `ESC ( 0`, `ESC # 8`, `ESC % G`, `ESC " q`, etc.
        c if (0x20..=0x2f).contains(&c) => {
            if b.len() < 3 {
                return None; // need the final byte
            }
            if (0x30..=0x7e).contains(&b[2]) {
                Some(3)
            } else {
                None
            }
        }
        // Plain two-byte sequences (ESC X): e.g. \x1b7, \x1b=, \x1bM.
        _ => Some(2),
    }
}

/// Byte length of a UTF-8 character whose lead byte is `b`, or `None` when
/// `b` cannot start a character (a continuation or invalid byte).
fn utf8_char_len(b: u8) -> Option<usize> {
    match b {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

/// Paint one complete character: colour code, character bytes, reset —
/// composed into a single stack buffer (no per-character allocation).
/// With a foreground colour a space is invisible anyway, so in normal (non
/// inverted) mode spaces pass through uncoloured; the cursor/column phase
/// still advances.
fn emit_char(
    out: &mut dyn Write,
    eng: &mut Engine,
    opts: &Options,
    reset: &[u8],
    bytes: &[u8],
    hue: f64,
) -> io::Result<()> {
    if bytes == b" " && !opts.invert {
        return out.write_all(b" ");
    }
    let rgb = color_for(hue, opts.pure);
    let mut buf = [0u8; 64];
    let mut n = write_sgr(&mut buf, eng.mode, opts.invert, rgb);
    buf[n..n + bytes.len()].copy_from_slice(bytes);
    n += bytes.len();
    buf[n..n + reset.len()].copy_from_slice(reset);
    n += reset.len();
    out.write_all(&buf[..n])
}

/// Colorize a byte stream as it arrives, without waiting for newlines.
///
/// The hue phase advances by `dx` per character within a line and by `dy`
/// per newline, matching the previous line-based output byte for byte on
/// ordinary text (the empty-line bookkeeping is preserved too). ANSI
/// escapes pass through untouched, even when split across reads; a
/// truncated escape or UTF-8 character at the end of a read is buffered
/// until it completes. Invalid UTF-8 bytes pass through uncolored.
fn paint_stream<R: BufRead + ?Sized>(
    fd: &mut R,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    const CHUNK: usize = 4096;
    let (dx, dy) = opts.phase_step();
    let reset: &[u8] = if opts.invert { b"\x1b[49m" } else { b"\x1b[39m" };

    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK + 64);
    let mut buf = [0u8; CHUNK];
    let mut col: u64 = 0; // character index inside the current line
    let mut line_start = true; // next painted char begins a line: advance dy first
    loop {
        let n = fd.read(&mut buf)?;
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        let mut i = 0;
        let plen = pending.len();
        while i < plen {
            let b = pending[i];
            if b == 0x1b {
                match escape_len(&pending[i..]) {
                    Some(l) => {
                        out.write_all(&pending[i..i + l])?;
                        i += l;
                    }
                    None => break, // truncated escape: wait for more input
                }
            } else if b < 0x80 {
                match b {
                    b'\n' => {
                        if line_start {
                            // An empty line still consumed its own phase step.
                            eng.os += dy;
                        }
                        line_start = true;
                        col = 0;
                        out.write_all(b"\n")?;
                    }
                    b'\t' => {
                        // Tabs expand to eight spaces, each painted like a char.
                        for _ in 0..8 {
                            if line_start {
                                eng.os += dy;
                                line_start = false;
                            }
                            emit_char(
                                out,
                                eng,
                                opts,
                                reset,
                                b" ",
                                eng.os + (col as f64) * dx,
                            )?;
                            col += 1;
                        }
                    }
                    _ => {
                        if line_start {
                            eng.os += dy;
                            line_start = false;
                        }
                        emit_char(
                            out,
                            eng,
                            opts,
                            reset,
                            &pending[i..i + 1],
                            eng.os + (col as f64) * dx,
                        )?;
                        col += 1;
                    }
                }
                i += 1;
            } else {
                // Multi-byte UTF-8 character, or an invalid byte.
                match utf8_char_len(b) {
                    Some(l) if i + l <= plen => {
                        if pending[i + 1..i + l].iter().all(|&c| c & 0xc0 == 0x80) {
                            if line_start {
                                eng.os += dy;
                                line_start = false;
                            }
                            emit_char(
                                out,
                                eng,
                                opts,
                                reset,
                                &pending[i..i + l],
                                eng.os + (col as f64) * dx,
                            )?;
                            col += 1;
                            i += l;
                        } else {
                            // Invalid continuation bytes pass through raw.
                            out.write_all(&pending[i..i + 1])?;
                            i += 1;
                        }
                    }
                    Some(_) => break, // character split across reads: hold
                    None => {
                        out.write_all(&pending[i..i + 1])?;
                        i += 1;
                    }
                }
            }
        }
        if i > 0 {
            pending.drain(..i);
        }
        // Push live so interactive streams (cmatrix etc.) show up promptly.
        out.flush()?;
    }
    // EOF: emit anything left (a truncated escape / split character) raw.
    if !pending.is_empty() {
        out.write_all(&pending)?;
        pending.clear();
    }
    Ok(())
}

// ── Screen-anchored output (--anchor) ───────────────────────────────────

/// Parameter numbers of a CSI sequence (bytes between `ESC [` and the final
/// byte). Missing parameters become `None`; each opcode applies its own
/// default. Private markers (`?`, `>`, `<`, `=`) and intermediate bytes are
/// skipped.
fn csi_params(seq: &[u8]) -> Vec<Option<u64>> {
    let mut params = Vec::new();
    let mut cur: Option<u64> = None;
    for &b in &seq[2..seq.len().saturating_sub(1)] {
        match b {
            b'0'..=b'9' => {
                let d = (b - b'0') as u64;
                cur = Some(cur.unwrap_or(0) * 10 + d);
            }
            b';' => {
                params.push(cur);
                cur = None;
            }
            _ => {} // private/intermediate bytes
        }
    }
    params.push(cur);
    params
}

/// Apply the cursor-movement effect of a CSI sequence to `(row, col)`.
fn csi_move(seq: &[u8], pos: &mut (i64, i64), saved: &mut (i64, i64)) {
    if seq.len() < 3 || seq[1] != b'[' {
        return;
    }
    let params = csi_params(seq);
    let p = |i: usize| params.get(i).copied().flatten().unwrap_or(1) as i64;
    match seq[seq.len() - 1] {
        b'H' | b'f' => {
            let r = (p(0) - 1).max(0);
            let c = (p(1) - 1).max(0);
            pos.0 = r;
            pos.1 = c;
        }
        b'A' => pos.0 -= p(0).max(0),
        b'B' => pos.0 += p(0),
        b'C' => pos.1 += p(0),
        b'D' => pos.1 -= p(0).max(0),
        b'E' => {
            pos.0 += p(0);
            pos.1 = 0;
        }
        b'F' => {
            pos.0 -= p(0).max(0);
            pos.1 = 0;
        }
        b'G' => pos.1 = (p(0) - 1).max(0),
        b'd' => pos.0 = (p(0) - 1).max(0),
        b's' => *saved = *pos, // save cursor
        b'u' => *pos = *saved, // restore cursor
        b'h' | b'l' => {
            // Entering/leaving the alternate screen resets the coordinate
            // space, so jump back to the top-left corner.
            if p(0) == 1049 || p(0) == 47 {
                *pos = (0, 0);
            }
        }
        _ => {} // colour/clear/scroll/etc.: cursor unchanged
    }
}

/// Colorize a stream whose hue is anchored to screen coordinates.
///
/// Full-screen TUIs (btop, htop, ...) redraw only the cells that changed,
/// jumping around with `ESC [ y ; x H`. In stream order those rewrites land
/// at unpredictable offsets, so a stream-linear hue flickers. Here the
/// virtual cursor position (parsed from the escape stream) picks the hue:
/// hue = os + row·dy + col·dx, so every fixed cell keeps one colour no
/// matter when or how often it is redrawn.
fn paint_anchored<R: BufRead + ?Sized>(
    fd: &mut R,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    const CHUNK: usize = 4096;
    let (dx, dy) = opts.phase_step();
    let reset: &[u8] = if opts.invert { b"\x1b[49m" } else { b"\x1b[39m" };

    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK + 64);
    let mut buf = [0u8; CHUNK];
    let mut pos: (i64, i64) = (0, 0); // (row, col) of the next cell
    let mut saved: (i64, i64) = (0, 0);
    let hue_at = |os: f64, pos: (i64, i64)| os + pos.0 as f64 * dy + pos.1 as f64 * dx;
    loop {
        let n = fd.read(&mut buf)?;
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        let mut i = 0;
        let plen = pending.len();
        while i < plen {
            let b = pending[i];
            if b == 0x1b {
                // ESC 7 / ESC 8: save / restore cursor.
                if i + 1 < plen && (pending[i + 1] == b'7' || pending[i + 1] == b'8') {
                    if pending[i + 1] == b'7' {
                        saved = pos;
                    } else {
                        pos = saved;
                    }
                    out.write_all(&pending[i..i + 2])?;
                    i += 2;
                    continue;
                }
                match escape_len(&pending[i..]) {
                    Some(l) => {
                        if l >= 3 && pending[i + 1] == b'[' {
                            csi_move(&pending[i..i + l], &mut pos, &mut saved);
                        }
                        out.write_all(&pending[i..i + l])?;
                        i += l;
                    }
                    None => break, // truncated escape: wait for more input
                }
            } else if b < 0x80 {
                match b {
                    b'\n' => {
                        // Terminal output processing maps LF to CRLF on the
                        // way to the display, so a new line starts at col 0.
                        pos.0 += 1;
                        pos.1 = 0;
                        out.write_all(b"\n")?;
                    }
                    b'\r' => {
                        pos.1 = 0;
                        out.write_all(b"\r")?;
                    }
                    b'\t' => {
                        // Tabs expand to eight cells; colour every cell.
                        for _ in 0..8 {
                            emit_char(
                                out,
                                eng,
                                opts,
                                reset,
                                b" ",
                                hue_at(eng.os, pos),
                            )?;
                            pos.1 += 1;
                        }
                    }
                    0x08 => {
                        pos.1 = (pos.1 - 1).max(0);
                        out.write_all(b"\x08")?;
                    }
                    c if c < 0x20 || c == 0x7f => {
                        // Other control bytes: pass through, no cell advance.
                        out.write_all(&pending[i..i + 1])?;
                    }
                    _ => {
                        emit_char(out, eng, opts, reset, &pending[i..i + 1], hue_at(eng.os, pos))?;
                        pos.1 += 1;
                    }
                }
                i += 1;
            } else {
                // Multi-byte UTF-8 character, or an invalid byte.
                match utf8_char_len(b) {
                    Some(l) if i + l <= plen => {
                        if pending[i + 1..i + l].iter().all(|&c| c & 0xc0 == 0x80) {
                            emit_char(out, eng, opts, reset, &pending[i..i + l], hue_at(eng.os, pos))?;
                            pos.1 += 1;
                            i += l;
                        } else {
                            out.write_all(&pending[i..i + 1])?;
                            i += 1;
                        }
                    }
                    Some(_) => break, // character split across reads: hold
                    None => {
                        out.write_all(&pending[i..i + 1])?;
                        i += 1;
                    }
                }
            }
        }
        if i > 0 {
            pending.drain(..i);
        }
        out.flush()?;
    }
    if !pending.is_empty() {
        out.write_all(&pending)?;
        pending.clear();
    }
    Ok(())
}

/// Colorize a stream of text. Non-animated input is painted incrementally in
/// 4096-byte blocks (so newline-less producers like `cmatrix` work); with
/// `--animate` each line is faded through its frames before the next one.
/// With `--anchor` the hue is computed from the *screen position* of every
/// character (tracked through the escape stream) instead of its position in
/// the stream, so a full-screen TUI that redraws only changed cells keeps
/// stable colours at every fixed location.
pub fn cat<R: BufRead + ?Sized>(
    fd: &mut R,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    eng.os = opts.os;
    if opts.animate && !opts.anchor {
        out.write_all(b"\x1b[?25l")?;
        let (_, dy) = opts.phase_step();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = fd.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            eng.os += dy;
            match std::str::from_utf8(&buf) {
                Ok(s) => println(s, opts, eng, out)?,
                Err(_) => out.write_all(&buf)?, // invalid UTF-8 → pass through
            }
        }
        return Ok(());
    }
    if !eng.paint_init {
        set_mode(eng, opts.truecolor);
    }
    if opts.anchor {
        paint_anchored(fd, opts, eng, out)
    } else {
        paint_stream(fd, opts, eng, out)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn render(body: &str, opts: &Options) -> Vec<u8> {
        let mut eng = Engine::new();
        eng.os = opts.os;
        let mut out = Vec::new();
        println_plain(body, opts, &mut eng, &mut out).unwrap();
        out
    }

    #[test]
    fn hue_to_rgb_wheel() {
        assert_eq!(hue_to_rgb(0.0), [255, 0, 0]); // red
        assert_eq!(hue_to_rgb(60.0), [255, 255, 0]); // yellow
        assert_eq!(hue_to_rgb(120.0), [0, 255, 0]); // green
        assert_eq!(hue_to_rgb(180.0), [0, 255, 255]); // cyan
        assert_eq!(hue_to_rgb(240.0), [0, 0, 255]); // blue
        assert_eq!(hue_to_rgb(300.0), [255, 0, 255]); // magenta
        assert_eq!(hue_to_rgb(360.0), [255, 0, 0]); // full revolution
        assert_eq!(hue_to_rgb(-120.0), [0, 0, 255]); // wraps to 240
        assert_eq!(hue_to_rgb(480.0), [0, 255, 0]); // wraps to 120
    }

    #[test]
    fn color_for_palettes() {
        // classic pastel: hue 0 = soft green (original lolcat sine mapping)
        assert_eq!(color_for(0.0, false), [128, 237, 18]);
        // pure wheel: hue 0 = pure red
        assert_eq!(color_for(0.0, true), [255, 0, 0]);
        // both wrap around after one revolution
        assert_eq!(color_for(360.0, false), color_for(0.0, false));
        assert_eq!(color_for(360.0, true), color_for(0.0, true));
    }

    #[test]
    fn render_256_fg() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        // pastel hue 0° = (128,237,18) → nearest xterm-256 = 118
        let exp = b"\x1b[38;5;118ma\x1b[39m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn render_truecolor() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.truecolor = true;
        let exp = b"\x1b[38;2;128;237;18ma\x1b[39m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn render_invert() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.invert = true;
        let exp = b"\x1b[48;5;118ma\x1b[49m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn render_pure_wheel() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.pure = true;
        // pure hue 0° = (255,0,0) → ANSI bright red = 9
        let exp = b"\x1b[38;5;9ma\x1b[39m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn rgb_to_256_corners() {
        assert_eq!(rgb_to_256(0, 0, 0), 0); // system black
        assert_eq!(rgb_to_256(255, 255, 255), 15); // system white (closer than 231)
    }

    #[test]
    fn rgb_to_256_cube() {
        assert_eq!(rgb_to_256(95, 0, 0), 52); // cube (1,0,0)=16+36
        assert_eq!(rgb_to_256(0, 95, 0), 22); // cube (0,1,0)=16+6
        assert_eq!(rgb_to_256(0, 0, 95), 17); // cube (0,0,1)=16+1
    }

    #[test]
    fn rgb_to_256_gray() {
        assert_eq!(rgb_to_256(8, 8, 8), 232); // 8 + 10*0
        assert_eq!(rgb_to_256(18, 18, 18), 233); // 8 + 10*1
        assert_eq!(rgb_to_256(238, 238, 238), 255); // 8 + 10*23
    }

    /// The naive full-palette scan (reference implementation).
    fn rgb_brute(r: u8, g: u8, b: u8) -> u8 {
        XTERM256_PALETTE
            .iter()
            .enumerate()
            .min_by_key(|(_, &[pr, pg, pb])| {
                let dr = (r as i32 - pr as i32).pow(2);
                let dg = (g as i32 - pg as i32).pow(2);
                let db = (b as i32 - pb as i32).pow(2);
                dr + dg + db
            })
            .map(|(i, _)| i as u8)
            .unwrap_or(0)
    }

    #[test]
    fn rgb_to_256_matches_brute_sampled() {
        // Dense grid plus all palette/midpoint boundary values.
        let mut xs: Vec<u8> = (0..=255).step_by(7).collect();
        xs.extend([
            0, 1, 2, 47, 48, 49, 94, 95, 96, 114, 115, 116, 127, 128, 129, 134, 135, 136,
            174, 175, 176, 214, 215, 216, 253, 254, 255,
        ]);
        for &r in &xs {
            for &g in &xs {
                for &b in &xs {
                    assert_eq!(rgb_to_256(r, g, b), rgb_brute(r, g, b), "({r},{g},{b})");
                }
            }
        }
    }

    #[test]
    #[ignore = "exhaustive over all 2^24 inputs; run with --release -- --ignored"]
    fn rgb_to_256_matches_brute_exhaustive() {
        for r in 0..=255u32 {
            for g in 0..=255u32 {
                for b in 0..=255u32 {
                    assert_eq!(
                        rgb_to_256(r as u8, g as u8, b as u8),
                        rgb_brute(r as u8, g as u8, b as u8),
                        "({r},{g},{b})"
                    );
                }
            }
        }
    }

    #[test]
    fn escape_passthrough() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        let out = render("\x1b[31mX", &opts);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("\x1b[31m\x1b[38;5;"));
        assert!(s.ends_with("X\x1b[39m"));
    }

    #[test]
    fn scan_lone_trailing_escape() {
        let pairs = scan_pairs("a\x1b[31m");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[1], ("\x1b[31m".to_string(), None));
    }

    #[test]
    fn scan_utf8_char() {
        let pairs = scan_pairs("你a");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].1, Some('你'));
        assert_eq!(pairs[1].1, Some('a'));
    }

    #[test]
    fn cat_line_based() {
        let input = b"a\nb\n";
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 90.0; // dy = 360/60 = 6.0° per line
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &input[..], &opts, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 12.0); // +6 per line
        assert_eq!(String::from_utf8(out).unwrap().matches('\n').count(), 2);
    }

    #[test]
    fn cat_default_dy_matches_classic() {
        let input = b"a\nb\n";
        let mut opts = Options::defaults();
        opts.os = 0.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &input[..], &opts, &mut eng, &mut out).unwrap();
        let expect = 2.0 * 6.0 * 71.6f64.to_radians().sin();
        assert!((eng.os - expect).abs() < 1e-9);
    }

    /// Reader that hands out at most `step` bytes per read, to exercise
    /// escape/UTF-8 sequences split across arbitrary chunk boundaries.
    struct Steps<'a> {
        data: &'a [u8],
        pos: usize,
        step: usize,
    }

    impl<'a> Steps<'a> {
        fn new(data: &'a [u8], step: usize) -> Steps<'a> {
            Steps { data, pos: 0, step }
        }
    }

    impl<'a> Read for Steps<'a> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = (self.data.len() - self.pos).min(self.step).min(buf.len());
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl<'a> BufRead for Steps<'a> {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Ok(&self.data[self.pos..])
        }
        fn consume(&mut self, amt: usize) {
            self.pos += amt;
        }
    }

    #[test]
    fn stream_splits_do_not_corrupt() {
        let cases: &[&[u8]] = &[
            b"plain text\nsecond line\n",
            b"\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m\n",
            "你a\n好b\n".as_bytes(),
            b"\x1b]0;title\x07tail\n",
            b"mix\xff\xfeutf\n",
            b"\x1b[31m",
            b"\r\n",
            b"\x1b(B \x1b(0x \x1b#8 \x1b%G \x1b\"q\n",
        ];
        for data in cases {
            let run = |step: usize| -> Vec<u8> {
                let mut opts = Options::defaults();
                opts.os = 1.0;
                let mut eng = Engine::new();
                let mut out = Vec::new();
                let mut r = Steps::new(data, step);
                cat(&mut r, &opts, &mut eng, &mut out).unwrap();
                out
            };
            assert_eq!(run(1), run(3), "step 1 vs 3 differs for {:?}", data);
            assert_eq!(run(1), run(4096), "step 1 vs 4096 differs for {:?}", data);
        }
    }

    #[test]
    fn stream_no_newline_matches_render() {
        // cmatrix-style input (no '\n' at all) must be painted immediately
        // and match the per-line renderer for the same body.
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 0.0;
        opts.truecolor = true;
        opts.pure = true;
        let exp = render("abcdef", &opts);
        let mut eng = Engine::new();
        let mut out = Vec::new();
        let mut r = Steps::new(b"abcdef", 2);
        cat(&mut r, &opts, &mut eng, &mut out).unwrap();
        assert_eq!(out, exp);
        assert_eq!(eng.os, 0.0); // angle 0 → dy = 0
    }

    #[test]
    fn stream_advances_os_per_line() {
        // os bookkeeping identical to the old line-based path: one dy per
        // line, including empty lines (a\n\nb\n = three advances).
        let mut opts = Options::defaults();
        opts.os = 10.0;
        opts.angle = 90.0; // dy = 6.0 per line, dx = 0
        let mut eng = Engine::new();
        let mut out = Vec::new();
        let mut r = Steps::new(b"a\n\nb\n", 1);
        cat(&mut r, &opts, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 10.0 + 3.0 * 6.0);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.matches('\n').count(), 3);
    }

    #[test]
    fn stream_keeps_charset_designations() {
        // ESC <intermediate> <final> sequences (charset selection, DEC
        // alignment, UTF-8 mode, …) must pass through intact, never split
        // so an escape byte leaks out as a visible coloured character.
        let data: &[u8] = b"\x1b(B\x1b(0\x1b#8\x1b%G\x1b\"q ab\n";
        let mut opts = Options::defaults();
        opts.os = 1.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        let mut r = Steps::new(data, 1); // 1 byte per read: forces every boundary
        cat(&mut r, &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        for seq in ["\x1b(B", "\x1b(0", "\x1b#8", "\x1b%G", "\x1b\"q"] {
            assert!(s.contains(seq), "missing intact sequence {:?}", seq);
        }
        // No colour code may be interposed inside these sequences.
        for split in ["\x1b(\x1b[", "\x1b#\x1b[", "\x1b%\x1b[", "\x1b\"\x1b["] {
            assert!(!s.contains(split), "escape split at {:?}", split);
        }
        // The trailing real text is still emitted.
        assert!(s.contains('a') && s.contains('b'), "trailing text must be present");
    }

    #[test]
    fn anchor_same_cell_stable_color() {
        // Same screen cell reached twice (with junk in between) must get the
        // same colour: hue depends on the position, not the stream offset.
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 0.0; // dx = 6°/col, dy = 0
        opts.freq = 60.0;
        opts.truecolor = true;
        opts.pure = true;
        opts.anchor = true;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        let data: &[u8] = b"\x1b[2;4HX\x1b[1;1Hjunk\x1b[2;4HZ";
        let mut r = Steps::new(data, 1);
        cat(&mut r, &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        // (row 1, col 3) → hue 0 + 1·0 + 3·6 = 18° → (255, 76, 0)
        let code18 = "\x1b[38;2;255;76;0m";
        assert!(s.contains(&format!("{}X\x1b[39m", code18)), "X: {:?}", s);
        assert!(s.contains(&format!("{}Z\x1b[39m", code18)), "Z: {:?}", s);
        // home cell 'j' → hue 0° = pure red
        assert!(s.contains("\x1b[38;2;255;0;0mj\x1b[39m"), "j: {:?}", s);
    }

    #[test]
    fn anchor_relative_cursor_moves() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 0.0;
        opts.freq = 60.0;
        opts.truecolor = true;
        opts.pure = true;
        opts.anchor = true;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        // a@col0 (hue 0); printing advances to col1, C moves to col2, D back.
        // b and c both land on col2 → identical hue (12° → 255;51;0).
        let data: &[u8] = b"\x1b[1;1Ha\x1b[Cb\x1b[Dc";
        let mut r = Steps::new(data, 1);
        cat(&mut r, &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[38;2;255;0;0ma\x1b[39m"), "a: {:?}", s);
        let code12 = "\x1b[38;2;255;51;0m";
        assert!(s.contains(&format!("{}b\x1b[39m", code12)), "b: {:?}", s);
        assert!(s.contains(&format!("{}c\x1b[39m", code12)), "c: {:?}", s);
    }

    #[test]
    fn anchor_row_phase_uses_dy() {
        // angle 90 → hue advances with the row: (1,1) vs (2,1) differ by dy.
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 90.0; // dy = 6°/row, dx ≈ 0
        opts.freq = 60.0;
        opts.truecolor = true;
        opts.pure = true;
        opts.anchor = true;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        let data: &[u8] = b"\x1b[1;1Ha\x1b[2;1Hb";
        let mut r = Steps::new(data, 1);
        cat(&mut r, &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[38;2;255;0;0ma\x1b[39m"), "row1 a: {:?}", s);
        assert!(s.contains("\x1b[38;2;255;25;0mb\x1b[39m"), "row2 b (hue 6): {:?}", s);
    }

    #[test]
    fn phase_step_cardinals() {
        let mut opts = Options::defaults();
        opts.freq = 60.0; // step = 6°/grid unit

        opts.angle = 0.0;
        assert_eq!(opts.phase_step(), (6.0, 0.0)); // up: vertical stripes

        opts.angle = 90.0;
        let (dx, dy) = opts.phase_step();
        assert!(dx.abs() < 1e-12);
        assert_eq!(dy, 6.0); // right: horizontal stripes

        opts.angle = 180.0;
        let (dx, dy) = opts.phase_step();
        assert_eq!(dx, -6.0);
        assert!(dy.abs() < 1e-12);

        opts.angle = 270.0;
        let (dx, dy) = opts.phase_step();
        assert!(dx.abs() < 1e-12);
        assert_eq!(dy, -6.0);

        opts.angle = -360.0; // normalizes to 0
        assert_eq!(opts.phase_step(), (6.0, 0.0));
        opts.angle = 360.0;
        assert_eq!(opts.phase_step(), (6.0, 0.0));
    }

    #[test]
    fn phase_step_default_angle() {
        let opts = Options::defaults();
        let (dx, dy) = opts.phase_step();
        assert!((dx - 1.894).abs() < 1e-3); // 6·cos(71.6°)
        assert!((dy - 5.693).abs() < 1e-3); // 6·sin(71.6°)
    }

    #[test]
    fn render_angle_0_vertical_stripes() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 0.0; // dx = 6°/char: hues change across the row
        opts.truecolor = true;
        // pastel hues 0° and 6°
        let exp = b"\x1b[38;2;128;237;18ma\x1b[39m\x1b[38;2;141;230;11mb\x1b[39m".to_vec();
        assert_eq!(render("ab", &opts), exp);
    }

    #[test]
    fn render_angle_90_chars_same_row_phase() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 90.0; // dx ≈ 0: all chars of a row share the hue
        let out = render("ab", &opts);
        let s = String::from_utf8(out).unwrap();
        // both chars carry the same color code
        assert_eq!(s.matches("\x1b[38;5;118m").count(), 2);
    }

    #[test]
    fn hue_cycles_once_per_freq_chars() {
        let mut opts = Options::defaults();
        opts.os = 0.0;
        opts.angle = 0.0;
        opts.freq = 5.0; // one full hue revolution per 5 chars
        opts.truecolor = true;
        opts.pure = true;
        let out = render("abcdef", &opts);
        let s = String::from_utf8(out).unwrap();
        // chars 0 and 5 (hue 0° and 360°) are both pure red
        assert!(s.starts_with("\x1b[38;2;255;0;0ma\x1b[39m"));
        assert!(s.contains("\x1b[38;2;255;0;0mf\x1b[39m"));
        // char 1 is hue 72° → (204, 255, 0)
        assert!(s.contains("\x1b[38;2;204;255;0mb\x1b[39m"));
    }

    #[test]
    fn cat_invalid_utf8_passthrough() {
        let input = b"\xff\n";
        let mut opts = Options::defaults();
        opts.os = 1.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &input[..], &opts, &mut eng, &mut out).unwrap();
        assert_eq!(out, b"\xff\n");
    }

    #[test]
    fn tab_expansion() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        let mut eng = Engine::new();
        eng.os = opts.os;
        let mut out = Vec::new();
        println("\ta\n", &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.matches(' ').count(), 8);
        assert!(s.ends_with("a\x1b[39m\n"));
    }

    #[test]
    fn animate_frame_structure() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        opts.animate = true;
        opts.duration = 1;
        opts.speed = 1000.0;
        let mut eng = Engine::new();
        eng.os = opts.os;
        let mut out = Vec::new();
        println("x\n", &opts, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("\x1b7\x1b8"));
        assert!(s.contains("x\x1b[39m"));
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn animate_restores_os() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        opts.animate = true;
        opts.duration = 2;
        opts.speed = 1000.0;
        let mut eng = Engine::new();
        eng.os = opts.os;
        let mut out = Vec::new();
        println("x\n", &opts, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 1.0);
        assert!(String::from_utf8(out).unwrap().starts_with("\x1b7"));
    }

    #[test]
    fn strip_csi_ops_cases() {
        assert_eq!(strip_csi_ops("a\x1b[Jb"), "ab");
        assert_eq!(strip_csi_ops("a\x1b[2Jb"), "ab");
        assert_eq!(strip_csi_ops("a\x1b[31mb"), "a\x1b[31mb");
    }
}
