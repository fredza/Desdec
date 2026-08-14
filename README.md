# Desdec

Desdec is a lightweight, open-source and educational binary explorer.

The first milestone targets safe, local inspection of ELF, PE and Mach-O
binaries: format detection, section tables, entry points, printable strings and
per-region entropy. It will grow toward x86-64 disassembly, control-flow
analysis, explainable pseudo-code and patch export.

Only analyse and modify binaries you own or are explicitly authorised to study.

## Platform binaries

The `Platform binaries` GitHub Actions workflow builds native debug and release
archives for Windows x86-64 and macOS Apple Silicon (`aarch64`), plus a Linux
x86-64 release archive. Push a version tag beginning with `v` to publish the
three release archives and their SHA-256 checksums on GitHub; the debug archives
remain available as workflow artifacts. Run the workflow manually from the
Actions tab to build artifacts without publishing a release.

- `desdec-windows-x86_64-debug` and `desdec-windows-x86_64-release`
- `desdec-macos-aarch64-debug` and `desdec-macos-aarch64-release`
- `desdec-linux-x86_64-release`

## Run

```sh
cargo run -p desdec-app
cargo run -p desdec-app -- /path/to/binary
```

Drag a binary into the window, choose **Open binary**, or pass one on the
command line.

## Project layout

- `crates/desdec-core`: dependency-free binary inspection and analysis domain.
- `crates/desdec-app`: native `egui` desktop application.
- `docs/`: architecture and collaboration records.

The working agreements shared by human contributors and AI assistants are in
[`docs/ai-collaboration/WORKLOG.md`](docs/ai-collaboration/WORKLOG.md).
