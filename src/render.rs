// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Line-oriented rendering: paint a whole string (plain, inverted, pure),
//! with or without the animated multi-frame fade, and emit one character
//! into a stream.

use std::io::{self, Write};

use crate::ansi::{expand_tabs, scan_pairs, strip_csi_ops};
use crate::color::{color_for, write_sgr};
use crate::engine::{set_mode, Engine};
use crate::options::Options;

/// Paint one complete character: colour code, character bytes, reset —
/// composed into a single stack buffer (no per-character allocation).
/// With a foreground colour a space is invisible anyway, so in normal (non
/// inverted) mode spaces pass through uncoloured; the cursor/column phase
/// still advances.
pub(crate) fn emit_char(
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

pub(crate) fn println(
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

pub(crate) fn println_plain(
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
    fn escape_passthrough() {
        let mut opts = Options::defaults();
        opts.os = 1.0;
        let out = render("\x1b[31mX", &opts);
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("\x1b[31m\x1b[38;5;"));
        assert!(s.ends_with("X\x1b[39m"));
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
}
