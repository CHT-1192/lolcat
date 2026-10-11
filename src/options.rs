// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! User-facing options (`--freq`, `--seed`, `--angle`, …) and the hue
//! phase-step model shared by every paint path.

/// Terminal tab-stop interval used by [`TabWidth::Stops`].
pub(crate) const TAB_STOP: usize = 8;

/// How a tab character expands. Configured by the `"tab"` key in
/// `lolcat.config.jsonc` (there is no CLI flag for it): a positive number
/// means exactly that many spaces, `-1` means the next terminal tab stop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TabWidth {
    /// Every tab becomes exactly this many spaces.
    Spaces(usize),
    /// Advance to the next multiple of [`TAB_STOP`] columns.
    Stops,
}

impl TabWidth {
    /// Number of cells a tab occupies when it starts at column `col`.
    pub(crate) fn cells(self, col: usize) -> usize {
        match self {
            TabWidth::Spaces(n) => n,
            TabWidth::Stops => TAB_STOP - (col % TAB_STOP),
        }
    }
}

impl Default for TabWidth {
    fn default() -> TabWidth {
        TabWidth::Spaces(TAB_STOP)
    }
}

/// 256-colour quantiser (`-Q/--quantize`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub(crate) enum Quantizer {
    /// True nearest entry of the xterm-256 palette (default).
    #[default]
    Nearest,
    /// The Paint gem's buckets, byte-identical to lolcat 100.0.1's
    /// 256-colour mode (`Paint.color`).
    Paint,
}

/// Where the reveal front meets the window's bottom row — the `"bottom"`
/// config key (no CLI flag). `0` and `1` both mean the first column;
/// fractions below 1 are a proportion of the terminal width; integers ≥ 1
/// are absolute columns (1-based).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum BottomSpec {
    /// `v × terminal columns` (0 < v < 1).
    Fraction(f64),
    /// Absolute 1-based column.
    Absolute(usize),
}

impl Default for BottomSpec {
    fn default() -> BottomSpec {
        // Maintainer-chosen default: the front may not leave the window
        // before the top row is finished, so 0.4 of the width is a value
        // where the diagonal just about fits the default angle.
        BottomSpec::Fraction(0.4)
    }
}

impl BottomSpec {
    /// Classify a config value. `0` → first column; `0 < v < 1` → fraction;
    /// `v ≥ 1` must be integral → absolute column. Negative, non-finite and
    /// non-integral values ≥ 1 are rejected (the caller skips the key).
    pub(crate) fn from_value(v: f64) -> Option<BottomSpec> {
        if !v.is_finite() || v < 0.0 {
            return None;
        }
        if v == 0.0 {
            return Some(BottomSpec::Absolute(1));
        }
        if v < 1.0 {
            return Some(BottomSpec::Fraction(v));
        }
        if v.fract() == 0.0 && v <= usize::MAX as f64 {
            Some(BottomSpec::Absolute(v as usize))
        } else {
            None
        }
    }

    /// Resolve to a 1-based column, clamped to `1..=cols`.
    pub(crate) fn column(self, cols: usize) -> usize {
        let cols = cols.max(1);
        let n = match self {
            BottomSpec::Fraction(f) => (f * cols as f64).round() as usize,
            BottomSpec::Absolute(n) => n,
        };
        n.clamp(1, cols)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Options {
    pub(crate) freq: f64,
    pub(crate) seed: i64,
    pub(crate) os: f64,
    pub(crate) angle: f64,
    pub(crate) pure: bool,
    pub(crate) anchor: bool,
    pub(crate) animate: bool,
    pub(crate) duration: u64,
    pub(crate) speed: f64,
    pub(crate) invert: bool,
    pub(crate) truecolor: bool,
    pub(crate) keep: bool,
    pub(crate) force: bool,
    /// Tab expansion (`"tab"` config key; default: 8 fixed spaces).
    pub(crate) tab: TabWidth,
    /// 256-colour mapping (`-Q/--quantize`; default: nearest).
    pub(crate) quantizer: Quantizer,
    /// Where the reveal front meets the window's bottom row (`"bottom"`
    /// config key; default 0.4 of the width). Only used when the text is
    /// taller than the screen.
    pub(crate) bottom: BottomSpec,
}

impl Options {
    pub(crate) fn defaults() -> Options {
        Options {
            freq: 60.0,
            seed: 0,
            os: 0.0,
            angle: 71.6,
            pure: false,
            anchor: false,
            animate: false,
            duration: 3,
            speed: 60.0,
            invert: false,
            truecolor: false,
            keep: true,
            force: false,
            tab: TabWidth::default(),
            quantizer: Quantizer::default(),
            bottom: BottomSpec::default(),
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
    pub(crate) fn phase_step(&self) -> (f64, f64) {
        let a = self.angle.rem_euclid(360.0).to_radians();
        let step = 360.0 / self.freq;
        (a.cos() * step, a.sin() * step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bottom_spec_values() {
        assert_eq!(BottomSpec::default(), BottomSpec::Fraction(0.4));
        assert_eq!(BottomSpec::from_value(0.0), Some(BottomSpec::Absolute(1)));
        assert_eq!(BottomSpec::from_value(1.0), Some(BottomSpec::Absolute(1)));
        assert_eq!(BottomSpec::from_value(0.2), Some(BottomSpec::Fraction(0.2)));
        assert_eq!(BottomSpec::from_value(24.0), Some(BottomSpec::Absolute(24)));
        assert_eq!(BottomSpec::from_value(-1.0), None);
        assert_eq!(BottomSpec::from_value(2.5), None);
        assert_eq!(BottomSpec::from_value(f64::NAN), None);
        // Fractions resolve against the terminal width, absolutes stay put.
        assert_eq!(BottomSpec::Fraction(0.4).column(120), 48);
        assert_eq!(BottomSpec::Fraction(0.2).column(120), 24);
        assert_eq!(BottomSpec::Absolute(24).column(120), 24);
        // Clamped into 1..=cols (0 and 1 both mean the first column).
        assert_eq!(BottomSpec::Absolute(9999).column(120), 120);
        assert_eq!(BottomSpec::Fraction(0.0).column(120), 1);
    }

    #[test]
    fn tab_width_cells() {
        // Default: eight fixed spaces wherever the tab appears.
        assert_eq!(TabWidth::default(), TabWidth::Spaces(8));
        assert_eq!(TabWidth::Spaces(4).cells(0), 4);
        assert_eq!(TabWidth::Spaces(4).cells(9), 4);
        // Tab stops: pad to the next multiple of 8.
        assert_eq!(TabWidth::Stops.cells(0), 8);
        assert_eq!(TabWidth::Stops.cells(1), 7);
        assert_eq!(TabWidth::Stops.cells(7), 1);
        assert_eq!(TabWidth::Stops.cells(8), 8);
        assert_eq!(TabWidth::Stops.cells(15), 1);
    }
}
