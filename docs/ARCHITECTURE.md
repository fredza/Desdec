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
- **Emulation** (`emulate`) — a processor Desdec builds: a register file, an
  address space laid out from the file's own segment table, and an interpreter
  over the instructions iced-x86 already decoded. See below.
- **Updates** (`update`) — asking GitHub whether a newer release exists, and
  fetching one intact. Off until the reader agrees to it; it never replaces
  the running program.
- **Optional external tools** — decompilers (`rizin` + `rz-ghidra`, RetDec),
  YARA, and the AI assistant. None is required, none is started unless it is
  selected, and none of them executes the analysed file.

Parsing untrusted files is bounded and total by construction: every read goes
through the bounds-checked helpers in `bytes.rs`, table walks are capped, no
input can panic, and at most `ANALYSIS_BYTE_LIMIT` (256 MiB) of a file is read.
`unsafe_code` is forbidden across the workspace.

## The emulated processor

Desdec never runs the file. `emulate` does not change that: it interprets the
instructions the listing shows, on memory it builds itself, and no byte of the
file ever reaches the machine's processor. There is no operating system behind
it either, and that is the whole of the trade — every point where a real
debugger would ask the system for an answer is a point where the run stops and
says so, rather than inventing one:

- an instruction the interpreter does not carry out — never skipped, because
  everything after it would be a fiction,
- a `syscall`, an interrupt, a `cpuid`: a question for a system,
- a read or a write outside anything the file maps,
- a call through the table of external calls, which only a loader fills in —
  it still reads zero, so the call lands on the first page, which is
  deliberately left unmapped exactly as every operating system leaves it.

Two things follow from being an interpreter rather than an attachment, and
neither is available to a debugger that attaches to a real process.

**A breakpoint can carry a condition.** A breakpoint inside a loop of ten
thousand turns is a breakpoint the reader presses "run" at ten thousand times;
what they wanted was the turn where `rcx` is four. `emulate::condition` is a
small expression language over exactly what the machine holds — registers by
name, memory through `[…]` at a stated width, numbers, and the usual
operators — with no way to call anything and nothing to assign to: a condition
is a question asked of the state, and asking it must not change the answer. An
expression that does not parse is refused where it is typed. One that reads an
address nothing maps has **no value** rather than a value of zero, so
`[rax]:1 == 0` does not hold for a pointer that leads nowhere. A pass count
lets a stated number of qualifying passes by first.

**The run goes backwards.** The state before each instruction is kept — the
register file, and the bytes that instruction overwrote — so stepping back
restores it exactly rather than working it out again or re-running from the
start. It is bounded, like the trace: the recent past, and the interface says
how much of it there is. Stepping back out of a fault undoes the fault with the
instruction, so a run that ended is a run that can carry on.

What it buys is what a static reading can never give: register values that are
values, a stack that is the stack, an indirect call that goes where it goes, a
loop whose trip count is a fact, and breakpoints that are reached rather than
reasoned about.

Three decisions are worth naming. **Memory is mapped by segment, not by
section** — that is what a loader maps, and mapping by section puts `.text` and
the note before it in one page under the note's read-only rights, so the first
fetch faults. **The file's bytes are borrowed, never copied**: a written page
goes to an overlay consulted first, so emulating a shared library costs the
pages the program wrote to. And **rights are enforced rather than recorded**:
writing to `.text` faults, and so does fetching from `.data`.

Only x86 and x86-64 have an interpreter. AArch64 is decoded and read like
everything else, and says plainly that it has no processor here.

## Updates

The one part of Desdec that reaches the network without being asked a question
about a file, and so the one with a consent of its own. Asking tells GitHub
that this copy was started, which is a thing to agree to rather than discover:
the question is put once, in the reader's language, with what it costs written
down. Its two answers are "yes" and "not this time" — a refusal for good lives
in the preferences, because a pop-up is a bad place to be asked to decide for
ever, and "never" offered beside "yes" punishes hesitation.

What a check does is compare three numbers and stop. What a download does is
finish the job: a release publishes a `.sha256` beside each archive, so the
archive is hashed on arrival and refused if it does not match — deleted rather
than left on disk under a name that looks right. That answers whether the bytes
are intact and nothing more; **who** made them is the detached `.asc` signature's
answer, and checking it stays a deliberate act with `gpg`.

Desdec never replaces itself. The archive lands in a folder and opening it is
the reader's own move — which also means there is no code here that writes to a
running executable, no restart, and none of the per-platform trouble that comes
with both.

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
