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

The current first milestone implements `formats`: safe header recognition for
ELF, PE and Mach-O. The following milestone will add structured parsers and
section/symbol access. Plugins will be designed only after the core data model
is stable; they must not obtain unrestricted filesystem or process access by
default.
