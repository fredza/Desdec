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
- **Symbols** — for all three formats, and (`discover`) the functions of a file
  that names none: the entry point, every address something calls, and
  compiler prologues standing where the last function ended. Each address
  carries the reason it is offered, and the reasons are not equally good — a
  call is what the bytes say, a prologue is a reading of their shape. Measured
  against a real binary's own symbol table: about ninety per cent of what it
  names is found again from the code alone.
- **Disassembly** — iced-x86 for x86 and x86-64, Capstone for AArch64. Every
  executable byte that was read is decoded; nothing caps the number of
  instructions, and `Decoded::truncated` marks only code lying past the bytes
  the analysis read. The x86 text is GAS's, in AT&T order, and everything built
  on the listing parses that one spelling. A reader who wants Intel order gets
  it from `Nasm`, which decodes the row's own fifteen bytes again rather than
  moving the words of the AT&T text around: the two syntaxes stop corresponding
  word for word at the string operations, the sign-extending moves and every
  operand whose width only a mnemonic suffix carried, and a rewriter would
  invent an instruction exactly there. It is a reading, not a second analysis —
  nothing about the instruction changes with the spelling.
- **Analysis** — printable strings, per-region entropy, the source language a
  file gives evidence of, what an operand designates, what last wrote a
  register, which condition flags an instruction settles and consults
  (`flags`), and the stack state at each instruction (`Trace`).
- **Decompilation** (`decompiler::native`) — C, from the analysis above and
  nothing else. Six passes: lifting one instruction to the effects it has
  (`lift`), the calling convention and the frame (`naming`), substitution and
  dead-store removal (`dataflow`), dominators and back edges (`structure`),
  and the C itself (`emit`), over a small IR (`ir`). Deterministic, about a
  millisecond per function, and every line of the output carries the address
  of the instruction it came from — which is what lets the view take a click
  to the listing, and what no external engine publishes.
- **Comparison** (`diff`) — two binaries set beside each other. Functions are
  paired in passes, strongest first: a name both files carry, the same address
  with the same bytes, the same bytes anywhere, the same instructions once the
  numbers they carry are set aside, and — last — the one unpaired function each
  side of a pair already made calls. Each pass sees only what the ones before
  it left, and every pass refuses a key that is not unique on *both* sides:
  three identical stubs on the left and three on the right pair six ways, and
  an arbitrary pairing is not a weaker answer than none, it is a wrong one.
  Whether a pair changed is settled by its bytes, which is exact; how far apart
  the bodies are is the longest common subsequence of their instructions with
  those numbers ignored, so a build at another base address is not reported as
  a program rewritten — and a changed pair that measures no distance says
  exactly that. The alignment is bounded (`MAXIMUM_ALIGNMENT`) and answers
  `None` rather than an estimate above it. Nothing here reads a file; it works
  over two analyses already made.
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
are intact and nothing more, and the window says as much: releases are not
signed from v0.4.1 on, so there is no second question anything published
answers. Archives up to v0.4.0 carry a detached `.asc` and stay checkable with
`gpg` and the key at the root of the repository.

Desdec never replaces itself. The archive lands in a folder and opening it is
the reader's own move — which also means there is no code here that writes to a
running executable, no restart, and none of the per-platform trouble that comes
with both.

## The decompiler

There is an intermediate representation, and it lives in the core:
`decompiler::native::ir`. Three ideas — a *place* a value can be written to, an
*expression* being read, a *statement* that is one effect — and two properties
the rest of the pipeline rests on.

The first is that the conditions are ordinary places. The machine has `ZF`,
`SF`, `CF` and `OF`, and the signed comparisons are combinations of them: `jl`
branches on `SF ≠ OF`. Writing those four out and substituting them gives, for
a comparison and the branch below it, an expression that is exactly true and
exactly unreadable. So what is recorded is the *question* — `Less`,
`LessOrEqual`, `BelowOrEqual` — which `cmp` answers in terms of its own
operands and which `jl` reads. Everything the output has that a line-by-line
translation lacks comes from that one decision, by way of the ordinary
substitution every other value goes through.

The second is that what is not modelled says so. An instruction the lifter does
not know becomes `Stmt::Opaque` carrying its own assembly; nothing is
propagated across one, nothing around one is deleted, and the view says what
fraction of the body was understood. This makes the output look less finished
than a decompiler that guesses, and it is the reason a reader can act on it.

Two limits are deliberate and are written down where they are implemented.
Substitution runs **within a basic block** and does not cross a branch, for the
reason `analysis::stack` and `operand` draw the same line: a value carried
across a branch is only the value that arrives if that branch was taken.
Liveness *does* cross, because deleting an assignment requires knowing no other
block reads it. And structuring recovers `while`, `if`/`else`, `break` and
`continue` where the graph has them, and emits a `goto` where it does not —
a function whose flow really is irreducible is one the reader needs to know is
irreducible.

The line-by-line translation is still there, in the application
(`ui/decompile.rs`), under the C and beside the listing. It is the only one of
the two that shows a whole binary rather than one function, and it is what
answers where an instruction was not modelled.

x86-64 is lifted; AArch64 is not yet, and reaches `Stmt::Opaque` by the same
door everything unmodelled does — so an AArch64 function decompiles to its own
listing inside a C frame, structured and named, rather than to nothing.

## The call graph

The Functions view answers "what is in this function" and the reference index
answers "who names this address". Neither answers the question a reader
actually arrives with — *how does anything get to here* — because that is a
chain, and no single step of it is visible from either end.

`callgraph` is that chain, built once per binary from the calls the listing
already decoded, in both directions. Three things it does not do: it does not
guess at `call *%rax`, it counts it, so a function whose callees are all
indirect does not read as one that calls nothing; it does not follow a call
into a library, whose code is in a file that is not open; and it does not
invent a caller for a function nothing calls — an entry point, a callback
handed to a library and dead code look alike from here, and "nothing calls
this" is the honest answer to all three.

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
