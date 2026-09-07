# Config file design (lolcat.config.jsonc)

Status: **planned** — not implemented. This document fixes the structure
before any code is written.

## 1. Goal

A JSONC config file that supplies **defaults for the CLI options**, so a
user can pin their favourite rainbow parameters (or default animation
settings) without a shell alias. Decisions already made with the user:

- Scope: every option that has a CLI long name becomes a possible key
  (`freq`, `seed`, `angle`, `pure`, `invert`, `truecolor`, `keep`,
  `duration`, `speed`, `anchor`, `animate`, `force`). Behaviour-only flags
  (`-h`, `-v`, `-C/--convert`) never come from config.
- Precedence: **command line > config file > built-in defaults**.
- Invalid files: **silent** (details only under `LOLCAT_DEBUG`); granularity
  below.
- Auto-generation: when no config is found, write the full default set;
  JSONC so the generated file can carry comments.
- Parser: `jsonc-parser` crate (user choice; no hand-rolled parser, no
  serde).

## 2. File name, format, discovery

Name: `lolcat.config.jsonc` (JSONC: JSON plus `//` and `/* */` comments).

Discovery order — **first file that exists wins** (single-file semantics,
no merging):

1. `$LOLCAT_CONFIG` — explicit path. If set but missing, keep searching
   (silent; logged under `LOLCAT_DEBUG`).
2. User layer:
   - macOS/Linux: `$XDG_CONFIG_HOME/lolcat/config.jsonc`, defaulting to
     `~/.config/lolcat/config.jsonc`.
   - Windows: `%APPDATA%\lolcat\config.jsonc`, falling back to
     `%LOCALAPPDATA%\lolcat\config.jsonc`.
3. Portable layer: `lolcat.config.jsonc` in the directory of the running
   executable (`std::env::current_exe()` parent).

The executable directory is resolved at runtime, so this layer travels with
the binary (e.g. the user's `~/.local/bin/lolcat` install).

## 3. Auto-generation

When no file exists at any layer, generate one with **every key set to its
built-in default**, plus a JSONC comment header explaining each key. Target
order:

1. portable layer (executable directory) — the original user requirement
   ("next to the executable; create it if missing");
2. if that directory is not writable, the user layer;
3. if neither is writable, skip silently.

The generated file must round-trip: parsing it yields exactly the built-in
defaults.

## 4. Semantics of bad input (silent)

Granularity (user choice: per-key skip, with one structural exception):

- **Syntax error** (the file cannot be parsed as a document at all, e.g.
  unbalanced braces, bad string): ignore the **whole file**, use defaults.
  Per-key recovery is impossible when the structure is broken.
- **Per-key errors**: unknown keys, wrong type, or a value that fails the
  same validation as the CLI (`validate()`: finite numbers, angle within
  `-360..=360`, `speed >= 0.1`, …) — skip **that key only**; all other keys
  take effect.
- All of the above are silent on stderr in normal runs; with `LOLCAT_DEBUG`
  set, print file path + reason for every skip.

Rationale for per-key skips: a typo like `"fre": 3` must not nuke the rest
of a valid config, but a broken `{` gives no place to attach individual
errors.

## 5. Keys

| Key | Type | Default | Notes |
|---|---|---|---|
| `freq` | number | 60.0 | finite, > 0 in practice (validated like CLI) |
| `seed` | integer | 0 | degrees; 0 = random starting hue |
| `angle` | number | 71.6 | finite, `-360..=360` |
| `duration` | integer | 3 | ≥ 1 |
| `speed` | number | 60.0 | finite, ≥ 0.1 |
| `pure` | bool | false | |
| `invert` | bool | false | |
| `truecolor` | bool | false | |
| `keep` | bool | true | |
| `anchor` | bool | false | |
| `animate` | bool | false | |
| `force` | bool | false | |

No nested objects, no arrays — flat scalar map only. Unknown keys are
skipped (see §4), so forward/older-version configs stay usable.

## 6. Integration with the CLI

Three-layer merge inside option construction:

```
effective = cli_arg           // explicitly given on the command line
            .or(config_key)   // from the config file (if any)
            .unwrap_or(builtin_default)
```

Implementation notes:

- `src/cli.rs`: the `Cli` fields currently carry clap `default_value`s. To
  know whether the user actually typed an option, switch fields to
  `Option<T>` (no clap defaults) and let the merge layer above supply the
  effective value. `validate()` runs once on the merged result.
- `src/help.rs`: the `-h` rainbow uses the original Ruby's hard-coded
  params unless `-A`/`-F` are on the argv. A `freq`/`angle` coming from the
  **config file** counts as explicit too (the help screen then follows the
  configured look) — the override check reads the merged values, not only
  argv.
- `-C/--convert` and `-v` do not read config.
- `-h` help text: one line documenting `lolcat.config.jsonc` + `$LOLCAT_CONFIG`.

## 7. Code layout

```
src/config.rs        # new module
  discover() -> Option<PathBuf>     # §2 order, env-aware
  load() -> ConfigDefaults          # silent; §4 semantics
  auto_generate()                   # §3, first writable target
  parse_jsonc(&str) -> …            # jsonc-parser + typed key extraction
  defaults_doc() -> String          # generated file body (keys + comments)
src/main.rs          # call load() before clap; merge; keep -C/-h early paths
src/cli.rs           # Option<T> fields + merge + single validate() pass
```

`ConfigDefaults` mirrors `Options` but all-`Option` (absent key = built-in
default at merge time), so config never has to spell out every key.

## 8. Windows / portability notes

The binary is otherwise cross-platform: the only `cfg(unix)` code is
`animate::terminal_size` (`ioctl`); on Windows `-a` degrades to plain
painting. The config module must therefore use only `std::env`
(`current_exe`, `var` for `LOLCAT_CONFIG`/`XDG_CONFIG_HOME`/`APPDATA`/
`LOCALAPPDATA`) — no `dirs`-style crate needed, no unix-only calls.
Path handling should be tested with env-var stubs (no real Windows needed).

## 9. Test plan

- JSONC parsing: comments (`//`, `/* */`, inside and between keys), strings
  containing `//` or quotes, trailing commas (accept or reject — decide),
  empty file, `{}`.
- §4 semantics: syntax error → all defaults; one bad key → others apply;
  unknown key skipped; `LOLCAT_DEBUG` diagnostics present only when set.
- §2 order: stub `LOLCAT_CONFIG`, `XDG_CONFIG_HOME`/`HOME`, executable dir;
  first existing wins; env path missing → falls through.
- §3: auto-generation writes to executable dir when writable, user layer
  when not; generated file parses back to built-in defaults.
- §6: CLI flag beats config key beats built-in for every key; merged values
  pass the same `validate()`; `-h` uses config `freq`/`angle` when argv has
  none.
- Existing suite stays green with a stray config present/absent
  (tests must not read the developer's real config: disable discovery or
  point `LOLCAT_CONFIG` at a fixture).

## 10. Open questions / deferred

- Trailing commas in JSONC: accept or reject?
- `duration`/`speed`/`animate` in config: should `lolcat -a` (flag, no
  value) still be required to enable animation even if config says
  `"animate": true`? (Yes — config only supplies defaults for *valued*
  options' values; a boolean *switch* on the CLI is inherently "explicit".)
- Help text wording and whether `--help` should list the config path found.
- Watch out for `seed`/randomness: config `seed` fixed → repeatable output;
  fine, but document.
