# Desdec

**English** · [Français](README.fr.md) · [Español](README.es.md)

Release and pre-release versions are signed with a private key; this is currently required.
The public key is distributed free of charge with the binary.

Desdec is a local, open-source binary explorer for reading executables you are
allowed to read. It opens an ELF, PE or Mach-O file, tells you what is inside
it, and never runs it on your machine.

Where it does run a binary, it runs it on a processor it builds itself — an
emulator with no operating system behind it, described under **Machine**
below. No byte of the file ever reaches your own processor.

Its guiding rule is that the tool must not invent anything. Where an answer is
exact — the address an operand designates, the bytes a patch would write — it
is given plainly. Where it is a reading that a branch could invalidate, it says
so. Where it does not know, it says that instead of guessing.

> Only analyse and modify binaries you own or are explicitly authorised to
> study.

![The disassembly view: the listing, the condition flags of the selected instruction in the bar, and the local pseudo-code beside it](docs/screenshots/disassembly.png)

## What it shows

| View | What you get |
| --- | --- |
| **Overview** | Format, architecture, entry point, SHA-256, entropy, hardening (RELRO, canary, NX, PIE, CFG), detected source language, and every linked library — each with an explanation of what it is for. |
| **Segments** | The section table: addresses, sizes, permissions and per-section entropy, so a packed or encrypted region stands out. |
| **Functions** | Named functions with their bodies, basic blocks and a local control-flow graph. One click on a row opens that function's code in the listing. A file that names none of them still has this view: its functions are found from its own code — the entry point, everything something calls, compiler prologues — and each row says which of those it came from, because an address something calls is a fact and a prologue is a reading. |
| **Strings** | Printable strings with their offsets and encodings, filterable, with the instructions that reference them. |
| **Disassembly** | x86, x86-64 (iced-x86) and AArch64 (Capstone) listings, with per-instruction byte editing. Right-click an instruction to be told what its operand designates and what last wrote each register it names. The bar carries the condition flags of the selected row — which it settles, which it consults, and which of them the bytes settle to a value known whatever ran before — and a row you have written about is marked in the margin. |
| **Pseudo-code** | A conservative, built-in translation of the decoded flow — or the output of Rizin/rz-ghidra or RetDec when one is installed and chosen. |
| **Machine** | An emulated processor, off until you ask for one. Registers, memory, a stack, breakpoints, watchpoints, step into/over/out, run to cursor, and a call stack — all of them measurements, because something really ran. It runs on a processor Desdec builds, never on yours: no byte of the file reaches your machine's processor, and a system call, a library that is not open or an instruction that is not emulated stops the run and is named rather than guessed past. x86 and x86-64. Breakpoints carry conditions (`rcx == 4`, `[rdi]:1 != 0`) and pass counts, so one inside a loop of ten thousand turns is worth setting. And the run goes **backwards**: the state before each instruction is kept, so a step back restores it exactly — including out of a fault, which a debugger attached to a process cannot do at all. |
| **Patches** | Pending byte edits, and the export that writes them to a **copy**. The analysed file is never modified. |
| **Updates** | Optional, and off until you say otherwise. Desdec can ask GitHub whether a newer release exists; the question is put once, and its answers are "yes" and "not this time" — turning it off for good is in the preferences. A download is hashed against the `.sha256` the release publishes and refused if it does not match. Desdec never replaces itself: the archive lands in a folder and you open it when you want to. |
| **YARA** | Optional. Runs a locally installed `yara` or `yr` against the open file with rules you provide. Off by default. |
| **AI assistance** | Optional, off by default. A model reads back what was decoded — a whole binary, a function, one instruction — and its answer is labelled a proposed reading, never a finding. A local model (Ollama) or Anthropic's API, whichever you configure. |
| **Script** | The reader's own rule, written once and run over the whole file: name every function longer than a page, mark every call into a library, find what a listing will not scroll to. It runs in a sandbox that has no file system, no network and no processes — only the analysis it was handed. |
| **Plugins** | A script someone else wrote, installed as a directory with a manifest. Its manifest *asks* for permissions — write notes, move the listing, propose patches — and the list is put in front of you before anything is enabled. A plugin that was never enabled has never run. |

Everything is available in French, English and Spanish, from a command palette
(`Ctrl+Shift+P`) with rebindable shortcuts.

## Screenshots

The interface is in French here; English and Spanish are one preference away.

**Before a file is opened.** The menu keeps the recent files and the views;
the action bar stays available whether the menu is open or shut.

![The empty state, with the navigation menu open](docs/screenshots/start.png)

**Functions.** Named functions with their size and block count, the local
control-flow graph of the selected one, and its pseudo-code underneath. The
arrow at the head of a row — or the button beside the address — opens that
function's code in the listing.

![The Functions view: the function list, a control-flow graph and pseudo-code](docs/screenshots/functions.png)

**Strings.** Every printable string with its offset and encoding, filterable,
and narrowed down to those that are unmapped or never referenced.

![The Strings view, with its filter and its two narrowing toggles](docs/screenshots/strings.png)

**External decompiler.** Rizin with rz-ghidra, or RetDec, when one is
installed and chosen — the engine that produced the text is always named, and
the matching assembly is one button away.

![Pseudo-code produced by rizin and rz-ghidra, with the engine named above it](docs/screenshots/decompile.png)

**Patches.** Byte edits wait here until you export them, and the export writes
a copy: the analysed file is never modified.

![The Patches view, with one pending edit and the export that writes a copy](docs/screenshots/patches.png)

**Command palette** (`Ctrl+Shift+P`). Every command, its shortcut, and the
recently opened files, in one searchable list.

![The command palette, listing commands with their shortcuts](docs/screenshots/command-palette.png)

**Preferences.** External engines are found on the `PATH` or given a path of
your own, and are only ever started once you select one.

![The Preferences window, on its Decompiler tab](docs/screenshots/preferences.png)

## Install and run

Requires Rust 1.85 or newer.

```sh
git clone https://github.com/fredza/Desdec.git
cd Desdec
cargo run --release -p desdec-app            # open the window
cargo run --release -p desdec-app -- /bin/ls # or analyse a file straight away
```

You can also drag a binary onto the window, or use **Open binary** (`Ctrl+O`).

Prebuilt archives for Windows x86-64, macOS Apple Silicon and Linux x86-64 are
published by the `Platform binaries` workflow on every tag beginning with `v`,
together with their SHA-256 checksums.

### Checking a published release

Every archive is signed by **Frédéric Zawalski @2026 bdom**, with the key
`C9A3 1D07 46E0 65C4 E2EA  33F6 08FA 1D81 8A91 F329`. The public key travels
with the binaries: it is attached to every release as
`desdec-signing-key.asc`, and it also sits at the root of this repository.

```sh
gpg --import desdec-signing-key.asc
gpg --verify desdec-linux-x86_64-release.tar.gz.asc \
             desdec-linux-x86_64-release.tar.gz
```

The SHA-256 checksum answers a different question: it says the download is
intact, not who produced it. The signature says both. The private key never
leaves the maintainer's machine — the build service does not hold it, it only
builds.

## What it does with your files and your machine

- **It never executes the binary being analysed.** Nothing in it is run,
  mapped, or loaded.
- **It reads, and writes only where you ask.** The analysed file is opened
  read-only; a patch is written to a separate copy you name yourself.
- **It makes no network connection unless you configure one.** Out of the box
  it connects to nothing. The optional AI assistance is the single exception,
  and only once you have chosen a provider: a local model over the loopback
  interface, or Anthropic's API over the internet. Even then it is the
  extracted facts — instructions, symbol names, strings — that are sent, never
  the file, and the view shows the exact text before you ask.
- **Every executable byte that was read is decoded** — there is no cap on the
  number of instructions. A large shared library really does reach eighteen
  million of them, and the listing is virtualised, so its length costs the
  interface nothing.
- What remains bounded is the reading itself: at most 256 MiB per file, 20 000
  strings, 4 096 section entries. When a limit is reached the interface says
  so rather than presenting a partial listing as if it were the whole program.
- The only external programs it will start are the ones you choose: a
  decompiler (`rizin`, `retdec-decompiler`), YARA, or a local model server.
  None is required, and none is started unless it is selected in the
  preferences.
- **A script reaches nothing but the analysis.** The scripting engine is
  handed the decoded binary and the notes on it, and there is no file system,
  no network and no process in its vocabulary — not by a rule that could be
  forgotten, but because none was ever registered. A script from somewhere
  else runs with exactly the permissions you granted it, and one whose
  manifest starts asking for more stops until you have seen the new list.
- **An API key is never written to the preferences file.** The Anthropic key is
  read from `ANTHROPIC_API_KEY`, or from a file you name whose permissions are
  yours to set.

### Where it keeps things

| | Preferences | Cached decompilations |
| --- | --- | --- |
| Linux | `$XDG_DATA_HOME/desdec/app.ron` or `~/.local/share/desdec/app.ron` | `$XDG_CACHE_HOME/desdec/decompiled` or `~/.cache/desdec/decompiled` |
| macOS | `~/Library/Application Support/Desdec/app.ron` | `~/Library/Caches/desdec/decompiled` |
| Windows | `%APPDATA%\Desdec\data\app.ron` | `%LOCALAPPDATA%\desdec\decompiled` |

Preferences are written a fraction of a second after they stop changing, and
flushed to disk there and then — not left to an auto-save timer or to a clean
shutdown. A window closed abruptly on Windows used to lose a theme chosen
moments earlier; it does not any more. Persistence can be turned off entirely,
which also clears what was already stored. Decompilations are cached under the
SHA-256 of the file they came from, so a truncated file — which has no
trustworthy digest — is never cached.

Two more directories belong to you rather than to the application, and neither
is a cache: the notes you take on a binary live under `desdec/notes`, one file
per binary keyed by its SHA-256 rather than by its path, and plugins live under
`desdec/plugins`, one directory each. The plugin window shows the exact path on
your machine, and `examples/plugins` in this repository holds one to copy there.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

The test suite runs in about twenty seconds and needs nothing installed. It
analyses synthetic ELF, PE and AArch64 Mach-O binaries built byte by byte in
`desdec-core::fixtures`, so the readers for formats the host does not use are
exercised on every run, on every platform.

To look at the icon set after changing a glyph:

```sh
DESDEC_ICON_SHEET=/tmp/icons.svg cargo test -p desdec-app icon_sheet
```

### Layout

- `crates/desdec-core` — binary inspection and analysis. Knows nothing about
  any user interface. Parsing untrusted input is bounded and total: every read
  goes through bounds-checked helpers, every table walk is capped, and no input
  can panic.
- `crates/desdec-app` — the native `egui` application.
- `docs/ARCHITECTURE.md` — the dependency direction and what is deliberately
  not in the core.

## Licence

Apache-2.0 OR MIT, at your option: [LICENSE-APACHE](LICENSE-APACHE) and
[LICENSE-MIT](LICENSE-MIT). Both are offered from the About window as well, so
the terms are reachable from the application itself.

Unless you state otherwise, any contribution you deliberately submit for
inclusion in this work shall be dual-licensed as above, with no additional
terms or conditions.
