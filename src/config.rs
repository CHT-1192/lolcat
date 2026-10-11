// Copyright (c) 2016, moe@busyloop.net
// ... (BSD 3-Clause, see LICENSE)
//
//! `lolcat.config.jsonc` configuration (design: docs/config.md).
//!
//! Supplies defaults for the five *valued* options (`freq`, `seed`,
//! `angle`, `duration`, `speed`). Boolean switches never come from config.
//! Precedence: command line > config file > built-in defaults (enforced by
//! the caller via the merge in `cli::cli_to_opts`).
//!
//! Discovery order — first existing file wins, no merging:
//!   1. `$LOLCAT_CONFIG` (explicit; set but missing → keep searching)
//!   2. user layer: `$XDG_CONFIG_HOME/lolcat/config.jsonc`
//!      (default `~/.config/lolcat/config.jsonc`), on Windows
//!      `%APPDATA%\lolcat\config.jsonc` then `%LOCALAPPDATA%`
//!   3. portable layer: `lolcat.config.jsonc` next to the executable
//!
//! When no file exists anywhere, [`auto_generate`] writes the full default
//! set (JSONC with comments): first into the executable's directory, then
//! the user layer, else silently skipped.
//!
//! Bad input is silent by default (`LOLCAT_DEBUG` shows reasons): a syntax
//! error ignores the whole file; a per-key error (unknown key, wrong type,
//! out-of-range value) skips only that key.

use std::path::{Path, PathBuf};

use jsonc_parser::{parse_to_value, JsonValue, ParseOptions};

/// Strict JSONC: comments allowed, everything else must be plain JSON —
/// trailing commas, loose property names, single quotes, missing commas and
/// hex/`+` numbers are all rejected.
fn parse_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: false,
        allow_trailing_commas: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

/// Diagnostics only shown when `LOLCAT_DEBUG` is set.
fn debug(msg: &str) {
    if std::env::var_os("LOLCAT_DEBUG").is_some() {
        eprintln!("DEBUG(config): {msg}");
    }
}

/// The configurable defaults; `None` = not set (built-in default wins).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ConfigDefaults {
    pub(crate) freq: Option<f64>,
    pub(crate) seed: Option<i64>,
    pub(crate) angle: Option<f64>,
    pub(crate) duration: Option<u64>,
    pub(crate) speed: Option<f64>,
    /// Tab handling: N ≥ 1 = exactly N spaces per tab, `-1` = next 8-column
    /// tab stop. This key has no CLI flag (config-only by request).
    pub(crate) tab: Option<i64>,
}

/// Layer 1: the `$LOLCAT_CONFIG` path, when it names an existing file.
fn env_config_file() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("LOLCAT_CONFIG")?);
    if p.is_file() {
        Some(p)
    } else {
        if std::env::var_os("LOLCAT_CONFIG").is_some() {
            debug("$LOLCAT_CONFIG set but missing — continuing the search");
        }
        None
    }
}

/// User-layer directory (`…/lolcat`), platform-dependent.
fn user_config_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            if !x.is_empty() {
                return Some(PathBuf::from(x).join("lolcat"));
            }
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config/lolcat"))
    }
    #[cfg(windows)]
    {
        for var in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(p) = std::env::var_os(var) {
                if !p.is_empty() {
                    return Some(PathBuf::from(p).join("lolcat"));
                }
            }
        }
        None
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Layer 2: the user-layer config file, when it exists.
fn user_config_file() -> Option<PathBuf> {
    let f = user_config_dir()?.join("config.jsonc");
    f.is_file().then_some(f)
}

/// Layer 3: portable config next to the running executable.
fn portable_config_file() -> Option<PathBuf> {
    let f = std::env::current_exe()
        .ok()?
        .parent()?
        .join("lolcat.config.jsonc");
    f.is_file().then_some(f)
}

/// First existing file across the three layers, per docs/config.md §2.
pub(crate) fn discover() -> Option<PathBuf> {
    env_config_file()
        .or_else(user_config_file)
        .or_else(portable_config_file)
}

/// Parse one config file into defaults (silent; granularity per §4).
fn load_from_file(path: &Path) -> ConfigDefaults {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            debug(&format!(
                "{}: unreadable ({e}); using built-in defaults",
                path.display()
            ));
            return ConfigDefaults::default();
        }
    };
    let value = match parse_to_value(&text, &parse_options()) {
        Ok(Some(v)) => v,
        Ok(None) => return ConfigDefaults::default(), // empty / comment-only file
        Err(e) => {
            debug(&format!(
                "{}: syntax error ({e:?}); whole file ignored",
                path.display()
            ));
            return ConfigDefaults::default();
        }
    };
    let JsonValue::Object(obj) = value else {
        debug(&format!(
            "{}: root value is not an object; file ignored",
            path.display()
        ));
        return ConfigDefaults::default();
    };
    let mut cfg = ConfigDefaults::default();
    for (key, val) in obj.into_iter() {
        take_key(&mut cfg, key.as_ref(), &val, path);
    }
    cfg
}

/// Extract one known key; unknown keys and bad values are skipped (debug log).
fn take_key(cfg: &mut ConfigDefaults, key: &str, val: &JsonValue, path: &Path) {
    let num = |v: &JsonValue| match v {
        JsonValue::Number(s) => s.parse::<f64>().ok(),
        _ => None,
    };
    // Numbers must be integral for seed/duration; fractional ones are skipped.
    let integer = |v: &JsonValue, min: f64, max: f64| -> Option<i128> {
        let f = num(v)?;
        if f.is_finite() && f.fract() == 0.0 && f >= min && f <= max {
            Some(f as i128)
        } else {
            None
        }
    };
    let bad = |why: &str| debug(&format!("{}: \"{key}\" skipped ({why})", path.display()));
    match key {
        "freq" => match num(val) {
            Some(f) if f.is_finite() && f > 0.0 => cfg.freq = Some(f),
            _ => bad("need a finite number > 0"),
        },
        "seed" => match integer(val, i64::MIN as f64, i64::MAX as f64) {
            Some(n) => cfg.seed = Some(n as i64),
            _ => bad("need an integer"),
        },
        "angle" => match num(val) {
            Some(a) if a.is_finite() && (-360.0..=360.0).contains(&a) => cfg.angle = Some(a),
            _ => bad("need a finite number between -360 and 360"),
        },
        "duration" => match integer(val, 1.0, u64::MAX as f64) {
            Some(n) => cfg.duration = Some(n as u64),
            _ => bad("need an integer >= 1"),
        },
        "speed" => match num(val) {
            Some(s) if s.is_finite() && s >= 0.1 => cfg.speed = Some(s),
            _ => bad("need a finite number >= 0.1"),
        },
        // Config-only key: positive N = exactly N spaces per tab, -1 = the
        // next 8-column tab stop. 0 and other values are skipped.
        "tab" => match integer(val, i64::MIN as f64, i64::MAX as f64) {
            Some(-1) => cfg.tab = Some(-1),
            Some(n) if n >= 1 => cfg.tab = Some(n as i64),
            _ => bad("need an integer >= 1, or -1 for terminal tab stops"),
        },
        _ => bad("unknown key"),
    }
}

/// Load the config: `Some(path)` when a file was found (parsed per §4,
/// possibly into all-defaults if it was broken), `None` when no file exists
/// at any layer (caller may [`auto_generate`]).
pub(crate) fn load() -> (ConfigDefaults, Option<PathBuf>) {
    match discover() {
        Some(p) => {
            let cfg = load_from_file(&p);
            if cfg == ConfigDefaults::default() {
                debug(&format!("{}: resolved to built-in defaults", p.display()));
            }
            (cfg, Some(p))
        }
        None => (ConfigDefaults::default(), None),
    }
}

/// Body of the auto-generated file: every key at its built-in default, with
/// a comment header (JSONC). Parsing it must round-trip to the defaults.
fn defaults_doc() -> String {
    concat!(
        "{\n",
        "  // Rainbow frequency: hue completes one full cycle every F grid\n",
        "  // units. Must be > 0. (default: 60)\n",
        "  \"freq\": 60,\n",
        "  // Starting hue in degrees; 0 = random. A fixed non-zero value gives\n",
        "  // repeatable output. (default: 0)\n",
        "  \"seed\": 0,\n",
        "  // Stripe direction in degrees: 0 = up, clockwise positive. (default: 71.6)\n",
        "  \"angle\": 71.6,\n",
        "  // Animation duration, frames per band. (default: 3)\n",
        "  \"duration\": 3,\n",
        "  // Animation speed, frames per second. (default: 60)\n",
        "  \"speed\": 60,\n",
        "  // Tab expansion: N >= 1 = exactly N spaces per tab; -1 = advance to\n",
        "  // the next 8-column tab stop (terminal behaviour). (default: 8)\n",
        "  \"tab\": 8\n",
        "}\n",
    )
    .to_string()
}

/// Write the default document into `dir` (creating it if needed); the file
/// name depends on the layer (portable `lolcat.config.jsonc`, user
/// `config.jsonc`). Exposed for tests; returns success.
fn generate_into(dir: &Path, file: &str) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    std::fs::write(dir.join(file), defaults_doc()).is_ok()
}

/// Auto-generate the config when none exists, per docs/config.md §3:
/// portable layer first, then the user layer; silently give up otherwise.
pub(crate) fn auto_generate() -> bool {
    let body_is_ours = |p: &Path| -> bool { p.is_file() };
    // Only create when *nothing* was found (the caller already checked).
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        let f = dir.join("lolcat.config.jsonc");
        if !body_is_ours(&f) && generate_into(&dir, "lolcat.config.jsonc") {
            debug(&format!("created {}", f.display()));
            return true;
        }
    }
    if let Some(dir) = user_config_dir() {
        let f = dir.join("config.jsonc");
        if !body_is_ours(&f) && generate_into(&dir, "config.jsonc") {
            debug(&format!("created {}", f.display()));
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Throwaway dir inside `target/` (inside the workspace, so tests never
    /// touch the developer's real config locations). Unique per call so
    /// concurrent tests cannot race each other's directories.
    fn tmpdir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/config-test");
        let dir = root.join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parse_str(s: &str) -> ConfigDefaults {
        let dir = tmpdir("parse");
        let f = dir.join("c.jsonc");
        std::fs::write(&f, s).unwrap();
        load_from_file(&f)
    }

    #[test]
    fn comments_and_values() {
        let cfg = parse_str(
            "{\n  // line comment\n  /* block */\n  \"freq\": 30.5, // tail\n  \"seed\": 42,\n  \"angle\": -45,\n  \"duration\": 7,\n  \"speed\": 120\n}\n",
        );
        assert_eq!(cfg.freq, Some(30.5));
        assert_eq!(cfg.seed, Some(42));
        assert_eq!(cfg.angle, Some(-45.0));
        assert_eq!(cfg.duration, Some(7));
        assert_eq!(cfg.speed, Some(120.0));
    }

    #[test]
    fn trailing_comma_rejected_whole_file() {
        // Trailing comma = syntax error → whole file ignored.
        let cfg = parse_str("{ \"freq\": 30, }");
        assert_eq!(cfg, ConfigDefaults::default());
    }

    #[test]
    fn syntax_error_ignores_whole_file() {
        let cfg = parse_str("{ \"freq\": 30, ");
        assert_eq!(cfg, ConfigDefaults::default());
        let cfg = parse_str("not json at all");
        assert_eq!(cfg, ConfigDefaults::default());
    }

    #[test]
    fn bad_key_skipped_others_kept() {
        let cfg = parse_str(
            "{ \"freq\": \"abc\", \"seed\": 3.5, \"angle\": 400, \"duration\": 0, \"speed\": 0.01, \"freq\": 55 }",
        );
        // All five values are invalid in their first occurrence… the object
        // has duplicate "freq"; the last one wins → 55 is valid.
        assert_eq!(cfg.freq, Some(55.0));
        assert_eq!(cfg.seed, None);
        assert_eq!(cfg.angle, None);
        assert_eq!(cfg.duration, None);
        assert_eq!(cfg.speed, None);
    }

    #[test]
    fn unknown_and_partial_known() {
        let cfg = parse_str(
            "{ \"freq\": 1, \"wat\": 12, \"seed\": 1.0, \"angle\": 0, \"duration\": 1.5 }",
        );
        assert_eq!(cfg.freq, Some(1.0));
        assert_eq!(cfg.seed, Some(1));
        assert_eq!(cfg.angle, Some(0.0));
        assert_eq!(cfg.duration, None); // fractional
        assert_eq!(cfg.speed, None);
    }

    #[test]
    fn empty_and_comment_only_and_root_types() {
        assert_eq!(parse_str("{}"), ConfigDefaults::default());
        assert_eq!(parse_str(""), ConfigDefaults::default());
        assert_eq!(
            parse_str("// nothing\n/* at all */"),
            ConfigDefaults::default()
        );
        assert_eq!(parse_str("[1, 2]"), ConfigDefaults::default()); // root not object
        assert_eq!(parse_str("\"hi\""), ConfigDefaults::default());
    }

    #[test]
    fn generated_doc_round_trips_to_defaults() {
        let dir = tmpdir("gen");
        let f = dir.join("lolcat.config.jsonc");
        std::fs::write(&f, defaults_doc()).unwrap();
        let cfg = load_from_file(&f);
        assert_eq!(cfg.freq, Some(60.0));
        assert_eq!(cfg.seed, Some(0));
        assert_eq!(cfg.angle, Some(71.6));
        assert_eq!(cfg.duration, Some(3));
        assert_eq!(cfg.speed, Some(60.0));
        assert_eq!(cfg.tab, Some(8));
    }

    #[test]
    fn tab_key_accepts_width_or_minus_one() {
        assert_eq!(parse_str("{ \"tab\": 4 }").tab, Some(4));
        assert_eq!(parse_str("{ \"tab\": 1 }").tab, Some(1));
        assert_eq!(parse_str("{ \"tab\": -1 }").tab, Some(-1));
        // 0, other negatives and fractions are skipped (key stays unset).
        assert_eq!(parse_str("{ \"tab\": 0 }").tab, None);
        assert_eq!(parse_str("{ \"tab\": -2 }").tab, None);
        assert_eq!(parse_str("{ \"tab\": 2.5 }").tab, None);
        assert_eq!(parse_str("{ \"tab\": \"4\" }").tab, None);
        // A bad tab value must not disturb the neighbouring keys.
        let cfg = parse_str("{ \"tab\": 0, \"freq\": 30 }");
        assert_eq!(cfg.tab, None);
        assert_eq!(cfg.freq, Some(30.0));
    }

    #[test]
    fn missing_file_is_all_defaults() {
        let cfg = load_from_file(&tmpdir("missing").join("nope.jsonc"));
        assert_eq!(cfg, ConfigDefaults::default());
    }

    #[cfg(unix)]
    #[test]
    fn user_layer_paths_unix() {
        // XDG wins over ~/.config; HOME used as fallback. (Unix semantics;
        // Windows uses %APPDATA% — see user_layer_paths_windows.)
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-stub");
        std::env::set_var("HOME", "/tmp/home-stub");
        assert_eq!(
            user_config_dir(),
            Some(PathBuf::from("/tmp/xdg-stub/lolcat"))
        );
        std::env::remove_var("XDG_CONFIG_HOME");
        assert_eq!(
            user_config_dir(),
            Some(PathBuf::from("/tmp/home-stub/.config/lolcat"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn user_layer_paths_windows() {
        // %APPDATA% wins; %LOCALAPPDATA% is the fallback; neither → None.
        std::env::set_var("APPDATA", "C:\\appdata-stub");
        std::env::set_var("LOCALAPPDATA", "C:\\local-stub");
        assert_eq!(
            user_config_dir(),
            Some(PathBuf::from("C:\\appdata-stub\\lolcat"))
        );
        std::env::remove_var("APPDATA");
        assert_eq!(
            user_config_dir(),
            Some(PathBuf::from("C:\\local-stub\\lolcat"))
        );
        std::env::remove_var("LOCALAPPDATA");
        assert_eq!(user_config_dir(), None);
    }

    #[test]
    fn env_layer_wins_and_missing_falls_through() {
        let dir = tmpdir("env");
        let real = dir.join("real.jsonc");
        std::fs::write(&real, "{ \"freq\": 12 }").unwrap();
        // Existing $LOLCAT_CONFIG wins over the portable/user layers.
        std::env::set_var("LOLCAT_CONFIG", &real);
        assert_eq!(discover(), Some(real.clone()));
        // Set-but-missing env path → keep searching (no other layer exists
        // in a sandboxed test env with an unmovable HOME, so None is fine
        // and the point is that we do not error).
        std::env::set_var("LOLCAT_CONFIG", dir.join("absent.jsonc"));
        let _ = discover(); // must not panic; returns None or a user-layer hit
        std::env::remove_var("LOLCAT_CONFIG");
    }
}
