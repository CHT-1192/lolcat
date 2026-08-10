// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! lolcat — modern Rust port (clap-derive CLI, standard exits).

mod lol;

use std::io::{self, BufRead, BufWriter, IsTerminal, Write};
use std::path::PathBuf;
use std::process;

use clap::Parser;
use lol::{Engine, Options};

const VERSION: &str = "100.0.1 (c)2011 moe@busyloop.net";

// Hardcoded help text matching the Ruby optimist educate output exactly.
// We use this instead of clap's auto-generated help to preserve the
// original format, then render it through the rainbow colorizer.
const HELP_HEADER: &str = "\nUsage: lolcat [OPTION]... [FILE]...\n\n\
Concatenate FILE(s), or standard input, to standard output.\n\
With no FILE, or when FILE is -, read standard input.\n\n";

const HELP_FOOTER: &str = concat!(
    "\nExamples:\n",
    "  lolcat f - g      Output f\'s contents, then stdin, then g\'s contents.\n",
    "  lolcat            Copy standard input to standard output.\n",
    "  fortune | lolcat  Display a rainbow cookie.\n",
    "\n",
    "Report lolcat bugs to <https://github.com/busyloop/lolcat/issues>\n",
    "lolcat home page: <https://github.com/busyloop/lolcat/>\n",
    "Report lolcat translation bugs to <http://speaklolcat.com/>\n",
);

const HELP_OPTIONS: [(&str, &str); 11] = [
    ("-p, --spread=<f>", "Rainbow spread (default: 3.0)"),
    ("-F, --freq=<f>", "Rainbow frequency (default: 0.1)"),
    ("-S, --seed=<i>", "Rainbow seed, 0 = random (default: 0)"),
    ("-a, --animate", "Enable psychedelics"),
    ("-d, --duration=<i>", "Animation duration (default: 12)"),
    ("-s, --speed=<f>", "Animation speed (default: 20.0)"),
    ("-i, --invert", "Invert fg and bg"),
    ("-t, --truecolor", "24-bit (truecolor)"),
    ("-f, --force", "Force color even when stdout is not a tty"),
    ("-v, --version", "Print version and exit"),
    ("-h, --help", "Show this message"),
];

fn help_text() -> String {
    let mut s = String::from(HELP_HEADER);
    for (spec, desc) in &HELP_OPTIONS {
        s.push_str(&format!("  {:<18}    {}\n", spec, desc));
    }
    s.push_str(HELP_FOOTER);
    s.push('\n');
    s
}

#[derive(Parser, Debug)]
#[command(
    name = "lolcat",
    about,
    version = VERSION,
    trailing_var_arg = true
)]
struct Cli {
    /// Rainbow spread (character widths per line height)
    #[arg(short = 'p', long = "spread", default_value = "3.0")]
    spread: f64,

    /// Rainbow frequency
    #[arg(short = 'F', long = "freq", default_value = "0.1")]
    freq: f64,

    /// Rainbow seed, 0 = random
    #[arg(short = 'S', long = "seed", default_value = "0")]
    seed: i64,

    /// Enable psychedelics (animation)
    #[arg(short = 'a', long = "animate", default_value_t = false)]
    animate: bool,

    /// Animation duration (number of frames per line)
    #[arg(short = 'd', long = "duration", default_value = "12")]
    duration: u64,

    /// Animation speed (frames per second)
    #[arg(short = 's', long = "speed", default_value = "20.0")]
    speed: f64,

    /// Invert foreground and background colors
    #[arg(short = 'i', long = "invert", default_value_t = false)]
    invert: bool,

    /// 24-bit truecolor mode
    #[arg(short = 't', long = "truecolor", default_value_t = false)]
    truecolor: bool,

    /// Force color even when stdout is not a tty
    #[arg(short = 'f', long = "force", default_value_t = false)]
    force: bool,

    /// Input files (use "-" for stdin)
    #[arg(default_value = "-")]
    files: Vec<PathBuf>,
}

/// Terminal restore guard — RAII equivalent of Ruby's `ensure` block.
struct ResetGuard { tty: bool }

impl Drop for ResetGuard {
    fn drop(&mut self) {
        if self.tty {
            let mut out = io::stdout();
            let _ = out.write_all(b"\x1b[m\x1b[?25h\x1b[?1;5;2004l");
            let _ = out.flush();
        }
    }
}

fn install_ctrlc_handler(tty: bool) {
    let _ = ctrlc::set_handler(move || {
        if tty {
            let mut out = io::stdout();
            let _ = out.write_all(b"\x1b[m\x1b[?25h\x1b[?1;5;2004l");
            let _ = out.flush();
        }
        process::exit(0);
    });
}

fn file_error(file: &PathBuf, msg: &str) -> ! {
    eprintln!("lolcat: {}: {}", file.display(), msg);
    process::exit(1);
}

/// Check raw args for -h/--help (including bundled like -ah) before
/// clap parsing, so we can render the help through the rainbow colorizer.
fn wants_help() -> bool {
    std::env::args().skip(1).any(|a| {
        a == "-h" || a == "--help"                  // standalone
        || (a.starts_with('-') && a.contains('h')   // bundled short e.g. -ah
            && !a.starts_with("--"))                 // but not --something
    })
}

fn main() {
    // Intercept --help/-h before clap to render rainbow-colored help
    if wants_help() {
        let stdout_tty = io::stdout().is_terminal();
        install_ctrlc_handler(stdout_tty);
        let text = help_text();
        let opts = Options::for_help();
        let mut eng = Engine::new();
        let mut out = BufWriter::new(io::stdout());
        {
            let _guard = ResetGuard { tty: stdout_tty };
            let _ = lol::cat(&mut text.as_bytes(), &opts, &mut eng, &mut out);
            let _ = out.write_all(b"\n");
            let _ = out.flush();
        }
        process::exit(0);
    }

    let cli = Cli::parse();

    // range validation (like Ruby's p.die)
    if cli.spread < 0.1 {
        eprintln!("Error: argument --spread must be >= 0.1.");
        process::exit(2);
    }
    if cli.duration < 1 {
        eprintln!("Error: argument --duration must be >= 1.");
        process::exit(2);
    }
    if cli.speed < 0.1 {
        eprintln!("Error: argument --speed must be >= 0.1.");
        process::exit(2);
    }

    let stdout_tty = io::stdout().is_terminal();
    install_ctrlc_handler(stdout_tty);

    let mut opts = Options::defaults();
    opts.spread = cli.spread;
    opts.freq = cli.freq;
    opts.seed = cli.seed;
    opts.os = if cli.seed == 0 { rand::random::<u8>() as f64 } else { cli.seed as f64 };
    opts.animate = cli.animate;
    opts.duration = cli.duration;
    opts.speed = cli.speed;
    opts.invert = cli.invert;
    opts.truecolor = cli.truecolor;
    opts.force = cli.force;

    let mut eng = Engine::new();
    let mut out = BufWriter::new(io::stdout());

    for file in &cli.files {
        let path_str = file.to_string_lossy();
        let mut fd: Box<dyn io::BufRead> = if path_str == "-" {
            Box::new(io::BufReader::new(io::stdin()))
        } else {
            match std::fs::File::open(file) {
                Ok(f) => Box::new(io::BufReader::new(f)),
                Err(e) => {
                    let msg = match e.kind() {
                        io::ErrorKind::NotFound => "No such file or directory",
                        io::ErrorKind::PermissionDenied => "Permission denied",
                        io::ErrorKind::IsADirectory => "Is a directory",
                        _ => "Is not a regular file",
                    };
                    file_error(file, msg);
                }
            }
        };

        if stdout_tty || opts.force {
            let _guard = ResetGuard { tty: stdout_tty };
            if let Err(e) = lol::cat(&mut *fd, &opts, &mut eng, &mut out) {
                if e.kind() == io::ErrorKind::BrokenPipe {
                    process::exit(1);
                }
                file_error(file, &e.to_string());
            }
        } else {
            let mut line = String::new();
            loop {
                line.clear();
                match fd.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Err(e) = out.write_all(line.as_bytes()) {
                            if e.kind() == io::ErrorKind::BrokenPipe { process::exit(1); }
                            file_error(file, &e.to_string());
                        }
                    }
                    Err(e) => {
                        if e.kind() == io::ErrorKind::BrokenPipe { process::exit(1); }
                        file_error(file, &e.to_string());
                    }
                }
            }
        }
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_defaults() {
        let cli = Cli::parse_from(["lolcat"]);
        assert_eq!(cli.spread, 3.0);
        assert_eq!(cli.freq, 0.1);
        assert_eq!(cli.seed, 0);
        assert!(!cli.animate);
        assert_eq!(cli.duration, 12);
        assert_eq!(cli.speed, 20.0);
        assert!(!cli.invert);
        assert!(!cli.truecolor);
        assert!(!cli.force);
        assert_eq!(cli.files.len(), 1);
    }

    #[test]
    fn cli_custom_values() {
        let cli = Cli::parse_from([
            "lolcat", "-p", "2.5", "-F", "0.3", "-S", "42", "-a", "-d", "6",
            "-s", "30", "-i", "-t", "-f", "file.txt",
        ]);
        assert_eq!(cli.spread, 2.5);
        assert_eq!(cli.freq, 0.3);
        assert_eq!(cli.seed, 42);
        assert!(cli.animate);
        assert_eq!(cli.duration, 6);
        assert_eq!(cli.speed, 30.0);
        assert!(cli.invert);
        assert!(cli.truecolor);
        assert!(cli.force);
        assert_eq!(cli.files[0].to_string_lossy(), "file.txt");
    }

    #[test]
    fn cli_dash_is_stdin() {
        let cli = Cli::parse_from(["lolcat", "-"]);
        assert_eq!(cli.files[0].to_string_lossy(), "-");
    }
}
