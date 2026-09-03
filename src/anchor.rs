// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Screen-anchored painter (`--anchor`): the hue of each character comes
//! from its *screen position* — tracked by simulating the cursor moves in
//! the escape stream — instead of its position in the byte stream, so a
//! full-screen TUI that redraws only changed cells keeps stable colours at
//! every fixed location. Invoked through [`crate::cat::cat`].

use std::io::{self, BufRead, Write};

use crate::ansi::{csi_move, escape_len, utf8_char_len};
use crate::engine::Engine;
use crate::options::Options;
use crate::render::emit_char;

/// Colorize a stream whose hue is anchored to screen coordinates.
///
/// Full-screen TUIs (btop, htop, ...) redraw only the cells that changed,
/// jumping around with `ESC [ y ; x H`. In stream order those rewrites land
/// at unpredictable offsets, so a stream-linear hue flickers. Here the
/// virtual cursor position (parsed from the escape stream) picks the hue:
/// hue = os + row·dy + col·dx, so every fixed cell keeps one colour no
/// matter when or how often it is redrawn.
pub(crate) fn paint_anchored<R: BufRead + ?Sized>(
    fd: &mut R,
    opts: &Options,
    eng: &mut Engine,
    out: &mut dyn Write,
) -> io::Result<()> {
    const CHUNK: usize = 4096;
    let (dx, dy) = opts.phase_step();
    let reset: &[u8] = if opts.invert {
        b"\x1b[49m"
    } else {
        b"\x1b[39m"
    };

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
                            emit_char(out, eng, opts, reset, b" ", hue_at(eng.os, pos))?;
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
                        emit_char(
                            out,
                            eng,
                            opts,
                            reset,
                            &pending[i..i + 1],
                            hue_at(eng.os, pos),
                        )?;
                        pos.1 += 1;
                    }
                }
                i += 1;
            } else {
                // Multi-byte UTF-8 character, or an invalid byte.
                match utf8_char_len(b) {
                    Some(l) if i + l <= plen => {
                        if pending[i + 1..i + l].iter().all(|&c| c & 0xc0 == 0x80) {
                            emit_char(
                                out,
                                eng,
                                opts,
                                reset,
                                &pending[i..i + l],
                                hue_at(eng.os, pos),
                            )?;
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
