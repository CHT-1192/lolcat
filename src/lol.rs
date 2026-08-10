// Copyright (c) 2016, moe@busyloop.net
// ... BSD 3-Clause (see LICENSE)
//
//! Rainbow coloring engine — byte-faithful port of `lib/lolcat/lol.rb`.
//! Replicates Paint-gem 256-color mapping, 4096-byte chunked reading, and
//! the partial-line phase heuristic.

use std::io::{self, Read, Write};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode { Truecolor, Pal256 }

#[derive(Clone, Copy)]
pub struct Options {
    pub freq: f64, pub seed: i64, pub os: f64, pub spread: f64,
    pub animate: bool, pub duration: i32, pub speed: f64,
    pub invert: bool, pub truecolor: bool, pub force: bool,
}

impl Options {
    pub fn defaults() -> Options {
        Options { freq: 0.1, seed: 0, os: 0.0, spread: 3.0, animate: false,
            duration: 12, speed: 20.0, invert: false, truecolor: false, force: false }
    }
    pub fn for_help() -> Options {
        Options { animate: false, duration: 12, os: rand::random::<f64>() * 8192.0,
            speed: 20.0, spread: 8.0, freq: 0.3, ..Options::defaults() }
    }
}

pub struct Engine {
    pub os: f64,
    old_os: Option<f64>,
    paint_init: bool,
    pub mode: ColorMode,
}

impl Engine {
    pub fn new() -> Engine { Engine { os: 0.0, old_os: None, paint_init: false, mode: ColorMode::Pal256 } }
}

pub fn rainbow(freq: f64, phase: f64) -> [u8; 3] {
    let r = (freq * phase).sin() * 127.0 + 128.0;
    let g = (freq * phase + 2.0 * std::f64::consts::PI / 3.0).sin() * 127.0 + 128.0;
    let b = (freq * phase + 4.0 * std::f64::consts::PI / 3.0).sin() * 127.0 + 128.0;
    [r as u8, g as u8, b as u8]
}

/// Paint gem's `rgb_to_256` — grayscale threshold heuristic + 6×6×6 cube.
pub fn rgb_to_256(red: u8, green: u8, blue: u8) -> u8 {
    let (r, g, b) = (red as f64, green as f64, blue as f64);
    let mut sep = 42.5;
    let gray = loop {
        if r < sep || g < sep || b < sep { break r < sep && g < sep && b < sep; }
        sep += 42.5;
    };
    if gray { (232 + ((r + g + b) / 33.0).round() as i32) as u8 }
    else { (16 + 36 * (6.0 * r / 256.0) as i32 + 6 * (6.0 * g / 256.0) as i32 + (6.0 * b / 256.0) as i32) as u8 }
}

pub fn color_code(mode: ColorMode, invert: bool, rgb: [u8; 3]) -> Vec<u8> {
    let [r, g, b] = rgb;
    match mode {
        ColorMode::Truecolor =>
            format!("\x1b[{};2;{};{};{}m", if invert { 48 } else { 38 }, r, g, b).into_bytes(),
        ColorMode::Pal256 => {
            let n = rgb_to_256(r, g, b);
            format!("\x1b[{};5;{}m", if invert { 48 } else { 38 }, n).into_bytes()
        }
    }
}

pub fn set_mode(eng: &mut Engine, truecolor: bool) {
    let detected = match std::env::var("COLORTERM").as_deref() {
        Ok("truecolor") | Ok("24bit") => ColorMode::Truecolor, _ => ColorMode::Pal256 };
    eng.mode = if truecolor { ColorMode::Truecolor } else { detected };
    if std::env::var_os("LOLCAT_DEBUG").is_some() {
        let num = |m: ColorMode| if m == ColorMode::Truecolor { 16777215 } else { 256 };
        eprintln!("DEBUG: Paint.mode = {} (detected: {})", num(eng.mode), num(detected));
    }
    eng.paint_init = true;
}

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
        if j < n { Some((chars[i..=j].iter().collect(), j + 1)) }
        else if j - (i + 1) >= 2 { Some((chars[i..j].iter().collect(), j)) }
        else { Some((chars[i..=i + 1].iter().collect(), i + 2)) }
    } else if is_marker(c) {
        let mut j = i + 2;
        while j < n && chars[j] != '\x07' && chars[j] != '\x1b' { j += 1; }
        Some((chars[i..j].iter().collect(), j))
    } else if c == '[' {
        let mut j = i + 2;
        while j < n && is_csi(chars[j]) { j += 1; }
        if j < n { Some((chars[i..=j].iter().collect(), j + 1)) }
        else if j - (i + 2) >= 1 { Some((chars[i..j].iter().collect(), j)) }
        else { Some((chars[i..=i + 1].iter().collect(), i + 2)) }
    } else { Some((chars[i..=i + 1].iter().collect(), i + 2)) }
}

pub fn incomplete_escape(buf: &[u8]) -> bool {
    let s = if buf.last() == Some(&b'\n') { &buf[..buf.len() - 1] } else { buf };
    let n = s.len();
    if n == 0 { return false; }
    let is_fmt = |b: u8| (0x20..=0x2f).contains(&b);
    let is_csi = |b: u8| (0x30..=0x3f).contains(&b);
    let mut j = n;
    while j > 0 && is_fmt(s[j - 1]) { j -= 1; }
    if j > 0 && s[j - 1] == 0x1b { return true; }
    let mut k = n;
    while k > 0 && s[k - 1] != 0x07 && s[k - 1] != 0x1b { k -= 1; }
    if k >= 1 && k < n && matches!(s[k], b']' | b'P' | b'X' | b'^' | b'_') && s[k - 1] == 0x1b { return true; }
    let mut l = n;
    while l > 0 && is_csi(s[l - 1]) { l -= 1; }
    if l >= 2 && s[l - 1] == b'[' && s[l - 2] == 0x1b { return true; }
    false
}

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
        out.push(chars[i]); i += 1;
    }
    out
}

pub fn println(line: &str, opts: &Options, eng: &mut Engine, out: &mut dyn Write) -> io::Result<()> {
    let chomped = line.ends_with('\n');
    let body = if chomped { &line[..line.len() - 1] } else { line };
    let mut body = expand_tabs(body);
    if opts.animate { println_ani(&mut body, opts, eng, out, chomped)?; }
    else { println_plain(&body, opts, eng, out, chomped)?; }
    if chomped { out.write_all(b"\n")?; }
    Ok(())
}

fn println_plain(str: &str, opts: &Options, eng: &mut Engine, out: &mut dyn Write, chomped: bool) -> io::Result<()> {
    if !eng.paint_init { set_mode(eng, opts.truecolor); }
    let filtered = scan_pairs(str);
    let reset = if opts.invert { b"\x1b[49m".as_slice() } else { b"\x1b[39m".as_slice() };
    for (i, (esc, ch)) in filtered.iter().enumerate() {
        let rgb = rainbow(opts.freq, eng.os + (i as f64) / opts.spread);
        let code = color_code(eng.mode, opts.invert, rgb);
        out.write_all(esc.as_bytes())?;
        out.write_all(&code)?;
        if let Some(c) = ch { let mut buf = [0u8; 4]; out.write_all(c.encode_utf8(&mut buf).as_bytes())?; }
        out.write_all(reset)?;
    }
    if !chomped {
        eng.old_os = Some(eng.os);
        eng.os += filtered.len() as f64 / opts.spread;
    } else if eng.old_os.is_some() { eng.os = eng.old_os.take().unwrap(); }
    Ok(())
}

fn println_ani(str: &mut String, opts: &Options, eng: &mut Engine, out: &mut dyn Write, chomped: bool) -> io::Result<()> {
    if str.is_empty() { return Ok(()); }
    out.write_all(b"\x1b7")?;
    let real_os = eng.os;
    for _ in 1..=opts.duration {
        out.write_all(b"\x1b8")?;
        eng.os += opts.spread;
        println_plain(str, opts, eng, out, chomped)?;
        *str = strip_csi_ops(str);
        std::thread::sleep(std::time::Duration::from_secs_f64(1.0 / opts.speed));
    }
    eng.os = real_os;
    Ok(())
}

pub fn cat<R: Read + ?Sized>(fd: &mut R, opts: &Options, eng: &mut Engine, out: &mut dyn Write) -> io::Result<()> {
    eng.os = opts.os;
    if opts.animate { out.write_all(b"\x1b[?25l")?; }
    let mut tmp = [0u8; 4096];
    'outer: loop {
        let mut buf: Vec<u8> = Vec::new();
        let mut eof = false;
        loop {
            let n = fd.read(&mut tmp)?;
            if n == 0 { eof = true; break; }
            buf.extend_from_slice(&tmp[..n]);
            if std::str::from_utf8(&buf).is_ok() && !incomplete_escape(&buf) { break; }
        }
        if eof { break 'outer; }
        let mut start = 0;
        for (i, &b) in buf.iter().enumerate() {
            if b == b'\n' {
                let line = std::str::from_utf8(&buf[start..=i]).unwrap();
                eng.os += 1.0; println(line, opts, eng, out)?;
                start = i + 1;
            }
        }
        if start < buf.len() {
            let line = std::str::from_utf8(&buf[start..]).unwrap();
            eng.os += 1.0; println(line, opts, eng, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(body: &str, opts: &Options, chomped: bool) -> Vec<u8> {
        let mut eng = Engine::new(); eng.os = opts.os;
        let mut out = Vec::new();
        println_plain(body, opts, &mut eng, &mut out, chomped).unwrap();
        out
    }

    #[test] fn rainbow_seed1_char0() { assert_eq!(rainbow(0.1, 1.0), [140, 231, 12]); }
    #[test] fn rainbow_zero_freq() { assert_eq!(rainbow(0.0, 0.0), [128, 237, 18]); }
    #[test] fn rgb_to_256_cube() { assert_eq!(rgb_to_256(140, 230, 13), 154); }
    #[test] fn rgb_to_256_gray() {
        assert_eq!(rgb_to_256(100, 100, 100), 241);
        assert_eq!(rgb_to_256(42, 42, 42), 236);
        assert_eq!(rgb_to_256(255, 255, 255), 255);
        assert_eq!(rgb_to_256(0, 0, 0), 232);
        assert_eq!(rgb_to_256(200, 50, 200), 170);
    }
    #[test] fn render_256_fg() {
        let mut o = Options::defaults(); o.os = 1.0;
        assert_eq!(render("a", &o, false), b"\x1b[38;5;154ma\x1b[39m".to_vec());
    }
    #[test] fn render_truecolor() {
        let mut o = Options::defaults(); o.os = 1.0; o.truecolor = true;
        assert_eq!(render("a", &o, false), b"\x1b[38;2;140;231;12ma\x1b[39m".to_vec());
    }
    #[test] fn render_invert() {
        let mut o = Options::defaults(); o.os = 1.0; o.invert = true;
        assert_eq!(render("a", &o, false), b"\x1b[48;5;154ma\x1b[49m".to_vec());
    }
    #[test] fn render_two_chars() {
        let mut o = Options::defaults(); o.os = 1.0;
        let out = render("ab", &o, false);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("\x1b[38;5;154ma\x1b[39m\x1b[38;5;"));
        assert!(s.ends_with("b\x1b[39m"));
    }
    #[test] fn escape_passthrough() {
        let mut o = Options::defaults(); o.os = 1.0;
        let out = render("\x1b[31mX", &o, false);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("\x1b[31m\x1b[38;5;"));
    }
    #[test] fn scan_trailing_escape() {
        let pairs = scan_pairs("a\x1b[31m");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[1], ("\x1b[31m".to_string(), None));
    }
    #[test] fn scan_escape_space() {
        let pairs = scan_pairs("x\x1b ");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[1], ("\x1b ".to_string(), None));
    }
    #[test] fn scan_escape_space_two() {
        let pairs = scan_pairs("\x1b  ");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], ("\x1b  ".to_string(), None));
    }
    #[test] fn scan_escape_mid() {
        let pairs = scan_pairs("\x1b  X");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "\x1b  X");
        assert_eq!(pairs[0].1, None);
    }
    #[test] fn scan_utf8() {
        let pairs = scan_pairs("你a");
        assert_eq!(pairs[0].1, Some('你'));
        assert_eq!(pairs[1].1, Some('a'));
    }
    #[test] fn scan_osc() {
        let pairs = scan_pairs("\x1b]0;title\x07x");
        assert_eq!(pairs[0].0, "\x1b]0;title");
        assert_eq!(pairs[0].1, Some('\x07'));
        assert_eq!(pairs[1].1, Some('x'));
    }
    #[test] fn incomplete_escape_cases() {
        assert!(incomplete_escape(b"abc\x1b[31"));
        assert!(incomplete_escape(b"abc\x1b"));
        assert!(incomplete_escape(b"abc\x1b "));
        assert!(incomplete_escape(b"abc\x1b]0;title"));
        assert!(incomplete_escape(b"abc\x1b[31\n"));
        assert!(!incomplete_escape(b"abc\x1b[31m"));
        assert!(!incomplete_escape(b"abc\x1b[31m\n"));
        assert!(!incomplete_escape(b"abc"));
        assert!(!incomplete_escape(b""));
    }
    #[test] fn strip_csi_ops_cases() {
        assert_eq!(strip_csi_ops("a\x1b[Jb"), "ab");
        assert_eq!(strip_csi_ops("a\x1b[2Jb"), "ab");
        assert_eq!(strip_csi_ops("a\x1b[31mb"), "a\x1b[31mb");
    }
    #[test] fn partial_line_phase() {
        let mut o = Options::defaults(); o.os = 1.0; o.spread = 3.0;
        let mut eng = Engine::new(); eng.os = o.os;
        let mut out = Vec::new();
        println_plain("ab", &o, &mut eng, &mut out, false).unwrap();
        assert_eq!(eng.old_os, Some(1.0));
        assert!((eng.os - (1.0 + 2.0 / 3.0)).abs() < 1e-9);
        println_plain("c", &o, &mut eng, &mut out, true).unwrap();
        assert_eq!(eng.os, 1.0);
        assert_eq!(eng.old_os, None);
    }
    #[test] fn cat_line_counting() {
        let mut o = Options::defaults(); o.os = 5.0; o.spread = 3.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut "a\nb\n".as_bytes(), &o, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 7.0);
        assert_eq!(String::from_utf8(out).unwrap().matches('\n').count(), 2);
    }
    #[test] fn cat_long_across_chunks() {
        let mut line = "x".repeat(5000) + "\n";
        let mut o = Options::defaults(); o.os = 1.0; o.spread = 3.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut line.as_bytes(), &o, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 2.0);
    }
    #[test] fn cat_discards_incomplete() {
        let mut o = Options::defaults(); o.os = 1.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &b"abc\x1b[31"[..], &o, &mut eng, &mut out).unwrap();
        assert!(out.is_empty());
    }
    #[test] fn cat_empty() {
        let mut o = Options::defaults(); o.os = 1.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &b""[..], &o, &mut eng, &mut out).unwrap();
        assert!(out.is_empty());
    }
    #[test] fn cat_no_trailing_nl() {
        let mut o = Options::defaults(); o.os = 1.0; o.spread = 3.0;
        let mut eng = Engine::new();
        let mut out = Vec::new();
        cat(&mut &b"hello"[..], &o, &mut eng, &mut out).unwrap();
        assert!(!out.ends_with(b"\n"));
        assert!((eng.os - (2.0 + 5.0 / 3.0)).abs() < 1e-9);
    }
    #[test] fn tabs_expand() {
        let mut o = Options::defaults(); o.os = 1.0; o.spread = 3.0;
        let mut eng = Engine::new(); eng.os = o.os;
        let mut out = Vec::new();
        println("\ta\n", &o, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.matches(' ').count(), 8);
        assert!(s.ends_with("a\x1b[39m\n"));
    }
    #[test] fn animate_frame() {
        let mut o = Options::defaults(); o.os = 1.0; o.animate = true; o.duration = 1; o.speed = 1000.0;
        let mut eng = Engine::new(); eng.os = o.os;
        let mut out = Vec::new();
        println("x\n", &o, &mut eng, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("\x1b7\x1b8"));
        assert!(s.contains("x\x1b[39m"));
        assert!(s.ends_with('\n'));
    }
    #[test] fn animate_restores_os() {
        let mut o = Options::defaults(); o.os = 1.0; o.animate = true; o.duration = 2; o.speed = 1000.0;
        let mut eng = Engine::new(); eng.os = o.os;
        let mut out = Vec::new();
        println("x\n", &o, &mut eng, &mut out).unwrap();
        assert_eq!(eng.os, 1.0);
    }
    #[test] fn animate_skips_empty() {
        let mut o = Options::defaults(); o.os = 1.0; o.animate = true; o.duration = 3; o.speed = 1000.0;
        let mut eng = Engine::new(); eng.os = o.os;
        let mut out = Vec::new();
        println("\n", &o, &mut eng, &mut out).unwrap();
        assert_eq!(out, b"\n");
    }
}
