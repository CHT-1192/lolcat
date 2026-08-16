# lolcat

Rainbow coloring effect for text console display — Rust port of [busyloop/lolcat](https://github.com/busyloop/lolcat).

## Building

```bash
$ cargo build --release
```

## Installation

```bash
$ cargo install --path .
```

## Usage

```
$ lolcat [OPTIONS] [FILES]...
```

Concatenate FILE(s), or standard input, to standard output.
With no FILE, or when FILE is `-`, read standard input.

### Options

| Flag | Description |
|---|---|
| `-p`, `--spread <f64>` | Rainbow spread — phase advance per grid unit = 1/spread (default: 1.0) |
| `-F`, `--freq <f64>` | Rainbow frequency (default: 0.1) |
| `-S`, `--seed <i64>` | Rainbow seed, 0 = random (default: 0) |
| `-A`, `--angle <f64>` | Rainbow direction in degrees: 0 = up, clockwise positive (default: 71.6) |
| `-a`, `--animate` | Enable psychedelics |
| `-d`, `--duration <u64>` | Animation duration (default: 12) |
| `-s`, `--speed <f64>` | Animation speed (default: 20.0) |
| `-i`, `--invert` | Invert fg and bg |
| `-t`, `--truecolor` | 24-bit truecolor mode |
| `-f`, `--force` | Force color when stdout is not a tty |
| `-V`, `--version` | Print version and exit |
| `-h`, `--help` | Show help and exit |

### Direction

`--angle` sets the stripe direction: `0` = up (vertical stripes), `90` = right
(horizontal stripes), `180` = down, `270` = left; negative angles go
counterclockwise. Values must be finite and within `-360..=360` (`-360`/`360`
normalize to `0`).

Angles are measured in the character grid: terminal cells are about twice as
tall as they are wide, so diagonal stripes appear steeper on screen than the
given angle. The cardinal directions 0/90/180/270 are exact.

## Examples

```bash
$ echo "hello world" | lolcat
$ fortune | cowsay | lolcat -a
$ lolcat -f file1.txt file2.txt
$ lolcat -A 90 -t -i -p 2.0 -F 0.05 README.md
$ lolcat -A 0    # vertical stripes
$ lolcat -A 180  # stripes reversed
```

## License

BSD 3-Clause — see [LICENSE](LICENSE).
