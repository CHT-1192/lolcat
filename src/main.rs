// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! lolcat — modern Rust port (clap-derive CLI, standard exits).

mod anchor;
mod animate;
mod ansi;
mod cat;
mod cli;
mod color;
mod engine;
mod help;
mod lol;
mod options;
mod render;
mod sigint;
mod stream;

use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::Path;
use std::process;

use clap::Parser;
use cli::{cli_to_opts, validate, Cli};
use help::{check_help, help_text};
use lol::Engine;
use sigint::{install_ctrlc_handler, ResetGuard};

fn file_error(file: &Path, msg: &str) -> ! {
    eprintln!("lolcat: {}: {}", file.display(), msg);
    process::exit(1);
}

fn main() {
    // --help/-h: strip it, re-parse the rest, and render help text
    // through the colorizer with those flags (equivalent to
    // `echo "help text" | lolcat <other flags>`)
    if let Some(opts) = check_help() {
        let stdout_tty = io::stdout().is_terminal();
        install_ctrlc_handler(stdout_tty, false);
        let text = help_text();
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

    // range validation
    if let Err(msg) = validate(&cli) {
        eprintln!("Error: argument {}.", msg);
        process::exit(2);
    }

    let stdout_tty = io::stdout().is_terminal();
    // With piped stdin, Ctrl-C must let the upstream program's exit
    // sequences drain (see install_ctrlc_handler); otherwise exit at once.
    let drain_on_int = !io::stdin().is_terminal();
    install_ctrlc_handler(stdout_tty, drain_on_int);
    let opts = cli_to_opts(&cli);

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
            // stdout is not a tty and --force is off: plain passthrough.
            // Copy in blocks so newline-less streams still flow.
            if let Err(e) = io::copy(&mut *fd, &mut out) {
                if e.kind() == io::ErrorKind::BrokenPipe {
                    process::exit(1);
                }
                file_error(file, &e.to_string());
            }
        }
    }
    let _ = out.flush();
}
