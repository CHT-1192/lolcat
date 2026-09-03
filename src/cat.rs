// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Stream colouring dispatcher. Non-animated input is painted incrementally
//! in 4096-byte blocks (so newline-less producers like `cmatrix` work);
//! with `--animate` each line is faded through its frames before the next
//! one. With `--anchor` the hue is computed from the *screen position* of
//! every character (tracked through the escape stream) instead of its
//! position in the stream, so a full-screen TUI that redraws only changed
//! cells keeps stable colours at every fixed location.

use std::io::{self, BufRead, Write};

use crate::anchor::paint_anchored;
use crate::engine::{set_mode, Engine};
use crate::options::Options;
use crate::render::println;
use crate::stream::paint_stream;

/// Colorize a stream of text. Non-animated input is painted incrementally in
/// 4096-byte blocks (so newline-less producers like `cmatrix` work); with
/// `--animate` each line is faded through its frames before the next one.
/// With `--anchor` the hue is computed from the *screen position* of every
/// character (tracked through the escape stream) instead of its position in
/// the stream, so a full-screen TUI that redraws only changed cells keeps
/// stable colours at every fixed location.
pub(crate) fn cat<R: BufRead + ?Sized>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Per-line renderer used as the reference for the streaming painter.
    fn render(body: &str, opts: &Options) -> Vec<u8> {
        let mut eng = Engine::new();
        eng.os = opts.os;
        let mut out = Vec::new();
        crate::render::println_plain(body, opts, &mut eng, &mut out).unwrap();
        out
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
        assert!(
            s.contains('a') && s.contains('b'),
            "trailing text must be present"
        );
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
        assert!(
            s.contains("\x1b[38;2;255;25;0mb\x1b[39m"),
            "row2 b (hue 6): {:?}",
            s
        );
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
}
