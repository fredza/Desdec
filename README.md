# Desdec

Desdec is a lightweight, open-source and educational binary explorer.

The first milestone targets safe, local inspection of ELF, PE and Mach-O
binaries. It starts with format detection, then will grow toward x86-64
disassembly, control-flow analysis, explainable pseudo-code and patch export.

Only analyse and modify binaries you own or are explicitly authorised to study.

## Platform binaries

The `Platform binaries` GitHub Actions workflow builds native debug and release
archives for Windows x86-64 and macOS Apple Silicon (`aarch64`). Run it manually
from the Actions tab, or push a version tag beginning with `v`. The four
archives are available as workflow artifacts:

- `desdec-windows-x86_64-debug` and `desdec-windows-x86_64-release`
- `desdec-macos-aarch64-debug` and `desdec-macos-aarch64-release`

## Run

```sh
cargo run -p desdec-app
```

Drag a binary into the window or choose **Open binary**.

## Project layout

- `crates/desdec-core`: dependency-free binary inspection and analysis domain.
- `crates/desdec-app`: native `egui` desktop application.
- `docs/`: architecture and collaboration records.

The working agreements shared by human contributors and AI assistants are in
[`docs/ai-collaboration/WORKLOG.md`](docs/ai-collaboration/WORKLOG.md).
