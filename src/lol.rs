// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Rainbow coloring engine — modern Rust port of lol.rb.
//!
//! Uses line-based reading (not chunked), standard xterm-256 nearest-color
//! mapping (not Paint gem grayscale heuristic), and no partial-line phase
//! tricks. Otherwise identical to the original ruby algorithm.

use std::io::{self, BufRead, Write};

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
    pub spread: f64,
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
            freq: 0.1,
            seed: 0,
            os: 0.0,
            spread: 3.0,
            animate: false,
            duration: 12,
            speed: 20.0,
            invert: false,
            truecolor: false,
            force: false,
        }
    }
    pub fn for_help() -> Options {
        Options {
            animate: false, duration: 12,
            os: rand::random::<f64>() * 8192.0,
            speed: 20.0, spread: 8.0, freq: 0.3,
            ..Options::defaults()
        }
    }
}

pub struct Engine {
    pub os: f64,
    paint_init: bool,
    pub mode: ColorMode,
}

impl Engine {
    pub fn new() -> Engine {
        Engine { os: 0.0, paint_init: false, mode: ColorMode::Pal256 }
    }
}

/// Three sine waves 120° apart, mapped to `[1, 255]`.
pub fn rainbow(freq: f64, phase: f64) -> [u8; 3] {
    let r = (freq * phase).sin() * 127.0 + 128.0;
    let g = (freq * phase + 2.0 * std::f64::consts::PI / 3.0).sin() * 127.0 + 128.0;
    let b = (freq * phase + 4.0 * std::f64::consts::PI / 3.0).sin() * 127.0 + 128.0;
    [r as u8, g as u8, b as u8]
}

// ── Standard xterm-256 palette (nearest-neighbor) ──────────────────────

const XTERM256_PALETTE: [[u8; 3]; 256] = {
    let mut pal = [[0u8; 3]; 256];
    // 0-15: ANSI system colors
    let sys = [
        [0, 0, 0], [128, 0, 0], [0, 128, 0], [128, 128, 0], [0, 0, 128],
        [128, 0, 128], [0, 128, 128], [192, 192, 192], [128, 128, 128],
        [255, 0, 0], [0, 255, 0], [255, 255, 0], [0, 0, 255], [255, 0, 255],
        [0, 255, 255], [255, 255, 255],
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

/// Standard xterm-256 nearest-neighbor (not the Paint gem's grayscale
/// heuristic). Ties go to the lower index.
pub fn rgb_to_256(red: u8, green: u8, blue: u8) -> u8 {
    let (r, g, b) = (red as i32, green as i32, blue as i32);
    XTERM256_PALETTE
        .iter()
        .enumerate()
        .min_by_key(|(_, &[pr, pg, pb])| {
            let dr = (r - pr as i32).pow(2);
            let dg = (g - pg as i32).pow(2);
            let db = (b - pb as i32).pow(2);
            dr + dg + db
        })
        .map(|(i, _)| i as u8)
        .unwrap_or(0)
}

/// Emit a foreground/background SGR color escape.
pub fn color_code(mode: ColorMode, invert: bool, rgb: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = rgb;
    match mode {
        ColorMode::Truecolor => {
            if invert {
                format!("\x1b[48;2;{};{};{}m", r, g, b)
            } else {
                format!("\x1b[38;2;{};{};{}m", r, g, b)
            }
        }
        ColorMode::Pal256 => {
            let n = rgb_to_256(r, g, b);
            if invert {
                format!("\x1b[48;5;{}m", n)
            } else {
                format!("\x1b[38;5;{}m", n)
            }
        }
    }
    .into_bytes()
}

/// Truecolor if `--truecolor` or `COLORTERM ∈ {truecolor, 24bit}`.
pub fn set_mode(eng: &mut Engine, truecolor: bool) {
    let detected = match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => ColorMode::Truecolor,
        _ => ColorMode::Pal256,
    };
    eng.mode = if truecolor { ColorMode::Truecolor } else { detected };
    if std::env::var_os("LOLCAT_DEBUG").is_some() {
        let num = |m: ColorMode| if m == ColorMode::Truecolor { 16777215 } else { 256 };
        eprintln!("DEBUG: Paint.mode = {} (detected: {})", num(eng.mode), num(detected));
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
                Some((seq, next)) => { esc.push_str(&seq); i = next; }
                None => break,
            }
        }
        let ch = if i < n { let c = chars[i]; i += 1; Some(c) } else { None };
        pairs.push((esc, ch));
    }
    pairs
}

fn parse_escape(chars: &[char], i: usize) -> Option<(String, usize)> {
    let n = chars.len();
    if i + 1 >= n { return None; }
    let c = chars[i + 1];
    let is_fmt = |ch: char| (0x20..=0x2f).contains(&(ch as u32));
    let is_csi = |ch: char| (0x30..=0x3f).contains(&(ch as u32));
    let is_marker = |ch: char| matches!(ch, ']' | 'P' | 'X' | '^' | '_');

    if is_fmt(c) {
        let mut j = i + 1;
        while j < n && is_fmt(chars[j]) { j += 1; }
        if j < n {
            Some((chars[i..=j].iter().collect(), j + 1))
        } else if j - (i + 1) >= 2 {
            Some((chars[i..j].iter().collect(), j))
        } else {
            Some((chars[i..=i + 1].iter().collect(), i + 2))
        }
    } else if is_marker(c) {
        let mut j = i + 2;
        while j < n && chars[j] != '\x07' && chars[j] != '\x1b' { j += 1; }
        Some((chars[i..j].iter().collect(), j))
    } else if c == '[' {
        let mut j = i + 2;
        while j < n && is_csi(chars[j]) { j += 1; }
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

fn expand_tabs(s: &str) -> String { s.replace('\t', "        ") }

fn strip_csi_ops(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        if chars[i] == '\x1b' && i + 1 < n && chars[i + 1] == '[' {
            let mut j = i + 2;
            while j < n && (0x30..=0x3f).contains(&(chars[j] as u32)) { j += 1; }
            if j < n && matches!(chars[j], '@' | 'J' | 'K' | 'P' | 'X') { i = j + 1; continue; }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn println(line: &str, opts: &Options, eng: &mut Engine, out: &mut dyn Write) -> io::Result<()> {
    let chomped = line.ends_with('\n');
    let body = if chomped { &line[..line.len() - 1] } else { line };
    let mut body = expand_tabs(body);
    if opts.animate {
        println_ani(&mut body, opts, eng, out, chomped)?;
    } else {
        println_plain(&body, opts, eng, out)?;
    }
    if chomped { out.write_all(b"\n")?; }
    Ok(())
}

fn println_plain(str: &str, opts: &Options, eng: &mut Engine, out: &mut dyn Write) -> io::Result<()> {
    if !eng.paint_init { set_mode(eng, opts.truecolor); }
    let filtered = scan_pairs(str);
    let reset = if opts.invert { b"\x1b[49m".as_slice() } else { b"\x1b[39m".as_slice() };
    for (i, (esc, ch)) in filtered.iter().enumerate() {
        let rgb = rainbow(opts.freq, eng.os + (i as f64) / opts.spread);
        let code = color_code(eng.mode, opts.invert, rgb);
        out.write_all(esc.as_bytes())?;
        out.write_all(&code)?;
        if let Some(c) = ch {
            let mut buf = [0u8; 4];
            out.write_all(c.encode_utf8(&mut buf).as_bytes())?;
        }
        out.write_all(reset)?;
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
    if str.is_empty() { return Ok(()); }
    out.write_all(b"\x1b7")?;
    let real_os = eng.os;
    for _ in 1..=opts.duration {
        out.write_all(b"\x1b8")?;
        eng.os += opts.spread;
        println_plain(str, opts, eng, out)?;
        *str = strip_csi_ops(str);
        std::thread::sleep(std::time::Duration::from_secs_f64(1.0 / opts.speed));
    }
    eng.os = real_os;
    Ok(())
}

/// Line-based reading — much simpler than the Ruby 4096-byte chunk
/// heuristic.  Invalid UTF-8 lines are passed through uncolored.
pub fn cat<R: BufRead + ?Sized>(
    fd: &mut R,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    eng.os = opts.os;
    if opts.animate { out.write_all(b"\x1b[?25l")?; }
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = fd.read_until(b'\n', &mut buf)?;
        if n == 0 { break; }
        eng.os += 1.0;
        match std::str::from_utf8(&buf) {
            Ok(s) => println(s, opts, eng, out)?,
            Err(_) => out.write_all(&buf)?, // invalid UTF-8 → pass through
        }
    }
    Ok(())
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
    fn rainbow_seed1_char0() {
        assert_eq!(rainbow(0.1, 1.0), [140, 231, 12]);
    }

    #[test]
    fn rainbow_zero_freq() {
        assert_eq!(rainbow(0.0, 0.0), [128, 237, 18]);
    }

    #[test]
    fn render_256_fg() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        // (140,231,12) → nearest std: cube(2,4,0) = 16+72+24 = 112
        let exp = b"\x1b[38;5;112ma\x1b[39m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn render_truecolor() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        opts.truecolor = true;
        let exp = b"\x1b[38;2;140;231;12ma\x1b[39m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn render_invert() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        opts.invert = true;
        let exp = b"\x1b[48;5;112ma\x1b[49m".to_vec();
        assert_eq!(render("a", &opts), exp);
    }

    #[test]
    fn rgb_to_256_corners() {
        assert_eq!(rgb_to_256(0, 0, 0), 0);         // system black
        assert_eq!(rgb_to_256(255, 255, 255), 15);  // system white (closer than 231)
    }

    #[test]
    fn rgb_to_256_cube() {
        assert_eq!(rgb_to_256(95, 0, 0), 52);      // cube (1,0,0)=16+36
        assert_eq!(rgb_to_256(0, 95, 0), 22);      // cube (0,1,0)=16+6
        assert_eq!(rgb_to_256(0, 0, 95), 17);      // cube (0,0,1)=16+1
    }

    #[test]
    fn rgb_to_256_gray() {
        assert_eq!(rgb_to_256(8, 8, 8), 232);      // 8 + 10*0
        assert_eq!(rgb_to_256(18, 18, 18), 233);   // 8 + 10*1
        assert_eq!(rgb_to_256(238, 238, 238), 255); // 8 + 10*23
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
        opts.os = 5.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &input[..], &opts, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 7.0); // +1 per line
        assert_eq!(String::from_utf8(out).unwrap().matches('\n').count(), 2);
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
