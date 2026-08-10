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
| `-p`, `--spread <f64>` | Rainbow spread (default: 3.0) |
| `-F`, `--freq <f64>` | Rainbow frequency (default: 0.1) |
| `-S`, `--seed <i64>` | Rainbow seed, 0 = random (default: 0) |
| `-a`, `--animate` | Enable psychedelics |
| `-d`, `--duration <u64>` | Animation duration (default: 12) |
| `-s`, `--speed <f64>` | Animation speed (default: 20.0) |
| `-i`, `--invert` | Invert fg and bg |
| `-t`, `--truecolor` | 24-bit truecolor mode |
| `-f`, `--force` | Force color when stdout is not a tty |
| `-V`, `--version` | Print version and exit |
| `-h`, `--help` | Show help and exit |

## Examples

```bash
$ echo "hello world" | lolcat
$ fortune | cowsay | lolcat -a
$ lolcat -f file1.txt file2.txt
$ lolcat -t -i -p 5.0 -F 0.05 README.md
```

## License

BSD 3-Clause — see [LICENSE](LICENSE).
