// Copyright (c) 2016, moe@busyloop.net
// ... BSD 3-Clause (see LICENSE)
//
//! lolcat — byte-faithful Rust port (optimist 3.0.x compatible CLI).

mod lol;

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufWriter, IsTerminal, Write};
use std::process;

use lol::{Engine, Options};

const VERSION_LINE: &str = "lolcat 100.0.1 (c)2011 moe@busyloop.net";

const HELP_HEADER: &str = "\nUsage: lolcat [OPTION]... [FILE]...\n\n\
Concatenate FILE(s), or standard input, to standard output.\n\
With no FILE, or when FILE is -, read standard input.\n\n";

const HELP_FOOTER: &str = "\nExamples:\n\
  lolcat f - g      Output f's contents, then stdin, then g's contents.\n\
  lolcat            Copy standard input to standard output.\n\
  fortune | lolcat  Display a rainbow cookie.\n\n\
Report lolcat bugs to <https://github.com/busyloop/lolcat/issues>\n\
lolcat home page: <https://github.com/busyloop/lolcat/>\n\
Report lolcat translation bugs to <http://speaklolcat.com/>\n";

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

#[derive(Clone, Copy, PartialEq)]
enum Kind { Flag, Float, Int }

struct Spec { name: &'static str, short: char, kind: Kind, default: Option<f64> }

const SPECS: [Spec; 11] = [
    Spec { name: "spread", short: 'p', kind: Kind::Float, default: Some(3.0) },
    Spec { name: "freq", short: 'F', kind: Kind::Float, default: Some(0.1) },
    Spec { name: "seed", short: 'S', kind: Kind::Int, default: Some(0.0) },
    Spec { name: "animate", short: 'a', kind: Kind::Flag, default: None },
    Spec { name: "duration", short: 'd', kind: Kind::Int, default: Some(12.0) },
    Spec { name: "speed", short: 's', kind: Kind::Float, default: Some(20.0) },
    Spec { name: "invert", short: 'i', kind: Kind::Flag, default: None },
    Spec { name: "truecolor", short: 't', kind: Kind::Flag, default: None },
    Spec { name: "force", short: 'f', kind: Kind::Flag, default: None },
    Spec { name: "version", short: 'v', kind: Kind::Flag, default: None },
    Spec { name: "help", short: 'h', kind: Kind::Flag, default: None },
];

fn lookup_long(n: &str) -> Option<&'static Spec> { SPECS.iter().find(|s| s.name == n) }
fn lookup_short(c: char) -> Option<&'static Spec> { SPECS.iter().find(|s| s.short == c) }

fn int_re(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'_')
}

fn ruby_to_i(s: &str) -> i64 {
    let neg = s.starts_with('-');
    let digits: String = s.trim_start_matches('-').chars().filter(|c| *c != '_').collect();
    if digits.is_empty() { return 0; }
    match digits.parse::<i64>() { Ok(v) => if neg { -v } else { v }, Err(_) => if neg { i64::MIN } else { i64::MAX } }
}

fn float_re(s: &str) -> bool {
    let b = s.as_bytes(); let mut i = 0;
    let digits = |b: &[u8], i: &mut usize| { let mut d = 0; while *i < b.len() && b[*i].is_ascii_digit() { d += 1; *i += 1; } d > 0 };
    if i < b.len() && b[i] == b'-' { i += 1; }
    let mut int_digits = 0;
    while i < b.len() && b[i].is_ascii_digit() { int_digits += 1; i += 1; }
    if int_digits > 0 {
        if i < b.len() && b[i] == b'.' { i += 1; if !digits(b, &mut i) { return false; } }
    } else { if i >= b.len() || b[i] != b'.' { return false; } i += 1; if !digits(b, &mut i) { return false; } }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1; if i < b.len() && (b[i] == b'+' || b[i] == b'-') { i += 1; }
        if !digits(b, &mut i) { return false; }
    }
    i == b.len()
}

fn is_param(s: &str) -> bool {
    if s.len() < 2 || !s.starts_with('-') { return true; }
    let c = s.chars().nth(1).unwrap();
    if c == '-' { return false; }
    if c == '.' && s.len() == 2 { return false; }
    if !c.is_ascii_digit() && c != '.' { return false; }
    true
}

fn collect_params(args: &[String], start: usize) -> Vec<String> {
    let mut p = Vec::new(); let mut pos = start;
    while pos < args.len() && is_param(&args[pos]) { p.push(args[pos].clone()); pos += 1; }
    p
}

struct ParseState { given: HashSet<&'static str>, params: HashMap<&'static str, String>, negated: HashSet<&'static str> }

fn yield_arg(arg: &str, params: &[String], st: &mut ParseState) -> Result<usize, String> {
    let (a, neg) = match arg.strip_prefix("--no-") {
        Some(rest) if !rest.is_empty() && !rest.contains('-') => (format!("--{}", rest), true),
        _ => (arg.to_string(), false),
    };
    let sym = if let Some(name) = a.strip_prefix("--") {
        if name.is_empty() || name.contains('-') { return Err(format!("invalid argument syntax: '{}'", a)); }
        let spec = lookup_long(name).or_else(|| lookup_long(&format!("no-{}", name)));
        if a.contains("--no-") { None } else { spec }
    } else if a.len() == 2 && a.starts_with('-') { lookup_short(a.chars().nth(1).unwrap()) }
    else { return Err(format!("invalid argument syntax: '{}'", a)) };

    let Some(spec) = sym else { return Err(format!("unknown argument '{}'", arg)) };
    if st.given.contains(spec.name) { return Err(format!("option '{}' specified multiple times", arg)); }
    let taken = if !params.is_empty() && spec.kind != Kind::Flag { st.params.insert(spec.name, params[0].clone()); 1 } else { 0 };
    st.given.insert(spec.name);
    if neg { st.negated.insert(spec.name); }
    Ok(taken)
}

enum Outcome { Run(Options, Vec<String>), Help, Version, Die(String) }

fn parse(args: &[String]) -> Outcome {
    let mut st = ParseState { given: HashSet::new(), params: HashMap::new(), negated: HashSet::new() };
    let mut leftovers = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" { leftovers.extend(args[i + 1..].iter().cloned()); break; }
        if let Some(rest) = args[i].strip_prefix("--") {
            if let Some((name, val)) = rest.split_once('=') {
                if !name.is_empty() {
                    if let Err(e) = yield_arg(&format!("--{}", name), &[val.to_string()], &mut st) { return Outcome::Die(e); }
                    i += 1; continue;
                }
            }
            let params = collect_params(args, i + 1);
            match yield_arg(&args[i], &params, &mut st) { Ok(t) => i += t + 1, Err(e) => return Outcome::Die(e) }
        } else if args[i].starts_with('-') && args[i].len() >= 2 {
            let shorts: Vec<char> = args[i][1..].chars().collect();
            for (j, &c) in shorts.iter().enumerate() {
                let sa = format!("-{}", c);
                if j == shorts.len() - 1 { let params = collect_params(args, i + 1); match yield_arg(&sa, &params, &mut st) { Ok(t) => i += t, Err(e) => return Outcome::Die(e) } }
                else if let Err(e) = yield_arg(&sa, &[], &mut st) { return Outcome::Die(e); }
            }
            i += 1;
        } else { leftovers.push(args[i].clone()); i += 1; }
    }
    if st.given.contains("version") { return Outcome::Version; }
    if st.given.contains("help") { return Outcome::Help; }
    let mut o = Options::defaults();
    for spec in &SPECS {
        match spec.kind {
            Kind::Flag => if st.given.contains(spec.name) {
                let v = !st.negated.contains(spec.name);
                match spec.name { "animate" => o.animate = v, "invert" => o.invert = v, "truecolor" => o.truecolor = v, "force" => o.force = v, _ => {} }
            },
            Kind::Float => {
                let val = match st.params.get(spec.name) {
                    Some(p) => { if !float_re(p) { return Outcome::Die(format!("option '{}' needs a floating-point number", spec.name)); } p.parse().unwrap() }
                    None => spec.default.unwrap(),
                };
                match spec.name { "spread" => o.spread = val, "freq" => o.freq = val, "speed" => o.speed = val, _ => {} }
            },
            Kind::Int => {
                let val = match st.params.get(spec.name) {
                    Some(p) => { if !int_re(p) { return Outcome::Die(format!("option '{}' needs an integer", spec.name)); } ruby_to_i(p) }
                    None => spec.default.unwrap() as i64,
                };
                match spec.name { "seed" => o.seed = val, "duration" => o.duration = val as i32, _ => {} }
            },
        }
    }
    Outcome::Run(o, leftovers)
}

fn die(msg: &str) -> ! { eprintln!("Error: {}.", msg); eprintln!("Try --help for help."); process::exit(-1); }

struct ResetGuard { tty: bool }
impl Drop for ResetGuard { fn drop(&mut self) { if self.tty { let mut out = io::stdout(); let _ = out.write_all(b"\x1b[m\x1b[?25h\x1b[?1;5;2004l"); let _ = out.flush(); } } }

fn install_ctrlc_handler(tty: bool) {
    let _ = ctrlc::set_handler(move || {
        if tty { let mut out = io::stdout(); let _ = out.write_all(b"\x1b[m\x1b[?25h\x1b[?1;5;2004l"); let _ = out.flush(); }
        process::exit(0);
    });
}

fn render_help(out: &mut dyn Write) -> ! {
    let opts = Options::for_help();
    let mut eng = Engine::new();
    { let _guard = ResetGuard { tty: io::stdout().is_terminal() };
      let text = help_text(); let mut input = text.as_bytes();
      let _ = lol::cat(&mut input, &opts, &mut eng, out);
      let _ = out.write_all(b"\n"); let _ = out.flush(); }
    process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdout_tty = io::stdout().is_terminal();
    let stdin_tty = io::stdin().is_terminal();
    install_ctrlc_handler(stdout_tty);
    match parse(&args) {
        Outcome::Die(msg) => die(&msg),
        Outcome::Version => { let mut out = io::stdout(); let _ = writeln!(out, "{}", VERSION_LINE); let _ = out.flush(); process::exit(0); }
        Outcome::Help => { render_help(&mut io::stdout()); }
        Outcome::Run(mut opts, files) => {
            if opts.spread < 0.1 { die("argument --spread must be >= 0.1"); }
            if (opts.duration as f64) < 0.1 { die("argument --duration must be >= 0.1"); }
            if opts.speed < 0.1 { die("argument --speed must be >= 0.1"); }
            opts.os = opts.seed as f64;
            if opts.os == 0.0 { opts.os = rand::random::<u8>() as f64; }
            let files: Vec<String> = if files.is_empty() { vec!["-".to_string()] } else { files };
            let mut eng = Engine::new();
            let mut out = BufWriter::new(io::stdout());
            for file in &files {
                let mut fd: Box<dyn io::Read> = if file == "-" { Box::new(io::stdin()) }
                else {
                    match std::fs::File::open(file) { Ok(f) => Box::new(f), Err(e) => {
                        let msg = match e.kind() { io::ErrorKind::NotFound => "No such file or directory",
                            io::ErrorKind::PermissionDenied => "Permission denied", io::ErrorKind::IsADirectory => "Is a directory",
                            _ => "Is not a regular file" };
                        let _ = writeln!(out, "lolcat: {}: {}", file, msg); let _ = out.flush(); process::exit(1); } }
                };
                if stdout_tty || opts.force {
                    let _guard = ResetGuard { tty: stdout_tty };
                    if let Err(e) = lol::cat(&mut *fd, &opts, &mut eng, &mut out) {
                        if e.kind() == io::ErrorKind::BrokenPipe { process::exit(1); }
                        let _ = writeln!(out, "lolcat: {}", e); process::exit(1); }
                } else {
                    let result: io::Result<()> = if stdin_tty {
                        let mut line = String::new(); let mut reader = io::BufReader::new(&mut *fd);
                        loop { line.clear(); match reader.read_line(&mut line) { Ok(0) => break, Ok(_) => { if let Err(e) = out.write_all(line.as_bytes()) { if e.kind() == io::ErrorKind::BrokenPipe { process::exit(1); } process::exit(1); } } Err(_) => break, } }
                        Ok(())
                    } else { io::copy(&mut *fd, &mut out).map(|_| ()) };
                    if let Err(e) = result { if e.kind() == io::ErrorKind::BrokenPipe { process::exit(1); } let _ = writeln!(out, "lolcat: {}", e); process::exit(1); }
                }
            }
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> (Options, Vec<String>) {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match parse(&v) { Outcome::Run(o, f) => (o, f), _ => panic!("expected Run") }
    }
    fn parse_die(args: &[&str]) -> String {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match parse(&v) { Outcome::Die(m) => m, _ => panic!("expected Die") }
    }

    #[test] fn defaults() { let (o, f) = parse_ok(&[]); assert_eq!(o.spread, 3.0); assert_eq!(o.freq, 0.1); assert_eq!(o.seed, 0); assert!(f.is_empty()); }
    #[test] fn long_short_values() { let (o, _) = parse_ok(&["--spread", "2", "-F", "0.5", "-S", "42"]); assert_eq!(o.spread, 2.0); assert_eq!(o.freq, 0.5); assert_eq!(o.seed, 42); }
    #[test] fn equals_form() { let (o, _) = parse_ok(&["--spread=4.5", "--seed=7"]); assert_eq!(o.spread, 4.5); assert_eq!(o.seed, 7); }
    #[test] fn bundled_flags() { let (o, _) = parse_ok(&["-ai"]); assert!(o.animate && o.invert); }
    #[test] fn no_form() { let (o, _) = parse_ok(&["--no-animate"]); assert!(!o.animate); }
    #[test] fn short_with_attached() { let (o, f) = parse_ok(&["-ap", "3"]); assert!(o.animate); assert_eq!(o.spread, 3.0); assert!(f.is_empty()); }
    #[test] fn flag_then_file() { let (o, files) = parse_ok(&["-a", "f.txt"]); assert!(o.animate); assert_eq!(files, vec!["f.txt"]); }
    #[test] fn dash_is_stdin() { let (_, f) = parse_ok(&["-"]); assert_eq!(f, vec!["-"]); }
    #[test] fn dashdash() { let (o, f) = parse_ok(&["--", "-a", "--spread"]); assert!(!o.animate); assert_eq!(o.spread, 3.0); assert_eq!(f, vec!["-a", "--spread"]); }
    #[test] fn neg_num_is_param() { let (o, _) = parse_ok(&["--spread", "-3"]); assert_eq!(o.spread, -3.0); }
    #[test] fn option_as_not_param() { let (o, f) = parse_ok(&["--spread", "-a"]); assert_eq!(o.spread, 3.0); assert!(o.animate); assert!(f.is_empty()); }
    #[test] fn float_re_test() { assert!(float_re("3.0")); assert!(float_re(".5")); assert!(float_re("-1e2")); assert!(float_re("1E+5")); assert!(!float_re("1.")); assert!(!float_re("")); assert!(!float_re("abc")); }
    #[test] fn int_re_test() { assert!(int_re("12")); assert!(int_re("-3")); assert!(int_re("1_5")); assert!(!int_re("1.5")); assert!(!int_re("")); }
    #[test] fn ruby_to_i_test() { assert_eq!(ruby_to_i("1_5"), 15); assert_eq!(ruby_to_i("_"), 0); assert_eq!(ruby_to_i("-12"), -12); }
    #[test] fn unknown_long() { assert_eq!(parse_die(&["--bogus"]), "unknown argument '--bogus'"); }
    #[test] fn no_prefix_match() { assert_eq!(parse_die(&["--anim"]), "unknown argument '--anim'"); }
    #[test] fn unknown_short_bundle() { assert_eq!(parse_die(&["-az"]), "unknown argument '-z'"); }
    #[test] fn invalid_syntax() { assert_eq!(parse_die(&["--foo-bar"]), "invalid argument syntax: '--foo-bar'"); }
    #[test] fn multiple() { assert_eq!(parse_die(&["-p", "1", "--spread", "2"]), "option '--spread' specified multiple times"); }
    #[test] fn bad_float() { assert_eq!(parse_die(&["--spread=abc"]), "option 'spread' needs a floating-point number"); }
    #[test] fn bad_int() { assert_eq!(parse_die(&["--seed=1.5"]), "option 'seed' needs an integer"); }
    #[test] fn help_wins() { let v: Vec<String> = vec!["--help".into(), "--spread=abc".into()]; assert!(matches!(parse(&v), Outcome::Help)); }
    #[test] fn version_wins() { let v: Vec<String> = vec!["-v".into(), "-h".into()]; assert!(matches!(parse(&v), Outcome::Version)); }
    #[test] fn unknown_beats_help() { assert_eq!(parse_die(&["-h", "--bogus"]), "unknown argument '--bogus'"); }
    #[test] fn is_param_test() { assert!(is_param("3")); assert!(is_param("-3")); assert!(!is_param("-a")); assert!(!is_param("--foo")); assert!(!is_param("-.")); }
    #[test] fn help_text_structure() {
        let h = help_text();
        assert!(h.starts_with("\nUsage: lolcat [OPTION]... [FILE]...\n\n"));
        assert!(h.contains("Report lolcat translation bugs to <http://speaklolcat.com/>\n\n"));
        assert!(h.ends_with("\n\n"));
        assert!(!h.contains("(c)2011"));
        for (spec, desc) in &HELP_OPTIONS {
            let line = format!("  {:<18}    {}\n", spec, desc);
            assert!(h.contains(&line), "missing: {:?}", line);
        }
    }
}
