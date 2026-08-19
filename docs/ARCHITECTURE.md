# Architecture

Desdec uses a strict dependency direction: the application may depend on the
core, but the core never depends on a graphical framework or a plugin runtime.

```text
desdec-app (egui)          views, windows, the command registry, the reader's
        |                  notes, the script engine and the plugin loader
        v
desdec-core                formats, disassembly, analysis, assembly, patching,
                           and the optional external tools
```

`desdec-core` imports no graphical crate and no scripting runtime, which is
what keeps it usable as a library and testable without a window.

## What the core holds today

- **Formats** — ELF, PE and Mach-O: header recognition, the section table, the
  entry point, load-time mapping, linked libraries, hardening (RELRO, canary,
  NX, PIE, CFG), and the PE import table read one descriptor at a time.
- **Symbols** — for all three formats.
- **Disassembly** — iced-x86 for x86 and x86-64, Capstone for AArch64. Every
  executable byte that was read is decoded; nothing caps the number of
  instructions, and `Decoded::truncated` marks only code lying past the bytes
  the analysis read.
- **Analysis** — printable strings, per-region entropy, the source language a
  file gives evidence of, what an operand designates, what last wrote a
  register, which condition flags an instruction settles and consults
  (`flags`), and the stack state at each instruction (`Trace`).
- **Assembly and patching** — one typed line encoded back to bytes
  (`assemble`), and patches that keep their length and are written to a copy.
- **Optional external tools** — decompilers (`rizin` + `rz-ghidra`, RetDec),
  YARA, and the AI assistant. None is required, none is started unless it is
  selected, and none of them executes the analysed file.

Parsing untrusted files is bounded and total by construction: every read goes
through the bounds-checked helpers in `bytes.rs`, table walks are capped, no
input can panic, and at most `ANALYSIS_BYTE_LIMIT` (256 MiB) of a file is read.
`unsafe_code` is forbidden across the workspace.

## What the diagram above is not yet

There is no intermediate representation. The pseudo-code the reader sees is
either an external decompiler's output or a conservative, line-by-line
translation of the decoded flow — and that translation lives in the
application (`ui/decompile.rs`), because it is a reading offered on screen
rather than a model the core reasons over. An IR belongs in the core if one is
ever built; nothing depends on it today, and the diagram should not be read as
saying it exists.

## Scripts and plugins

A script does not act. It is handed a *subject* — the analysis, the file's
bytes, the reference index, the reader's notes — which the application lends
for the run and takes back afterwards, and it produces a list of effects the
application applies once the script has finished. Nothing a script does reaches
the application directly, and nothing it does reaches the analysed file at all:
a patch it proposes lands in the same pending list the patch editor writes to.

Capability is granted, never acquired. The engine is built with no file system,
no network and no process in its vocabulary — those functions are not
registered, so there is no rule to circumvent — and `import` and `eval` are
disabled so a script cannot bring in code that was never reviewed. What is left
is bounded: operations, wall-clock time, effects, search hits and printed
lines, none of which a script can raise.

A plugin is a directory holding a manifest and one script. The manifest *asks*
for permissions; the reader grants them, having seen the list, and a plugin
whose manifest later asks for more stops until the new list has been seen. A
plugin that was never enabled has never run.
