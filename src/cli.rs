// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! Command-line interface: the clap-derive `Cli` struct, its translation to
//! [`Options`](crate::options::Options), and numeric range validation.

use std::path::PathBuf;

use clap::Parser;

use crate::help::VERSION;
use crate::options::Options;

#[derive(Parser, Debug)]
#[command(
    name = "lolcat",
    about,
    version = VERSION,
    trailing_var_arg = true,
    allow_negative_numbers = true
)]
pub(crate) struct Cli {
    /// Rainbow frequency: hue completes one full cycle every F grid units
    #[arg(short = 'F', long = "freq", default_value = "60")]
    freq: f64,

    /// Rainbow seed, 0 = random hue
    #[arg(short = 'S', long = "seed", default_value = "0")]
    seed: i64,

    /// Rainbow direction in degrees: 0 = up (vertical stripes), clockwise positive
    #[arg(short = 'A', long = "angle", default_value = "71.6")]
    angle: f64,

    /// Enable psychedelics (animation)
    #[arg(short = 'a', long = "animate", default_value_t = false)]
    animate: bool,

    /// Animation duration (number of frames per line)
    #[arg(short = 'd', long = "duration", default_value = "2")]
    duration: u64,

    /// Animation speed (frames per second)
    #[arg(short = 's', long = "speed", default_value = "60.0")]
    speed: f64,

    /// Invert foreground and background colors
    #[arg(short = 'i', long = "invert", default_value_t = false)]
    invert: bool,

    /// 24-bit truecolor mode
    #[arg(short = 't', long = "truecolor", default_value_t = false)]
    truecolor: bool,

    /// Pure saturated hue-wheel palette (default: classic pastel)
    #[arg(short = 'P', long = "pure", default_value_t = false)]
    pure: bool,

    /// Anchor colours to fixed screen positions (stable for full-screen TUIs)
    #[arg(short = 'B', long = "anchor", default_value_t = false)]
    anchor: bool,

    /// Keep the input's own styling (background colours, bold, …; under -i
    /// the input foreground) when the animation freezes a cell. Default: on.
    #[arg(short = 'K', long = "keep", action = clap::ArgAction::SetTrue)]
    keep: bool,

    /// Turn `--keep` off.
    #[arg(long = "no-keep", action = clap::ArgAction::SetTrue)]
    no_keep: bool,

    /// Force color even when stdout is not a tty
    #[arg(short = 'f', long = "force", default_value_t = false)]
    force: bool,

    /// Input files (use "-" for stdin)
    #[arg(default_value = "-")]
    pub(crate) files: Vec<PathBuf>,
}

/// Range validation for numeric options; returns the offending message.
pub(crate) fn validate(cli: &Cli) -> Result<(), String> {
    if !cli.freq.is_finite() || cli.freq <= 0.0 {
        return Err("--freq must be a finite number > 0".into());
    }
    if cli.duration < 1 {
        return Err("--duration must be >= 1".into());
    }
    if cli.speed < 0.1 {
        return Err("--speed must be >= 0.1".into());
    }
    if !cli.angle.is_finite() || !(-360.0..=360.0).contains(&cli.angle) {
        return Err("--angle must be a finite number between -360 and 360".into());
    }
    Ok(())
}

pub(crate) fn cli_to_opts(cli: &Cli) -> Options {
    let mut opts = Options::defaults();
    opts.freq = cli.freq;
    opts.seed = cli.seed;
    opts.angle = cli.angle;
    // os is the starting hue in degrees: random around the wheel when
    // seed = 0, otherwise seed mod 360.
    opts.os = if cli.seed == 0 {
        rand::random_range(0.0..360.0)
    } else {
        cli.seed.rem_euclid(360) as f64
    };
    opts.animate = cli.animate;
    opts.duration = cli.duration;
    opts.speed = cli.speed;
    opts.invert = cli.invert;
    opts.truecolor = cli.truecolor;
    opts.pure = cli.pure;
    opts.anchor = cli.anchor;
    opts.keep = !cli.no_keep || cli.keep;
    opts.force = cli.force;
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_defaults() {
        let cli = Cli::parse_from(["lolcat"]);
        assert_eq!(cli.freq, 60.0);
        assert_eq!(cli.seed, 0);
        assert_eq!(cli.angle, 71.6);
        assert!(!cli.animate);
        assert_eq!(cli.duration, 2);
        assert_eq!(cli.speed, 60.0);
        assert!(!cli.invert);
        assert!(!cli.truecolor);
        assert!(!cli.pure);
        assert!(!cli.anchor);
        assert!(!cli.force);
        assert_eq!(cli.files.len(), 1);
    }

    #[test]
    fn cli_custom_values() {
        let cli = Cli::parse_from([
            "lolcat", "-F", "30", "-S", "42", "-A", "90", "-a", "-d", "6", "-s", "30", "-i", "-t",
            "-P", "-B", "-f", "file.txt",
        ]);
        assert_eq!(cli.freq, 30.0);
        assert_eq!(cli.seed, 42);
        assert_eq!(cli.angle, 90.0);
        assert!(cli.animate);
        assert_eq!(cli.duration, 6);
        assert_eq!(cli.speed, 30.0);
        assert!(cli.invert);
        assert!(cli.truecolor);
        assert!(cli.pure);
        assert!(cli.anchor);
        assert!(cli.force);
        assert_eq!(cli.files[0].to_string_lossy(), "file.txt");
    }

    #[test]
    fn cli_angle_negative_and_attached() {
        let cli = Cli::parse_from(["lolcat", "-A-45"]);
        assert_eq!(cli.angle, -45.0);
        let cli = Cli::parse_from(["lolcat", "--angle", "-360"]);
        assert_eq!(cli.angle, -360.0);
    }

    #[test]
    fn cli_dash_is_stdin() {
        let cli = Cli::parse_from(["lolcat", "-"]);
        assert_eq!(cli.files[0].to_string_lossy(), "-");
    }

    #[test]
    fn validate_ranges() {
        let ok = |mut f: Box<dyn FnMut(&mut Cli)>| {
            let mut cli = Cli::parse_from(["lolcat"]);
            f(&mut cli);
            validate(&cli)
        };
        assert!(ok(Box::new(|_| {})).is_ok());
        assert!(ok(Box::new(|c| c.freq = 60.0)).is_ok());
        assert!(ok(Box::new(|c| c.freq = 0.0)).is_err());
        assert!(ok(Box::new(|c| c.freq = -5.0)).is_err());
        assert!(ok(Box::new(|c| c.freq = f64::NAN)).is_err());
        assert!(ok(Box::new(|c| c.freq = f64::INFINITY)).is_err());
        assert!(ok(Box::new(|c| c.duration = 0)).is_err());
        assert!(ok(Box::new(|c| c.speed = 0.05)).is_err());
        assert!(ok(Box::new(|c| c.angle = 360.0)).is_ok());
        assert!(ok(Box::new(|c| c.angle = -360.0)).is_ok());
        assert!(ok(Box::new(|c| c.angle = 360.1)).is_err());
        assert!(ok(Box::new(|c| c.angle = -360.1)).is_err());
        assert!(ok(Box::new(|c| c.angle = f64::NAN)).is_err());
        assert!(ok(Box::new(|c| c.angle = f64::INFINITY)).is_err());
    }
}
