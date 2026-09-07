# Config file design (lolcat.config.jsonc)

Status: **implemented** (see `src/config.rs`). This document fixes the
structure; follow it when changing the module.

## 1. Goal

A JSONC config file that supplies **defaults for the valued CLI options**,
so a user can pin their favourite rainbow parameters (or default animation
settings) without a shell alias. Decisions made with the user:

- Scope: the config file carries only the **valued (numeric) options**:
  `freq`, `seed`, `angle`, `duration`, `speed`. **No booleans** — switch
  options (`pure`, `invert`, `truecolor`, `keep`, `anchor`, `animate`,
  `force`) are inherently "explicit" on the command line and never come
  from config. Behaviour-only flags (`-h`, `-v`, `-C/--convert`) likewise
  never come from config.
- Precedence (for the keys that exist): **command line > config file >
  built-in defaults**.
- Invalid files: **silent** (details only under `LOLCAT_DEBUG`); granularity
  in §4.
- Auto-generation: when no config is found, write the full default set;
  JSONC so the generated file can carry comments.
- Parser: `jsonc-parser` crate (user choice; no hand-rolled parser, no
  serde). **Trailing commas are rejected** (treated as a syntax error).

## 2. File name, format, discovery

Name: `lolcat.config.jsonc` (JSONC: JSON plus `//` and `/* */` comments;
no trailing commas).

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

`--help` does **not** print the resolved config path; the help text stays
simple. Documentation lives in this repository (see README → this file).

## 3. Auto-generation

When no file exists at any layer, generate one with **every key set to its
built-in default** (`freq`/`seed`/`angle`/`duration`/`speed`), plus a JSONC
comment header explaining each key. Target order:

1. portable layer (executable directory) — the original user requirement
   ("next to the executable; create it if missing");
2. if that directory is not writable, the user layer;
3. if neither is writable, skip silently.

The generated file must round-trip: parsing it yields exactly the built-in
defaults. A fixed `seed` in config gives repeatable output — documented
behaviour, fine.

## 4. Semantics of bad input (silent)

Granularity (user choice: per-key skip, with one structural exception):

- **Syntax error** (the file cannot be parsed as a document at all —
  unbalanced braces, bad string, trailing comma): ignore the **whole file**,
  use defaults. Per-key recovery is impossible when the structure is broken.
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

| Key        | Type    | Default | Notes                                        |
| ---------- | ------- | ------- | -------------------------------------------- |
| `freq`     | number  | 60.0    | finite, > 0 in practice (validated like CLI) |
| `seed`     | integer | 0       | degrees; 0 = random starting hue; a fixed value gives repeatable output |
| `angle`    | number  | 71.6    | finite, `-360..=360`                         |
| `duration` | integer | 3       | ≥ 1                                          |
| `speed`    | number  | 60.0    | finite, ≥ 0.1                                |

No nested objects, no arrays — flat scalar map only. Boolean switches are
deliberately **not** configurable (see §1). Unknown keys are skipped
(see §4), so forward/older-version configs stay usable.

## 6. Integration with the CLI

Three-layer merge inside option construction:

```
effective = cli_arg           // explicitly given on the command line
            .or(config_key)   // from the config file (if any)
            .unwrap_or(builtin_default)
```

Implementation notes:

- `src/cli.rs`: only the five valued options (`freq`/`seed`/`angle`/
  `duration`/`speed`) switch from clap `default_value`s to `Option<T>`, so
  the merge layer can tell "user typed it" from "use config/built-in".
  Boolean flags keep their current clap form. `validate()` runs once on the
  merged result.
- `src/help.rs`: the `-h` rainbow uses the original Ruby's hard-coded
  params unless `-A`/`-F` are on the argv. A `freq`/`angle` coming from the
  **config file** counts as explicit too (the help screen then follows the
  configured look) — the override check reads the merged values, not only
  argv.
- `-C/--convert` and `-v` do not read config.

## 7. Code layout

```
src/config.rs        # new module
  discover() -> Option<PathBuf>     # §2 order, env-aware
  load() -> ConfigDefaults          # silent; §4 semantics
  auto_generate()                   # §3, first writable target
  parse_jsonc(&str) -> …            # jsonc-parser + typed key extraction
  defaults_doc() -> String          # generated file body (keys + comments)
src/main.rs          # call load() before clap; merge; keep -C/-h early paths
src/cli.rs           # Option<T> for the five valued options + merge + validate
```

`ConfigDefaults` mirrors the five valued options, all-`Option` (absent key =
built-in default at merge time), so config never has to spell out every key.

## 8. Windows / portability notes

The binary is otherwise cross-platform: the only `cfg(unix)` code is
`animate::terminal_size` (`ioctl`); on Windows `-a` degrades to plain
painting. The config module must therefore use only `std::env`
(`current_exe`, `var` for `LOLCAT_CONFIG`/`XDG_CONFIG_HOME`/`APPDATA`/
`LOCALAPPDATA`) — no `dirs`-style crate needed, no unix-only calls.
Path handling should be tested with env-var stubs (no real Windows needed).

## 9. Test plan

- JSONC parsing: comments (`//`, `/* */`, inside and between keys), strings
  containing `//` or quotes, **trailing commas rejected**, empty file, `{}`.
- §4 semantics: syntax error (incl. trailing comma) → all defaults; one bad
  key → others apply; unknown key skipped; `LOLCAT_DEBUG` diagnostics
  present only when set.
- §2 order: stub `LOLCAT_CONFIG`, `XDG_CONFIG_HOME`/`HOME`, executable dir;
  first existing wins; env path missing → falls through.
- §3: auto-generation writes to executable dir when writable, user layer
  when not; generated file parses back to built-in defaults.
- §6: CLI flag beats config key beats built-in for each of the five keys;
  merged values pass the same `validate()`; `-h` uses config `freq`/`angle`
  when argv has none.
- Existing suite stays green with a stray config present/absent (tests must
  not read the developer's real config: disable discovery or point
  `LOLCAT_CONFIG` at a fixture).
