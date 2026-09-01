#!/usr/bin/env bash
# Builds the checkout you are standing in and installs it.
#
#   scripts/install-local.sh                     # into ~/.local/bin/desdec
#   scripts/install-local.sh --prefix /usr/local/bin
#   scripts/install-local.sh --name desdec-dev   # beside an installed release
#   scripts/install-local.sh --no-desktop        # binary only, no menu entry
#
# On Linux it also puts Desdec in the desktop menu: the entry from
# `packaging/`, and the icon asked of the binary that was just installed, both
# under the user's own data directory. On macOS it writes the application
# bundle that does the same job there — `~/Applications/Desdec.app`, with the
# icon asked of the same binary. Nothing there needs root either, and
# `--no-desktop` skips whichever of the two this machine would have got.
#
# No network, no release, no checksum — there is nothing to check, because
# nothing was downloaded. The binary comes out of the sources in front of you,
# which is the whole difference between this and `install.sh`: that one is for
# someone who wants the published Desdec, this one is for someone who has the
# repository open and wants what is in it.
#
# `install.sh --from-source` also builds, but it exists inside a script whose
# subject is releases: it can clone, it can pick a tag, and it carries the
# machinery for both. This is the short path.
#
# Nothing here needs root unless the prefix does, nothing is written outside
# the prefix and the usual `target/` directory, and no shell profile is
# edited: if the prefix is not on the PATH, the script says so and leaves the
# line for you to add.
set -euo pipefail

BINARY="desdec-app"          # what cargo builds
DEFAULT_NAME="desdec"        # what it is called once installed

prefix="${DESDEC_PREFIX:-$HOME/.local/bin}"
name="$DEFAULT_NAME"
desktop=yes
# Where a desktop looks for menu entries and icons, as the XDG base directory
# specification names it.
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'USAGE'
Usage: install-local.sh [options]

  --prefix <dir>   Install into this directory (default: ~/.local/bin)
  --name <name>    Install under this name (default: desdec)
  --no-desktop     Do not add the menu entry, its icon or the macOS bundle
  -h, --help       Show this message

The environment variable DESDEC_PREFIX sets the default prefix, and
XDG_DATA_HOME the directory the menu entry and icon are written under.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) prefix="${2:-}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift 2 ;;
        --name)   name="${2:-}"; [ -n "$name" ] || die "--name needs a name"; shift 2 ;;
        --no-desktop) desktop=no; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $1" ;;
    esac
done

command -v cargo >/dev/null 2>&1 || die "cargo is needed and is not installed"

# The repository is found from this script's own location, not from the
# working directory: `~/bin/install-local.sh` typed from anywhere must build
# the checkout the script lives in, and not whatever happens to be around.
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
root="$(dirname -- "$script_dir")"
[ -f "$root/Cargo.toml" ] || die "$root does not look like the Desdec checkout"
grep -q "$BINARY" "$root/Cargo.toml" || die "$root/Cargo.toml does not mention $BINARY"

say "Building $root — this takes a few minutes the first time"
# `--locked` so the build uses the lockfile as committed. A local install that
# quietly resolved newer dependencies would not be the checkout you are
# looking at, which is the one thing this script promises.
( cd "$root" && cargo build --locked --release -p "$BINARY" )

built="$root/target/release/$BINARY"
[ -x "$built" ] || die "cargo reported success but $built is not there"

mkdir -p "$prefix"
target="$prefix/$name"
# Written under a temporary name in the target directory and then moved, so a
# running copy is replaced atomically rather than truncated under its feet.
staged="$prefix/.$name.$$"
cp "$built" "$staged"
chmod +x "$staged"
mv -f "$staged" "$target"

# Ask it what it is, which is also a check that what was just installed runs
# at all. Bounded, because a binary from before v0.4.1 takes the argument for
# a file to analyse and opens a window instead of answering.
reported=""
if command -v timeout >/dev/null 2>&1; then
    reported="$(timeout 5 "$target" --version 2>/dev/null || true)"
fi
case "$reported" in
    desdec\ v*) say "Installed $target — $reported" ;;
    *)          say "Installed $target" ;;
esac

# The menu entry, and the icon it names.
#
# Both are the application's own: the entry comes from the checkout that was
# just built, and the icon is asked of the binary that was just installed —
# which is the only arrangement in which the menu cannot come to show an older
# mark than the window. Every failure here is a warning and not an error: the
# binary is installed and works, and a desktop that would not take the entry
# is not a reason to say the install failed.
install_desktop_entry() {
    local entry="$root/packaging/Desdec.desktop"
    [ -f "$entry" ] || { warn "no $entry — skipping the menu entry"; return 0; }

    local applications="$data_home/applications"
    local icons="$data_home/icons/hicolor/128x128/apps"
    mkdir -p "$applications" "$icons" || { warn "cannot write under $data_home"; return 0; }

    # Asked of the binary itself. A copy committed beside the entry would be a
    # second drawing of the same mark, and the two would drift.
    if ! "$target" --write-icon "$icons/$name.png" 2>/dev/null; then
        warn "$target could not write its icon — skipping the menu entry"
        warn "    (a Desdec older than v0.4.2 does not know --write-icon)"
        return 0
    fi

    # `Exec` and `TryExec` point at what was actually installed, under the name
    # it was installed as: a prefix that is not on the PATH would otherwise
    # give a menu entry that does nothing when pressed. `Icon` follows the
    # name too, so `--name desdec-dev` does not overwrite the release's icon.
    local written="$applications/$desktop_file"
    sed -e "s|^Exec=.*|Exec=$target %f|" \
        -e "s|^TryExec=.*|TryExec=$target|" \
        -e "s|^Icon=.*|Icon=$name|" \
        -e "s|^Name=Desdec$|Name=$menu_name|" \
        "$entry" > "$written" || { warn "cannot write $written"; return 0; }

    # Best effort, and quiet: a desktop that keeps no such cache is the common
    # case, and saying so every time would be noise.
    command -v update-desktop-database >/dev/null 2>&1 &&
        update-desktop-database "$applications" >/dev/null 2>&1 || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 &&
        gtk-update-icon-cache -qtf "$data_home/icons/hicolor" >/dev/null 2>&1 || true

    say "Added the menu entry $written"
}

# The file's own name is what a Wayland compositor matches against the window's
# application id, which is `Desdec`: the ordinary install must therefore be
# `Desdec.desktop` or the window opens under a generic icon.
#
# An install under another name gets a file and a menu name of its own, so it
# does not overwrite the ordinary one and can be told from it in the menu. Its
# *window* still carries the id `Desdec` — the application id is compiled in —
# so a desktop pairing by that id will show it under the ordinary entry. Two
# builds of one program is what that costs, and it costs nothing else.
if [ "$name" = "$DEFAULT_NAME" ]; then
    desktop_file="Desdec.desktop"
    menu_name="Desdec"
else
    desktop_file="$name.desktop"
    menu_name="Desdec ($name)"
fi

# The macOS application bundle, which is what a menu entry is on that side:
# a binary in `~/.local/bin` is reachable from a terminal and from nowhere
# else, and macOS puts a program in the Dock, in Spotlight and in Launchpad by
# way of a bundle. The same one `install.sh` writes for a downloaded release,
# from the binary this checkout just built.
install_app_bundle() {
    local label="$menu_name"
    local identifier="io.github.fredza.desdec"
    if [ "$name" != "$DEFAULT_NAME" ]; then
        # macOS identifies an application by its bundle identifier and by
        # nothing else, so a side install needs one of its own or Launch
        # Services may open either for the other. Anything an identifier may
        # not hold becomes a dash.
        identifier="io.github.fredza.$(printf '%s' "$name" | tr -c 'A-Za-z0-9-' '-')"
    fi
    local app="${DESDEC_APPLICATIONS:-$HOME/Applications}/$label.app"
    local contents="$app/Contents"

    mkdir -p "$contents/MacOS" "$contents/Resources" || {
        warn "cannot write $app — skipping the application bundle"
        return 0
    }

    # Asked of the binary itself, as an `.icns`, which is the only kind of file
    # the Dock reads. A Desdec from before 2026-09-01 writes a PNG whatever the
    # extension, and a bundle carrying one shows the blank sheet macOS uses for
    # an application with no icon — so the four bytes are read back, and the
    # bundle goes without an icon rather than with a wrong one.
    local icon="$contents/Resources/$name.icns"
    local icon_key=""
    if "$target" --write-icon "$icon" 2>/dev/null && [ "$(head -c 4 "$icon" 2>/dev/null)" = "icns" ]; then
        icon_key="    <key>CFBundleIconFile</key><string>$name</string>"
    else
        rm -f "$icon"
        warn "$target could not write an .icns — the bundle goes without an icon"
    fi

    # The version the binary announces, not the one the checkout's Cargo.toml
    # says: it is this binary that will be launched. Absent when `--version`
    # was not asked or did not answer, rather than guessed at.
    local version_keys=""
    case "$reported" in
        desdec\ v*)
            local version="${reported#desdec v}"
            version="${version%% *}"
            version_keys="    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>"
            ;;
    esac

    # Copied in rather than symlinked out: a bundle whose executable points
    # outside itself is not what any part of macOS expects to find, and a
    # rebuilt checkout would otherwise change what the Dock launches without
    # anyone asking it to.
    cp "$target" "$contents/MacOS/$name" || {
        warn "cannot copy the binary into $app — skipping the application bundle"
        return 0
    }
    chmod +x "$contents/MacOS/$name"

    cat > "$contents/Info.plist" <<PLIST || { warn "cannot write $contents/Info.plist"; return 0; }
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$label</string>
    <key>CFBundleDisplayName</key><string>$label</string>
    <key>CFBundleIdentifier</key><string>$identifier</string>
    <key>CFBundleExecutable</key><string>$name</string>
$icon_key
$version_keys
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

    # The Finder notices a bundle whose directory changed; without this it can
    # keep showing the previous icon until it is asked again.
    touch "$app" 2>/dev/null || true
    say "Added the application $app"
}

if [ "$desktop" = yes ] && [ "$(uname -s)" = Linux ]; then
    install_desktop_entry
elif [ "$desktop" = yes ] && [ "$(uname -s)" = Darwin ]; then
    install_app_bundle
fi

case ":$PATH:" in
    *":$prefix:"*) ;;
    *)
        say ""
        warn "$prefix is not on your PATH. To reach it by name, add:"
        warn "    export PATH=\"$prefix:\$PATH\""
        ;;
esac

say ""
say "Run it with:  $name              # open the window"
say "              $name /bin/ls      # or analyse a file straight away"
