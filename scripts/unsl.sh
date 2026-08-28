#!/usr/bin/env bash
# Takes back out what `insl.sh` put in: the binary, the icon, the menu entry,
# and the one PATH line it appended to your shell profile.
#
#   scripts/unsl.sh                     # remove ~/.local/bin/desdec and its desktop files
#   scripts/unsl.sh --dry-run           # say what would go, remove nothing
#   scripts/unsl.sh --name desdec-dev   # remove a side install, leave the release alone
#   scripts/unsl.sh --prefix /usr/local/bin
#   scripts/unsl.sh --purge             # and the notes, preferences and library catalogue
#
# `--purge` reaches what Desdec keeps for itself. It never reaches a `.dcl`
# saved beside a binary: that is a file of yours, in a directory of yours, and
# uninstalling a program is no reason to go through a disk deleting files by
# extension.
#   scripts/unsl.sh --keep-path         # leave the shell profile untouched
#
# Run it before installing over an older Desdec — an install under a different
# name, or into a prefix you no longer use, leaves a binary behind that the
# new one does not overwrite and a menu entry that still points at it.
#
# What it will not do is delete something it cannot identify. The binary is
# read, not run: a version too old to answer `--version` would open a window
# instead of answering, and a Desdec too broken to start is exactly the one
# you want to be able to remove. It is removed if it is a 64-bit ELF carrying
# the application's own name, and refused otherwise — `--name ls --prefix
# /usr/bin` takes nothing out.
#
# The reader's own files are kept unless you ask for them by name: notes,
# preferences and the library catalogue survive an ordinary uninstall, so
# reinstalling brings back the session you left. `--purge` is the flag that
# says otherwise, and it names every directory it empties.
set -euo pipefail

DEFAULT_NAME="desdec"

prefix="${DESDEC_PREFIX:-$HOME/.local/bin}"
name="$DEFAULT_NAME"
dry=no
purge=no
fix_path=yes
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'USAGE'
Usage: unsl.sh [options]

  --dry-run        List what would be removed, remove nothing
  --prefix <dir>   Look for the binary here (default: ~/.local/bin)
  --name <name>    Remove the install under this name (default: desdec)
  --purge          Also remove the notes, preferences and library catalogue
  --keep-path      Do not touch the PATH line in your shell profile
  -h, --help       Show this message

DESDEC_PREFIX, XDG_DATA_HOME and XDG_CONFIG_HOME say where to look, the same
way they do for insl.sh.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) dry=yes; shift ;;
        --prefix)  prefix="${2:-}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift 2 ;;
        --prefix=*) prefix="${1#*=}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift ;;
        --name)    name="${2:-}"; [ -n "$name" ] || die "--name needs a name"; shift 2 ;;
        --name=*)  name="${1#*=}"; [ -n "$name" ] || die "--name needs a name"; shift ;;
        --purge)   purge=yes; shift ;;
        --keep-path) fix_path=no; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $1" ;;
    esac
done

# The same trailing-slash normalisation insl.sh does, so `--prefix ~/.local/bin/`
# names the very file that install wrote and does not miss it by one character.
prefix="${prefix%/}"
[ -n "$prefix" ] || prefix="/"

removed=0
kept=0

# Everything goes through here, so `--dry-run` cannot forget a case and the
# tally at the end counts what actually happened.
drop() {
    local what="$1"
    if [ "$dry" = yes ]; then
        say "would remove  $what"
    else
        rm -rf -- "$what" || { warn "cannot remove $what"; return 0; }
        say "removed  $what"
    fi
    removed=$((removed + 1))
}

if [ "$name" = "$DEFAULT_NAME" ]; then
    desktop_file="Desdec.desktop"
else
    desktop_file="$name.desktop"
fi

# The binary — read to be sure of what it is, never executed.
binary="$prefix/$name"
if [ ! -e "$binary" ]; then
    say "no $binary — nothing installed there under that name"
    kept=$((kept + 1))
else
    magic="$(head -c 5 -- "$binary" 2>/dev/null | od -An -tx1 | tr -d ' \n')"
    if [ "$magic" != "7f454c4602" ]; then
        die "$binary is not a 64-bit ELF — refusing to remove it"
    fi
    # Its own application id, which every build carries and which nothing else
    # on a prefix is likely to. Cheaper and surer than trusting the file name.
    if ! grep -qa 'Desdec' -- "$binary"; then
        die "$binary does not look like Desdec — refusing to remove it"
    fi
    drop "$binary"
fi

# The icon, in every size a past install may have written it at: v0.4.1 writes
# 128x128, and an install that once wrote another size would otherwise leave a
# stale mark behind for the theme to find.
if [ -d "$data_home/icons/hicolor" ]; then
    while IFS= read -r icon; do
        drop "$icon"
    done < <(find "$data_home/icons/hicolor" -type f -name "$name.png" -path '*/apps/*' 2>/dev/null)
fi

# The menu entry, if it is the one that points at what was just removed. An
# entry naming another prefix belongs to another install, and taking it out
# would leave that install with an icon and no way into the menu.
entry="$data_home/applications/$desktop_file"
if [ -f "$entry" ]; then
    # Match the Exec line as a string, not a pattern. The binary's path holds a
    # dot — `~/.local` — that grep would read as "any character", so a regex
    # could claim an install one character apart is this one. The `%f` insl.sh
    # appends is the only thing allowed to follow the path.
    exec_line="$(grep -m1 '^Exec=' -- "$entry" || true)"
    case "$exec_line" in
        "Exec=$binary" | "Exec=$binary "*)
            drop "$entry" ;;
        *)
            warn "$entry does not point at $binary — left as it is"
            warn "    ($exec_line)"
            kept=$((kept + 1)) ;;
    esac
else
    kept=$((kept + 1))
fi

# The PATH line, and only the one this family of scripts wrote: the marker
# comment is what identifies it, so a line you added yourself years ago stays.
# The profile is copied beside itself first — it is not a file to rewrite
# without a way back.
remove_path_line() {
    local profile
    case "$(basename -- "${SHELL:-/bin/sh}")" in
        zsh)  profile="${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash) profile="$HOME/.bashrc" ;;
        *)    profile="$HOME/.profile" ;;
    esac
    [ -f "$profile" ] || return 0
    grep -q 'added by insl\.sh' -- "$profile" || return 0

    if [ "$dry" = yes ]; then
        say "would remove  the insl.sh PATH line in $profile"
        removed=$((removed + 1))
        return 0
    fi

    cp -- "$profile" "$profile.insl.bak"
    local staged="$profile.insl.$$"
    # The marker comment and the export under it, as a pair. `drop` is cleared
    # on any other line, so a marker followed by something else takes nothing
    # with it.
    awk '
        $0 ~ /^#/ && index($0, "added by insl.sh") { drop = 1; next }
        drop && $0 ~ /^export PATH=/ { drop = 0; next }
        { drop = 0; print }
    ' < "$profile" > "$staged" || { warn "cannot rewrite $profile"; rm -f -- "$staged"; return 0; }
    mv -f -- "$staged" "$profile"
    say "removed  the insl.sh PATH line in $profile (kept a copy as $profile.insl.bak)"
    removed=$((removed + 1))
}

[ "$fix_path" = yes ] && remove_path_line

# The reader's own, and only when asked. Both directories are named `desdec`
# whatever the binary was installed as, so a `--name` install shares them:
# purging one purges the notes of the other, which is why this is a flag and
# not a default.
if [ "$purge" = yes ]; then
    for directory in "$data_home/desdec" "$config_home/desdec"; do
        [ -d "$directory" ] || continue
        say ""
        say "$directory holds:"
        find "$directory" -type f 2>/dev/null | sed 's/^/    /'
        drop "$directory"
    done
    # And say what `--purge` does *not* reach, because it is the reader's work
    # too: a `.dcl` saved beside a binary is a file of theirs, in a directory
    # of theirs, and nothing here goes looking for one. Removing Desdec has
    # never been a reason to walk somebody's disk deleting files by extension.
    say ""
    say "kept  the .dcl files you saved beside your binaries — this removes"
    say "      Desdec, not the work you did with it"
else
    for directory in "$data_home/desdec" "$config_home/desdec"; do
        [ -d "$directory" ] && say "kept  $directory — pass --purge to remove your notes and preferences too"
    done
fi

if [ "$dry" = no ] && [ "$removed" -gt 0 ]; then
    command -v update-desktop-database >/dev/null 2>&1 &&
        update-desktop-database "$data_home/applications" >/dev/null 2>&1 || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 &&
        gtk-update-icon-cache -qtf "$data_home/icons/hicolor" >/dev/null 2>&1 || true
fi

say ""
if [ "$dry" = yes ]; then
    say "Nothing was removed — $removed item(s) would be."
elif [ "$removed" -eq 0 ]; then
    say "Nothing to remove."
else
    say "Removed $removed item(s)."
    say "A dock keeps a pinned tile until it is unpinned by hand, and some"
    say "desktops forget a removed entry only after a logout."
fi
