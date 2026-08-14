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

Plugins will be designed only after the core data model is stable; they must not obtain unrestricted filesystem or process access by
default.
