// Copyright (c) 2016, moe@busyloop.net
// All rights reserved.
// ... (BSD 3-Clause, see LICENSE)
//
//! User-facing options (`--freq`, `--seed`, `--angle`, …) and the hue
//! phase-step model shared by every paint path.

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
            duration: 12,
            speed: 20.0,
            invert: false,
            truecolor: false,
            keep: true,
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
}
