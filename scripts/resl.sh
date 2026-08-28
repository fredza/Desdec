#!/usr/bin/env bash
# Replaces the Desdec installed on this machine with the one this checkout
# builds: the old one out, the new one in, in that order and in one command.
#
#   scripts/resl.sh                     # remove, rebuild, install
#   scripts/resl.sh --dry-run           # say what would happen, change nothing
#   scripts/resl.sh --no-build          # install what is already in target/release
#   scripts/resl.sh --name desdec-dev   # replace a side install
#   scripts/resl.sh --prefix /usr/local/bin
#   scripts/resl.sh --no-desktop        # binary only, no icon and no menu entry
#
# `unsl.sh` and `insl.sh` already do the two halves. What this adds is the
# order, and the reason for it: an install under an older name, or into a
# prefix no longer used, leaves behind a binary the new one does not overwrite
# and a menu entry the dock still points at. Installing over the top hides that
# rather than fixing it — the dock keeps launching yesterday's build, and the
# version in the About window disagrees with the one on the PATH.
#
# **Your notes are never touched.** `unsl.sh --purge` exists and takes the
# annotations, the preferences and the library catalogue with it; this never
# passes it. An update that deleted what a reader wrote about their binaries
# would be a worse defect than the one it was fixing, and there is no flag here
# to ask for it: uninstalling for good is `unsl.sh --purge`, deliberately, and
# it is a different act from updating.
#
# Nothing here needs root unless the prefix does.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="desdec-app"          # what cargo builds
DEFAULT_NAME="desdec"        # what it is called once installed

prefix="${DESDEC_PREFIX:-$HOME/.local/bin}"
name="$DEFAULT_NAME"
build=yes
desktop=yes
fix_path=yes
dry=no

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
step() { printf '\n== %s\n' "$*"; }

usage() {
    cat <<'USAGE'
Usage: resl.sh [options]

  --dry-run       Say what would happen; remove, build and install nothing
  --no-build      Install what is in target/release instead of rebuilding
  --prefix <dir>  Install into this directory (default: ~/.local/bin)
  --name <name>   Install under this name (default: desdec)
  --no-desktop    Do not install the icon and the menu entry
  --no-path       Do not touch any shell profile
  -h, --help      Show this message

Your notes, preferences and library catalogue are kept. To remove those too,
uninstall on purpose with `unsl.sh --purge`.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) dry=yes; shift ;;
        --no-build) build=no; shift ;;
        --prefix) prefix="${2:-}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift 2 ;;
        --prefix=*) prefix="${1#*=}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift ;;
        --name) name="${2:-}"; [ -n "$name" ] || die "--name needs a name"; shift 2 ;;
        --name=*) name="${1#*=}"; [ -n "$name" ] || die "--name needs a name"; shift ;;
        --no-desktop) desktop=no; shift ;;
        --no-path) fix_path=no; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $1" ;;
    esac
done

for half in unsl.sh insl.sh; do
    [ -x "$root/scripts/$half" ] || die "$root/scripts/$half is missing or not executable"
done

# What is installed now, asked of the binary rather than assumed. A version too
# old to answer `--version` opens a window instead of printing anything, so the
# question is given a moment and its silence is an answer of its own.
installed="unknown"
target="$prefix/$name"
if [ -x "$target" ]; then
    installed="$(timeout 5 "$target" --version 2>/dev/null | head -1 || true)"
    [ -n "$installed" ] || installed="present, and too old to say which version"
else
    installed="none"
fi

# And what is about to replace it, read from the crate rather than from a tag:
# this installs what the checkout builds, which is not always what was released.
building="$(
    cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" 2>/dev/null \
        | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "desdec-app"))' \
        2>/dev/null || echo "unknown"
)"

say "installed : $installed"
say "installing: v$building  (from $root)"
say "prefix    : $prefix/$name"

if [ "$dry" = yes ]; then
    step "dry run — nothing below is carried out"
    say "  1. $root/scripts/unsl.sh --dry-run --prefix $prefix --name $name"
    "$root/scripts/unsl.sh" --dry-run --prefix "$prefix" --name "$name" \
        $([ "$fix_path" = no ] && echo --keep-path) || true
    say ""
    say "  2. cargo build --release -p desdec-app"
    say "  3. $root/scripts/insl.sh --prefix $prefix --name $name"
    say ""
    say "your notes and preferences would be kept either way"
    exit 0
fi

# The old one first. `--keep-path` throughout: the line `insl.sh` appends is
# the same line it would append again, and removing it only to put it back
# rewrites the reader's shell profile twice for nothing.
step "removing what is installed"
"$root/scripts/unsl.sh" --prefix "$prefix" --name "$name" --keep-path

# The build, before anything is installed: a compilation that fails must leave
# the machine with no Desdec rather than with half of one, and this is the
# order that makes the failure visible instead of silent.
if [ "$build" = yes ]; then
    step "building"
    ( cd "$root" && cargo build --release -p "$BINARY" )
else
    [ -x "$root/target/release/$BINARY" ] \
        || die "--no-build, but $root/target/release/$BINARY is not there"
fi

step "installing"
install_args=(--prefix "$prefix" --name "$name")
[ "$desktop" = no ] && install_args+=(--no-desktop)
[ "$fix_path" = no ] && install_args+=(--no-path)
"$root/scripts/insl.sh" "${install_args[@]}"

# What actually ended up on the machine, asked of it rather than assumed: the
# whole point of this script is that the dock and the PATH agree with each
# other, and the only way to know they do is to ask them.
step "checking"
now="$(timeout 5 "$prefix/$name" --version 2>/dev/null | head -1 || true)"
if [ -z "$now" ]; then
    die "$prefix/$name does not answer --version"
fi
say "  on disk    : $now"

# The name the shell would actually reach, which is not always the one just
# installed: another copy earlier in the PATH keeps winning, and that is
# exactly the state this script exists to end.
reached="$(command -v "$name" 2>/dev/null || true)"
if [ -z "$reached" ]; then
    warn "  on PATH    : $name is not reachable — open a new shell, or add:"
    warn "                 export PATH=\"$prefix:\$PATH\""
elif [ "$reached" != "$prefix/$name" ]; then
    warn "  on PATH    : $reached — an older copy is found first"
    warn "               remove it, or put $prefix ahead of it in your PATH"
else
    say "  on PATH    : $reached"
fi

if [ "$desktop" = yes ] && [ "$(uname -s)" = Linux ]; then
    data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
    entry="$data_home/applications/Desdec.desktop"
    [ "$name" = "$DEFAULT_NAME" ] || entry="$data_home/applications/$name.desktop"
    if [ -f "$entry" ]; then
        exec_line="$(awk -F= '/^Exec=/ { print $2; exit }' "$entry")"
        say "  menu entry : $entry"
        say "  launches   : $exec_line"
        # The entry pointing somewhere else is how a dock goes on starting
        # yesterday's build long after the PATH stopped doing so.
        case "$exec_line" in
            "$prefix/$name"*|"$name"*) ;;
            *) warn "               ^ this is not what was just installed" ;;
        esac
    else
        warn "  menu entry : $entry is not there"
    fi
fi

say ""
say "Your notes, preferences and library catalogue were kept."
if [ "$desktop" = yes ] && [ "$(uname -s)" = Linux ]; then
    say "A dock that still shows the old icon is holding the entry it read at"
    say "login: unpin it and pin it again, or log out and back in."
fi
