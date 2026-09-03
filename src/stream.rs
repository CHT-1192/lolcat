// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Streaming painter: colorizes a byte stream as it arrives in 4096-byte
//! blocks, without waiting for newlines, so newline-less producers keep
//! flowing. Invoked through [`crate::cat::cat`].

use std::io::{self, BufRead, Write};

use crate::ansi::{escape_len, utf8_char_len};
use crate::engine::Engine;
use crate::options::Options;
use crate::render::emit_char;

/// Colorize a byte stream as it arrives, without waiting for newlines.
///
/// The hue phase advances by `dx` per character within a line and by `dy`
/// per newline, matching the previous line-based output byte for byte on
/// ordinary text (the empty-line bookkeeping is preserved too). ANSI
/// escapes pass through untouched, even when split across reads; a
/// truncated escape or UTF-8 character at the end of a read is buffered
/// until it completes. Invalid UTF-8 bytes pass through uncolored.
pub(crate) fn paint_stream<R: BufRead + ?Sized>(
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
                            emit_char(out, eng, opts, reset, b" ", eng.os + (col as f64) * dx)?;
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
