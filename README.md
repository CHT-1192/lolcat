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
| `-F`, `--freq <f64>` | Hue cycles once every F grid units (default: 60) |
| `-S`, `--seed <i64>` | Rainbow seed: starting hue in degrees, 0 = random (default: 0) |
| `-A`, `--angle <f64>` | Direction: 0 = up, clockwise positive (default: 71.6) |
| `-B`, `--anchor` | Color by fixed screen position (stable for TUIs) |
| `-a`, `--animate` | Enable psychedelics |
| `-d`, `--duration <u64>` | Animation duration (default: 12) |
| `-s`, `--speed <f64>` | Animation speed (default: 20.0) |
| `-i`, `--invert` | Invert fg and bg |
| `-t`, `--truecolor` | 24-bit truecolor mode |
| `-P`, `--pure` | Pure saturated hue wheel (default: classic pastel) |
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

### Frequency

The hue rotates one full revolution per `F` grid units along the stripe
direction. At `-A 0` that is one rainbow cycle every `F` characters; at
`-A 90` it is every `F` lines. The default `F = 60` matches the classic
lolcat band width. `-S` sets the starting hue (degrees mod 360; random
when `0`).

By default colors use the original lolcat sine mapping (soft pastel);
`-P` switches to a saturated hue wheel where hue 0° = pure red
(`-P -A 0 -F 5` → ff0000, back to red on the 6th character).

## Examples

```bash
$ echo "hello world" | lolcat
$ fortune | cowsay | lolcat -a
$ cmatrix | lolcat               # matrix rain, in rainbow
$ pipes.sh -p 10 | lolcat        # animated pipes, rainbow
$ btop | lolcat -B               # live monitor; -B pins colours to screen cells
$ lolcat -f file1.txt file2.txt
$ lolcat -A 90 -t -i -F 30 README.md
$ lolcat -P -A 0 -F 5   # pure hues, one rainbow cycle per 5 characters
$ lolcat -A 180         # stripes reversed
```

## License

BSD 3-Clause — see [LICENSE](LICENSE).
