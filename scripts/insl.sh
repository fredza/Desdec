#!/usr/bin/env bash
# Installs the ELF this checkout builds, and the three things that make it an
# application rather than a file: a name on your PATH, an icon, and a menu
# entry a dock can pin.
#
#   scripts/insl.sh                     # ~/.local/bin/desdec, icon, menu entry
#   scripts/insl.sh --build             # rebuild first, even if a binary is there
#   scripts/insl.sh --prefix /usr/local/bin
#   scripts/insl.sh --name desdec-dev   # beside an installed release
#   scripts/insl.sh --no-desktop        # binary only
#   scripts/insl.sh --no-path           # do not touch any shell profile
#
# What it does that `install-local.sh` does not: it installs the ELF that is
# already in `target/release` instead of building one every time, it checks
# that the thing it is about to install really is an ELF executable for this
# machine, and it *adds* the prefix to the PATH — one line, once, in the
# profile of the shell you actually use — rather than printing the line and
# leaving it to you.
#
# Nothing here needs root unless the prefix does, and nothing is written
# outside the prefix, `~/.local/share` and that one profile line.
set -euo pipefail

BINARY="desdec-app"          # what cargo builds
DEFAULT_NAME="desdec"        # what it is called once installed

prefix="${DESDEC_PREFIX:-$HOME/.local/bin}"
name="$DEFAULT_NAME"
build=auto
desktop=yes
fix_path=yes
# Where a desktop looks for menu entries and icons, as the XDG base directory
# specification names it.
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'USAGE'
Usage: insl.sh [options]

  --build          Rebuild before installing, even if target/release has a binary
  --prefix <dir>   Install into this directory (default: ~/.local/bin)
  --name <name>    Install under this name (default: desdec)
  --no-desktop     Do not add the icon and the menu entry (Linux only)
  --no-path        Do not add the prefix to the PATH in your shell profile
  -h, --help       Show this message

DESDEC_PREFIX sets the default prefix, XDG_DATA_HOME the directory the icon
and the menu entry are written under.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --build)  build=yes; shift ;;
        --prefix) prefix="${2:-}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift 2 ;;
        --prefix=*) prefix="${1#*=}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift ;;
        --name)   name="${2:-}"; [ -n "$name" ] || die "--name needs a name"; shift 2 ;;
        --name=*) name="${1#*=}"; [ -n "$name" ] || die "--name needs a name"; shift ;;
        --no-desktop) desktop=no; shift ;;
        --no-path)    fix_path=no; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $1" ;;
    esac
done

# A trailing slash on the prefix would make every path below it carry a double
# one — `~/.local/bin//desdec` — which works but reads as a mistake in the
# messages this prints. Strip it, without turning a bare `/` into nothing.
prefix="${prefix%/}"
[ -n "$prefix" ] || prefix="/"

# The checkout is found from this script's own location, not from the working
# directory: `insl.sh` typed from `target/release` — or from anywhere else —
# must install the checkout the script lives in.
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(dirname -- "$script_dir")"
[ -f "$root/Cargo.toml" ] || die "$root does not look like the Desdec checkout"

built="$root/target/release/$BINARY"

if [ "$build" = yes ] || [ ! -x "$built" ]; then
    command -v cargo >/dev/null 2>&1 || die "cargo is needed to build and is not installed"
    say "Building $root — this takes a few minutes the first time"
    # `--locked` so the build uses the lockfile as committed: an install that
    # quietly resolved newer dependencies would not be the checkout in front
    # of you, which is the one thing a local install promises.
    ( cd "$root" && cargo build --locked --release -p "$BINARY" )
    [ -x "$built" ] || die "cargo reported success but $built is not there"
else
    say "Installing the binary already in target/release — pass --build to rebuild it"
fi

# It says ELF on the tin, so look. Four bytes, and the fifth, which is 2 for
# the 64-bit class: a 32-bit object or a stray script under that name would
# install just as happily and then fail at the moment it is run, from the
# menu, with nowhere to print why.
magic="$(head -c 5 -- "$built" | od -An -tx1 | tr -d ' \n')"
case "$magic" in
    7f454c4602) ;;
    7f454c46*)  die "$built is an ELF, but not a 64-bit one" ;;
    *)          die "$built is not an ELF executable" ;;
esac

mkdir -p "$prefix"
target="$prefix/$name"
# Written under a temporary name in the target directory and then moved, so a
# running copy is replaced atomically rather than truncated under its feet.
staged="$prefix/.$name.$$"
cp "$built" "$staged"
chmod +x "$staged"
mv -f "$staged" "$target"

# Ask it what it is, which is also a check that what was just installed runs
# at all. Bounded, because a Desdec from before v0.4.1 takes the argument for
# a file to analyse and opens a window instead of answering.
reported=""
if command -v timeout >/dev/null 2>&1; then
    reported="$(timeout 5 "$target" --version 2>/dev/null || true)"
fi
case "$reported" in
    desdec\ v*) say "Installed $target — $reported" ;;
    *)          say "Installed $target" ;;
esac

# The file's own name is what a Wayland compositor matches against the window's
# application id, which is `Desdec`: the ordinary install must therefore be
# `Desdec.desktop` or the window opens under a generic icon and the dock pins
# a second, empty tile beside the running one.
#
# An install under another name gets a file and a menu name of its own, so it
# does not overwrite the ordinary one. Its *window* still carries the id
# `Desdec` — that is compiled in — so a desktop pairing by id shows it under
# the ordinary entry.
if [ "$name" = "$DEFAULT_NAME" ]; then
    desktop_file="Desdec.desktop"
    menu_name="Desdec"
else
    desktop_file="$name.desktop"
    menu_name="Desdec ($name)"
fi

# The icon and the entry that names it.
#
# Both are the application's own: the entry comes from the checkout, and the
# icon is asked of the binary that was just installed — the only arrangement
# in which the menu cannot come to show an older mark than the window. Every
# failure here is a warning: the binary is installed and runs, and a desktop
# that would not take the entry is no reason to call the install failed.
install_desktop_entry() {
    local entry="$root/packaging/Desdec.desktop"
    [ -f "$entry" ] || { warn "no $entry — skipping the icon and the menu entry"; return 0; }

    local applications="$data_home/applications"
    local icons="$data_home/icons/hicolor/128x128/apps"
    mkdir -p "$applications" "$icons" || { warn "cannot write under $data_home"; return 0; }

    if ! "$target" --write-icon "$icons/$name.png" 2>/dev/null; then
        warn "$target could not write its icon — skipping the menu entry"
        warn "    (a Desdec older than v0.4.1 does not know --write-icon)"
        return 0
    fi

    # `Exec` and `TryExec` point at what was actually installed, by absolute
    # path: a dock launches an entry without a login shell, so a prefix that
    # this script only just added to a profile is not on the PATH there yet —
    # and would not be for a pinned tile until the next login. `Icon` follows
    # the install name too, so `--name desdec-dev` does not overwrite the
    # release's icon.
    local written="$applications/$desktop_file"
    sed -e "s|^Exec=.*|Exec=$target %f|" \
        -e "s|^TryExec=.*|TryExec=$target|" \
        -e "s|^Icon=.*|Icon=$name|" \
        -e "s|^Name=Desdec$|Name=$menu_name|" \
        "$entry" > "$written" || { warn "cannot write $written"; return 0; }

    # A dock silently ignores an entry it considers malformed, which is the
    # hardest failure here to guess at from the outside. If the validator is
    # installed, let it say so.
    if command -v desktop-file-validate >/dev/null 2>&1; then
        desktop-file-validate "$written" >&2 || warn "$written did not validate — the dock may ignore it"
    fi

    # Best effort, and quiet: a desktop that keeps no such cache is the common
    # case, and saying so every time would be noise.
    command -v update-desktop-database >/dev/null 2>&1 &&
        update-desktop-database "$applications" >/dev/null 2>&1 || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 &&
        gtk-update-icon-cache -qtf "$data_home/icons/hicolor" >/dev/null 2>&1 || true

    say "Added the icon $icons/$name.png and the menu entry $written"
}

# The prefix on the PATH, in the profile of the shell you actually use.
#
# One line, appended once and marked, so running this script again — or after
# a `--prefix` — neither duplicates it nor leaves the profile guessing where
# it came from. A shell already exporting the prefix is left alone, and so is
# a profile that mentions it: this appends, it never edits what is there.
install_path_line() {
    local profile literal
    case "$(basename -- "${SHELL:-/bin/sh}")" in
        zsh)  profile="${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash) profile="$HOME/.bashrc" ;;
        *)    profile="$HOME/.profile" ;;
    esac

    # Written as `$HOME/...` rather than `/home/you/...`: a profile is a file
    # people copy between machines and accounts.
    literal="$prefix"
    case "$prefix" in "$HOME"/*) literal="\$HOME${prefix#"$HOME"}" ;; esac

    if [ -f "$profile" ] && grep -qF -e "$prefix" -e "$literal" -- "$profile"; then
        say "$profile already mentions $prefix — left as it is"
        return 0
    fi

    {
        printf '\n# Desdec — added by insl.sh on %s\n' "$(date +%Y-%m-%d)"
        printf 'export PATH="%s:$PATH"\n' "$literal"
    } >> "$profile" || { warn "cannot append to $profile"; return 0; }

    say "Added $prefix to the PATH in $profile"
    say "    This shell does not see it yet:  export PATH=\"$prefix:\$PATH\""
}

if [ "$desktop" = yes ] && [ "$(uname -s)" = Linux ]; then
    install_desktop_entry
fi

case ":$PATH:" in
    *":$prefix:"*) ;;
    *)
        if [ "$fix_path" = yes ]; then
            install_path_line
        else
            warn "$prefix is not on your PATH. To reach it by name, add:"
            warn "    export PATH=\"$prefix:\$PATH\""
        fi
        ;;
esac

say ""
say "Run it with:  $name              # open the window"
say "              $name /bin/ls      # or analyse a file straight away"
if [ "$desktop" = yes ] && [ "$(uname -s)" = Linux ]; then
    say ""
    say "To pin it: open $menu_name from the menu, then right-click its icon in"
    say "the dock and choose to pin it. Some desktops read new entries only after"
    say "a logout, or after Alt+F2 r under X11."
fi
