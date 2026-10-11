// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! Command-line interface: the clap-derive `Cli` struct, its translation to
//! [`Options`](crate::options::Options), and numeric range validation.

use std::path::PathBuf;

use clap::Parser;

use crate::config::ConfigDefaults;
use crate::help::VERSION;
use crate::options::{Options, Quantizer, TabWidth, TAB_STOP};

#[derive(Parser, Debug)]
#[command(
    name = "lolcat",
    about,
    version = VERSION,
    disable_version_flag = true,
    trailing_var_arg = true,
    allow_negative_numbers = true
)]
pub(crate) struct Cli {
    /// Rainbow frequency: hue completes one full cycle every F grid units
    #[arg(short = 'F', long = "freq")]
    freq: Option<f64>,

    /// Rainbow seed, 0 = random hue
    #[arg(short = 'S', long = "seed")]
    seed: Option<i64>,

    /// Rainbow direction in degrees: 0 = up (vertical stripes), clockwise positive
    #[arg(short = 'A', long = "angle")]
    angle: Option<f64>,

    /// Enable psychedelics (animation)
    #[arg(short = 'a', long = "animate", default_value_t = false)]
    animate: bool,

    /// Animation duration (number of frames per line)
    #[arg(short = 'd', long = "duration")]
    duration: Option<u64>,

    /// Animation speed (frames per second)
    #[arg(short = 's', long = "speed")]
    speed: Option<f64>,

    /// Invert foreground and background colors
    #[arg(short = 'i', long = "invert", default_value_t = false)]
    invert: bool,

    /// 24-bit truecolor mode
    #[arg(short = 't', long = "truecolor", default_value_t = false)]
    truecolor: bool,

    /// 256-colour mapping: nearest (default) or paint (lolcat 100.0.1 parity)
    #[arg(short = 'Q', long = "quantize", value_enum)]
    quantize: Option<Quantizer>,

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

    /// Print version and exit (lowercase -v; hidden -V kept for compat)
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    version: Option<bool>,

    #[arg(short = 'V', action = clap::ArgAction::Version, hide = true)]
    version_cap: Option<bool>,

    /// Input files (use "-" for stdin)
    #[arg(default_value = "-")]
    pub(crate) files: Vec<PathBuf>,
}

/// Range validation for the numeric options **explicitly given on the
/// command line** (`None` = absent, falls through to config/built-in). The
/// config file validates its own keys when parsing, and built-in defaults
/// are valid by construction, so validating each source at its origin is
/// equivalent to validating the merged result once.
pub(crate) fn validate(cli: &Cli) -> Result<(), String> {
    if let Some(f) = cli.freq {
        if !f.is_finite() || f <= 0.0 {
            return Err("--freq must be a finite number > 0".into());
        }
    }
    if let Some(d) = cli.duration {
        if d < 1 {
            return Err("--duration must be >= 1".into());
        }
    }
    if let Some(s) = cli.speed {
        if !s.is_finite() || s < 0.1 {
            return Err("--speed must be a finite number >= 0.1".into());
        }
    }
    if let Some(a) = cli.angle {
        if !a.is_finite() || !(-360.0..=360.0).contains(&a) {
            return Err("--angle must be a finite number between -360 and 360".into());
        }
    }
    Ok(())
}

/// Effective value of one option: command line > config file > built-in.
fn pick<T: Copy>(cli: Option<T>, cfg: Option<T>, builtin: T) -> T {
    cli.or(cfg).unwrap_or(builtin)
}

pub(crate) fn cli_to_opts(cli: &Cli, cfg: &ConfigDefaults) -> Options {
    let d = Options::defaults();
    let freq = pick(cli.freq, cfg.freq, d.freq);
    let seed = pick(cli.seed, cfg.seed, d.seed);
    let angle = pick(cli.angle, cfg.angle, d.angle);
    let duration = pick(cli.duration, cfg.duration, d.duration);
    let speed = pick(cli.speed, cfg.speed, d.speed);

    let mut opts = Options::defaults();
    opts.freq = freq;
    opts.seed = seed;
    opts.angle = angle;
    // os is the starting hue in degrees: random around the wheel when
    // seed = 0, otherwise seed mod 360.
    opts.os = if seed == 0 {
        rand::random_range(0.0..360.0)
    } else {
        seed.rem_euclid(360) as f64
    };
    opts.duration = duration;
    opts.speed = speed;
    opts.animate = cli.animate;
    opts.invert = cli.invert;
    opts.truecolor = cli.truecolor;
    opts.pure = cli.pure;
    opts.anchor = cli.anchor;
    opts.keep = !cli.no_keep || cli.keep;
    opts.force = cli.force;
    opts.quantizer = cli.quantize.unwrap_or(Quantizer::Nearest);
    // Config-only key (no CLI flag) with a built-in default: where the
    // reveal front meets the bottom row when the text scrolls.
    opts.bottom = cfg.bottom.unwrap_or_default();
    // Config-only key (no CLI flag): positive N = N spaces, -1 = tab stops.
    opts.tab = match cfg.tab {
        Some(-1) => TabWidth::Stops,
        Some(n) if n >= 1 => TabWidth::Spaces(n as usize),
        _ => TabWidth::Spaces(TAB_STOP),
    };
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_defaults_absent() {
        // The five valued options are absent unless typed on the command
        // line; booleans keep their clap defaults. Built-in numeric
        // defaults are applied at merge time (see merge_precedence).
        let cli = Cli::parse_from(["lolcat"]);
        assert_eq!(cli.freq, None);
        assert_eq!(cli.seed, None);
        assert_eq!(cli.angle, None);
        assert_eq!(cli.duration, None);
        assert_eq!(cli.speed, None);
        assert!(!cli.animate);
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
        assert_eq!(cli.freq, Some(30.0));
        assert_eq!(cli.seed, Some(42));
        assert_eq!(cli.angle, Some(90.0));
        assert!(cli.animate);
        assert_eq!(cli.duration, Some(6));
        assert_eq!(cli.speed, Some(30.0));
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
        assert_eq!(cli.angle, Some(-45.0));
        let cli = Cli::parse_from(["lolcat", "--angle", "-360"]);
        assert_eq!(cli.angle, Some(-360.0));
    }

    #[test]
    fn cli_dash_is_stdin() {
        let cli = Cli::parse_from(["lolcat", "-"]);
        assert_eq!(cli.files[0].to_string_lossy(), "-");
    }

    #[test]
    fn cli_quantize_flag() {
        let cli = Cli::parse_from(["lolcat", "-Q", "paint"]);
        assert_eq!(cli.quantize, Some(Quantizer::Paint));
        let cli = Cli::parse_from(["lolcat", "--quantize=nearest"]);
        assert_eq!(cli.quantize, Some(Quantizer::Nearest));
        assert!(Cli::try_parse_from(["lolcat", "-Q", "bogus"]).is_err());
        // Absent → nearest (current behaviour, no output change).
        let cli = Cli::parse_from(["lolcat"]);
        assert_eq!(cli.quantize, None);
        assert_eq!(
            cli_to_opts(&cli, &ConfigDefaults::default()).quantizer,
            Quantizer::Nearest
        );
    }

    #[test]
    fn merge_precedence() {
        // command line > config > built-in defaults
        let empty = ConfigDefaults::default();
        let opts = cli_to_opts(&Cli::parse_from(["lolcat"]), &empty);
        let d = Options::defaults();
        assert_eq!(opts.freq, d.freq);
        assert_eq!(opts.seed, d.seed);
        assert_eq!(opts.angle, d.angle);
        assert_eq!(opts.duration, d.duration);
        assert_eq!(opts.speed, d.speed);
        assert!((0.0..360.0).contains(&opts.os)); // seed 0 → random hue

        // Config supplies values when the command line does not.
        let cfg = ConfigDefaults {
            freq: Some(12.0),
            seed: Some(90),
            angle: Some(180.0),
            duration: Some(9),
            speed: Some(7.5),
            tab: None,
            bottom: None,
        };
        let opts = cli_to_opts(&Cli::parse_from(["lolcat"]), &cfg);
        assert_eq!(opts.freq, 12.0);
        assert_eq!(opts.seed, 90);
        assert_eq!(opts.angle, 180.0);
        assert_eq!(opts.duration, 9);
        assert_eq!(opts.speed, 7.5);
        assert_eq!(opts.os, 90.0); // fixed seed → repeatable

        // Command line beats config.
        let cli = Cli::parse_from(["lolcat", "-F", "30", "-A", "90"]);
        let opts = cli_to_opts(&cli, &cfg);
        assert_eq!(opts.freq, 30.0);
        assert_eq!(opts.angle, 90.0);
        assert_eq!(opts.speed, 7.5); // untouched by argv → config
        assert_eq!(opts.duration, 9);

        // `tab` is config-only: absent → 8 fixed spaces; positive → that many
        // spaces; -1 → tab stops.
        assert_eq!(opts.tab, TabWidth::Spaces(8));
        let tab_cfg = ConfigDefaults {
            tab: Some(4),
            ..ConfigDefaults::default()
        };
        assert_eq!(cli_to_opts(&cli, &tab_cfg).tab, TabWidth::Spaces(4));
        let stops_cfg = ConfigDefaults {
            tab: Some(-1),
            ..ConfigDefaults::default()
        };
        assert_eq!(cli_to_opts(&cli, &stops_cfg).tab, TabWidth::Stops);
    }

    #[test]
    fn validate_ranges() {
        let ok = |mut f: Box<dyn FnMut(&mut Cli)>| {
            let mut cli = Cli::parse_from(["lolcat"]);
            f(&mut cli);
            validate(&cli)
        };
        assert!(ok(Box::new(|_| {})).is_ok());
        assert!(ok(Box::new(|c| c.freq = Some(60.0))).is_ok());
        assert!(ok(Box::new(|c| c.freq = Some(0.0))).is_err());
        assert!(ok(Box::new(|c| c.freq = Some(-5.0))).is_err());
        assert!(ok(Box::new(|c| c.freq = Some(f64::NAN))).is_err());
        assert!(ok(Box::new(|c| c.freq = Some(f64::INFINITY))).is_err());
        assert!(ok(Box::new(|c| c.duration = Some(0))).is_err());
        assert!(ok(Box::new(|c| c.speed = Some(0.05))).is_err());
        assert!(ok(Box::new(|c| c.speed = Some(f64::NAN))).is_err());
        assert!(ok(Box::new(|c| c.angle = Some(360.0))).is_ok());
        assert!(ok(Box::new(|c| c.angle = Some(-360.0))).is_ok());
        assert!(ok(Box::new(|c| c.angle = Some(360.1))).is_err());
        assert!(ok(Box::new(|c| c.angle = Some(-360.1))).is_err());
        assert!(ok(Box::new(|c| c.angle = Some(f64::NAN))).is_err());
        assert!(ok(Box::new(|c| c.angle = Some(f64::INFINITY))).is_err());
        // -a itself does not reject any angle: unsupported ones degrade to
        // plain colouring at dispatch time.
        let anim = |a: f64| {
            let cli = Cli::parse_from(["lolcat", "-a", "-A", &a.to_string()]);
            validate(&cli)
        };
        assert!(anim(45.0).is_ok());
        assert!(anim(0.0).is_ok());
        assert!(anim(-45.0).is_ok());
    }
}
