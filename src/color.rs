// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! Colour math: hue → RGB mapping for both palettes, the standard xterm-256
//! nearest-colour table, and SGR sequence emission into a stack buffer.

use crate::engine::ColorMode;

/// True hue wheel: hue 0° = pure red (ff0000), one full revolution around
/// the HSV circle (yellow → green → cyan → blue → magenta → red).
fn hue_to_rgb(hue: f64) -> [u8; 3] {
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
pub(crate) fn color_for(hue: f64, pure: bool) -> [u8; 3] {
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
fn rgb_to_256(red: u8, green: u8, blue: u8) -> u8 {
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
pub(crate) fn write_sgr(buf: &mut [u8], mode: ColorMode, invert: bool, rgb: [u8; 3]) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;

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
            0, 1, 2, 47, 48, 49, 94, 95, 96, 114, 115, 116, 127, 128, 129, 134, 135, 136, 174, 175,
            176, 214, 215, 216, 253, 254, 255,
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
}
