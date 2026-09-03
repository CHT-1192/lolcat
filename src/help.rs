// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! Help text: the hardcoded, Ruby-optimist-shaped strings and the
//! `-h/--help` detection that re-parses the remaining flags so help can be
//! rendered through the colorizer.

use std::process;

use clap::Parser;

use crate::cli::{cli_to_opts, validate, Cli};
use crate::options::Options;

pub(crate) const VERSION: &str = "100.0.2 (c)2011 moe@busyloop.net";

// Hardcoded help text matching the Ruby optimist educate output exactly.
// We use this instead of clap's auto-generated help to preserve the
// original format, then render it through the rainbow colorizer.
const HELP_HEADER: &str = "\nUsage: lolcat [OPTION]... [FILE]...\n\n\
Concatenate FILE(s), or standard input, to standard output.\n\
With no FILE, or when FILE is -, read standard input.\n\n";

const HELP_FOOTER: &str = concat!(
    "\nExamples:\n",
    "  echo \"hello\" | lolcat        Make everything a rainbow.\n",
    "  fortune | cowsay | lolcat    Rainbow fortune cookie.\n",
    "  cmatrix | lolcat             Matrix rain, in rainbow.\n",
    "  pipes.sh -p 10 | lolcat      Animated rainbow pipes.\n",
    "  htop | lolcat -B             Live process monitor, stable colours.\n",
    "\n",
    "Report lolcat bugs to <https://github.com/busyloop/lolcat/issues>\n",
    "lolcat home page: <https://github.com/busyloop/lolcat/>\n",
    "Report lolcat translation bugs to <http://speaklolcat.com/>\n",
);

const HELP_OPTIONS: [(&str, &str); 13] = [
    (
        "-F, --freq=<f>",
        "Hue cycles once every F grid units (default: 60)",
    ),
    (
        "-S, --seed=<i>",
        "Rainbow seed, 0 = random hue (default: 0)",
    ),
    (
        "-A, --angle=<f>",
        "Direction: 0 = up, clockwise positive (default: 71.6)",
    ),
    (
        "-B, --anchor",
        "Color by fixed screen position (stable for TUIs)",
    ),
    ("-a, --animate", "Enable psychedelics"),
    ("-d, --duration=<i>", "Animation duration (default: 12)"),
    ("-s, --speed=<f>", "Animation speed (default: 20.0)"),
    ("-i, --invert", "Invert fg and bg"),
    ("-t, --truecolor", "24-bit (truecolor)"),
    (
        "-P, --pure",
        "Pure saturated hue wheel (default: classic pastel)",
    ),
    ("-f, --force", "Force color even when stdout is not a tty"),
    ("-V, --version", "Print version and exit"),
    ("-h, --help", "Show this message"),
];

pub(crate) fn help_text() -> String {
    let mut s = String::from(HELP_HEADER);
    for (spec, desc) in &HELP_OPTIONS {
        s.push_str(&format!("  {:<18}    {}\n", spec, desc));
    }
    s.push_str(HELP_FOOTER);
    s.push('\n');
    s
}

/// If raw args contain -h/--help (incl. bundled like -ah), strip the
/// help flag(s) and parse the rest with clap. Returns the parsed options
/// so help can be rendered through the colorizer with those flags applied.
/// `-th` → truecolor help, `-ah` → animate help, etc.
pub(crate) fn check_help() -> Option<Options> {
    let raw: Vec<String> = std::env::args().collect();
    let mut has_help = false;
    let mut filtered: Vec<String> = vec![raw[0].clone()]; // keep bin name

    for arg in &raw[1..] {
        if arg == "-h" || arg == "--help" {
            has_help = true;
            continue;
        }
        // bundled short: strip 'h', keep the rest (e.g. -ath → -at)
        if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 1 && arg[1..].contains('h')
        {
            has_help = true;
            let stripped: String = arg.chars().filter(|&c| c != 'h').collect();
            if stripped != "-" {
                filtered.push(stripped);
            }
            continue;
        }
        filtered.push(arg.clone());
    }

    if !has_help {
        return None;
    }

    let cli = Cli::parse_from(&filtered);
    if let Err(msg) = validate(&cli) {
        eprintln!("Error: argument {}.", msg);
        process::exit(2);
    }
    Some(cli_to_opts(&cli))
}
