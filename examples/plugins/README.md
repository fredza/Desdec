# Plugins

A plugin is a directory: a manifest saying what it is and what it needs, and a
script. Copy one of these directories into Desdec's plugin directory — the
plugin window (`Plugins` in the menu) shows the exact path on your machine —
then open that window, read what the plugin asks for, and enable it.

```text
plugins/
  large-functions/
    plugin.ron     the manifest
    plugin.rhai    the script it names
```

Nothing is compiled and nothing is loaded into the process. A plugin is text,
read by the same sandboxed engine the script console runs, which is why
installing one is a smaller decision than installing a native plugin into a
debugger.

## The manifest

```ron
(
    name: "Large functions",
    version: "1.0",
    author: "Desdec",
    description: "What it does, in one sentence.",
    script: "plugin.rhai",
    hooks: [OnOpen],
    permissions: [WriteNotes],
)
```

Every field may be left out. `script` names one file **in the plugin's own
directory**; a path leading anywhere else is refused rather than followed.

| Hook | When it runs |
| --- | --- |
| `OnOpen` | Once, as soon as a binary has been analysed. |
| `OnDemand` | Only when you press Run in the plugin window. |

| Permission | What it allows |
| --- | --- |
| `WriteNotes` | `label`, `comment`, `bookmark`, `unbookmark`, `clear_note` |
| `Navigate` | `go_to` |
| `ProposePatches` | `patch` — which still lands in the pending list, never in the file |

A plugin asks; it does not take. The list is shown before you enable it, and
enabling grants exactly what the manifest asked for at that moment. If the
manifest later asks for more, the plugin stops until you have seen the new
list — it does not silently acquire the difference, and it does not quietly run
with less.

## The language

Scripts are [Rhai](https://rhai.rs): `let`, `if`, `for … in`, `fn`, arrays,
maps, string interpolation with backticks. `import` and `eval` are disabled,
and there is no file system, no network and no process in the vocabulary —
those functions were never registered, so there is no rule to get around.

Bounds, which no script can raise: 50 million operations, 10 seconds, 200 000
changes, 1 000 printed lines. A script that reaches one is stopped where it
stands and says which bound it hit; whatever it had already asked for still
happens.

### What the binary is

| | |
| --- | --- |
| `binary()` | `name`, `format`, `architecture`, `size`, `analysed`, `truncated`, `code_truncated`, `entropy`, `sha256`, `instructions` |
| `entry()` | Entry point, or `()` when the format declares none |
| `sections()` | `name`, `address`, `size`, `offset`, `stored`, `readable`, `writable`, `executable`, `mapped`, `entropy` |
| `section_at(a)` | The section containing an address, or `()` |
| `functions()` | `name`, `address`, `end`, `size` |
| `symbols()` | `name`, `address`, `size`, `imported` |
| `strings()` | `text`, `offset`, `address`, `truncated` |

### The listing and the bytes

| | |
| --- | --- |
| `instruction_count()` | How many instructions were decoded |
| `instruction(a)` | The one at exactly this address, or `()` |
| `instruction_at_index(i)` | The i-th, for walking the whole listing |
| `instructions(from, upto)` | Every instruction in an address range |
| `read(a, n)` | `n` bytes from an address, as a blob |
| `read_at_offset(o, n)` | The same, from a file offset |
| `offset_of(a)` / `address_at(o)` | Between the two, when the mapping holds |

An instruction is `address`, `text`, `bytes`, `size`, `section`, `offset`.

### Finding things

| | |
| --- | --- |
| `find_bytes("48 8b ?? 05")` | `??` stands for any byte |
| `find_instructions(t)` | Text in the decoded instructions |
| `find_notes(t)` | Text in your own labels and comments |
| `refs_to(a)` | `from` and `kind` — `call`, `jump`, `reads`, `pointer` |
| `ref_count(a)` | How many places name this address |

Each hit is `address`, `offset`, `section`, `text`.

### Notes, and changing them

| | |
| --- | --- |
| `label_of(a)`, `comment_of(a)`, `bookmarked(a)`, `notes()` | Read |
| `label(a, t)`, `comment(a, t)` | Write |
| `bookmark(a)`, `unbookmark(a)`, `clear_note(a)` | Mark |
| `go_to(a)` | Move the listing |
| `patch(a, "nop")` | Assemble one line at that address |

`patch` follows the same rules as the patch editor: a shorter encoding is
filled out with `nop`, and a longer one is refused with the room it needed.

### Addresses

An address is its own type, not a number. A binary mapped high — a kernel
module at `0xffffffff81000000` — has addresses that do not fit a signed 64-bit
integer, and comparing one as a number would come out backwards.

```rhai
let here = address(0x401000);
print(here);        // 0x401000
print(here + 4);    // 0x401004
print(here.int);    // 4198400, and lossy above the middle of the space
if entry() > 0x400000 { … }
```
