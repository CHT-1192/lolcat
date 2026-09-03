// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! ANSI escape-sequence parsing and bookkeeping shared by the renderer, the
//! streaming painters and the screen-anchored painter: splitting a string
//! into (escape, char) pairs, measuring byte lengths of sequences, stripping
//! cursor-erasing ops, tab expansion, and CSI cursor-move simulation.

/// Scan a string into `(escape_run, char)` pairs — equivalent to Ruby's
/// `str.scan(ANSI_ESCAPE)`.
pub(crate) fn scan_pairs(s: &str) -> Vec<(String, Option<char>)> {
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

/// Expand tabs to eight spaces (matches the `expand` command's tab stops).
pub(crate) fn expand_tabs(s: &str) -> String {
    s.replace('\t', "        ")
}

/// Remove CSI cursor-erase operations (`ESC [ … @/J/K/P/X`) from a string,
/// so an animated line keeps its length after a frame clears part of it.
pub(crate) fn strip_csi_ops(s: &str) -> String {
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

/// Length in bytes of a complete ANSI escape sequence starting at `b[0]`
/// (which must be ESC), or `None` when the sequence is truncated and more
/// input is needed to finish it.
pub(crate) fn escape_len(b: &[u8]) -> Option<usize> {
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
pub(crate) fn utf8_char_len(b: u8) -> Option<usize> {
    match b {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

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
pub(crate) fn csi_move(seq: &[u8], pos: &mut (i64, i64), saved: &mut (i64, i64)) {
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
        b'h' | b'l' if p(0) == 1049 || p(0) == 47 => {
            // Entering/leaving the alternate screen resets the coordinate
            // space, so jump back to the top-left corner.
            *pos = (0, 0);
        }
        b'h' | b'l' => {}
        _ => {} // colour/clear/scroll/etc.: cursor unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn strip_csi_ops_cases() {
        assert_eq!(strip_csi_ops("a\x1b[Jb"), "ab");
        assert_eq!(strip_csi_ops("a\x1b[2Jb"), "ab");
        assert_eq!(strip_csi_ops("a\x1b[31mb"), "a\x1b[31mb");
    }
}
