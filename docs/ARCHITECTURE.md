# Architecture

Desdec uses a strict dependency direction: the application may depend on the
core, but the core never depends on a graphical framework or a plugin runtime.

```text
desdec-app (egui)
        |
        v
desdec-core
  formats -> memory -> disassembly -> analysis -> IR -> patching
```

The current milestone implements `formats`: header recognition plus structured
reading of the section table, the entry point, printable strings and per-region
entropy for ELF, PE and Mach-O (`desdec-core::analysis`). Parsing untrusted
files is bounded and total by construction — every read goes through the
bounds-checked helpers in `bytes.rs`, table walks are capped, and no input can
panic. The following milestone will add symbols and imports/exports.

## Scripts and plugins

The scripting engine lives in `desdec-app` and never in the core: `desdec-core`
depends on no graphical framework and no plugin runtime, which is what keeps it
usable as a library and testable without either.

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
is bounded: operations, wall-clock time, effects and printed lines, none of
which a script can raise.

A plugin is a directory holding a manifest and one script. The manifest *asks*
for permissions; the reader grants them, having seen the list, and a plugin
whose manifest later asks for more stops until the new list has been seen. A
plugin that was never enabled has never run.
