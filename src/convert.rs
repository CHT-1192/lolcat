// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! `-C/--convert`: convert between the original lolcat's `-p/--spread` +
//! `-F/--freq` and our `-A/--angle` + `-F/--freq`.
//!
//! Equivalence rule (per-column phase rate and slope are matched):
//!
//! - original → ours: `A = atan(p)` (degrees), `F_ours = 2π·sin(A)/F_orig`
//! - ours → original: `p = tan(A)`, `F_orig = 2π·sin(A)/F_ours`
//!
//! Results are rounded to float32 precision. With no arguments it runs as a
//! small REPL that keeps converting until you quit.

use std::io::{self, BufRead, Write};

const USAGE: &str = "\
Convert between busyloop/lolcat (-p spread -F freq) and CHT-1192/lolcat (-A angle -F freq).

  lolcat -C -p 3 -F 0.1     busyloop → CHT-1192
  lolcat -C -A 71.6 -F 60   CHT-1192 → busyloop
  lolcat -C                 interactive REPL (type 'q' to quit)

REPL input is one of:
  p <spread> F <freq>        busyloop → CHT-1192
  A <angle> F <freq>         CHT-1192 → busyloop
";

pub(crate) fn requested(args: &[String]) -> bool {
    args.iter().any(|a| a == "-C" || a == "--convert")
}

pub(crate) fn run(args: &[String]) {
    let rest: Vec<String> = args
        .iter()
        .skip(1) // argv[0] is the program name
        .filter(|a| **a != "-C" && **a != "--convert")
        .cloned()
        .collect();
    if rest.is_empty() {
        repl();
    } else {
        if let Err(msg) = convert_flags(&rest) {
            eprintln!("{msg}");
            eprintln!("{USAGE}");
            std::process::exit(1);
        }
    }
}

fn is_flag(a: &str, short: &str, long: &str) -> bool {
    a == short
        || a == long
        || a == short.trim_start_matches('-')
        || a.starts_with(&format!("{long}="))
        || a.starts_with(short)
}

fn val_of(args: &[String], shorts: &[&str], longs: &[&str]) -> Result<f64, String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let need_next = shorts
            .iter()
            .chain(longs.iter())
            .any(|s| a == s || a == s.trim_start_matches('-'));
        let inline = longs
            .iter()
            .find_map(|l| a.strip_prefix(&format!("{l}=")))
            .or_else(|| shorts.iter().find_map(|s| a.strip_prefix(s)))
            .or_else(|| shorts.iter().find_map(|s| a.strip_prefix(&format!("{s}="))))
            .filter(|v| !v.is_empty());
        if let Some(v) = inline {
            return v.parse::<f64>().map_err(|_| format!("bad number: {v}"));
        }
        if need_next {
            let v = it
                .next()
                .ok_or_else(|| format!("missing value after {a}"))?;
            return v.parse::<f64>().map_err(|_| format!("bad number: {v}"));
        }
    }
    Err("missing value".into())
}

fn convert_flags(args: &[String]) -> Result<(), String> {
    let has = |short: &str, long: &str| args.iter().any(|a| is_flag(a, short, long));
    if has("-A", "--angle") {
        let a = val_of(args, &["-A"], &["--angle"])?;
        let f = val_of(args, &["-F"], &["--freq"])?;
        let (p, fo) = ours_to_orig(a, f);
        println!("-p {}  -F {}", f32s(p), f32s(fo));
    } else if has("-p", "--spread") {
        let p = val_of(args, &["-p"], &["--spread"])?;
        let f = val_of(args, &["-F"], &["--freq"])?;
        let (a, fo) = orig_to_ours(p, f);
        println!("-A {}  -F {}", f32s(a), f32s(fo));
    } else {
        return Err("use -p ... -F ... (busyloop) or -A ... -F ... (CHT-1192)".into());
    }
    Ok(())
}

/// float32 rounding for output.
fn f32s(v: f64) -> String {
    let f = v as f32;
    if f == f.trunc() && f.abs() < 1e15 {
        format!("{f:.0}")
    } else {
        format!("{f}")
    }
}

/// ours (angle degrees, freq) → original (spread, freq).
fn ours_to_orig(a: f64, f: f64) -> (f64, f64) {
    let rad = a.to_radians();
    (rad.tan(), 2.0 * std::f64::consts::PI * rad.sin() / f)
}

/// original (spread, freq) → ours (angle degrees, freq).
fn orig_to_ours(p: f64, f: f64) -> (f64, f64) {
    let a = p.atan().to_degrees();
    let rad = a.to_radians();
    (a, 2.0 * std::f64::consts::PI * rad.sin() / f)
}

fn repl() {
    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        print!("lolcat-C> ");
        let _ = io::stdout().flush();
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed, "q" | "quit" | "exit") {
            break;
        }
        let toks: Vec<String> = trimmed.split_whitespace().map(String::from).collect();
        if toks.first().map(|s| s.as_str()) == Some("help")
            || toks.first().map(|s| s.as_str()) == Some("h")
        {
            println!("{USAGE}");
            continue;
        }
        match convert_flags(&toks) {
            Ok(()) => {}
            Err(e) => println!("? {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trip() {
        // ours default (71.6, 60) ≈ original default (3.0, 0.1), roughly.
        let (p, f) = ours_to_orig(71.6, 60.0);
        assert!((p - 3.0).abs() < 0.05, "p={p}");
        assert!((f - 0.1).abs() < 0.01, "f={f}");
        let (a2, f2) = orig_to_ours(3.0, 0.1);
        assert!((a2 - 71.565).abs() < 0.05, "a={a2}");
        assert!((f2 - 60.0).abs() < 0.5, "f2={f2}");
        // round-trip stable
        let (p2, _) = ours_to_orig(a2, f2);
        assert!((p2 - 3.0).abs() < 1e-3);
    }

    #[test]
    fn f32_formatting() {
        assert_eq!(f32s(71.565051177078), "71.56505");
        assert_eq!(f32s(60.0), "60");
        assert_eq!(f32s(0.1), "0.1");
    }
}
