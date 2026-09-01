#!/usr/bin/env bash
# Installs Desdec from a published release, or builds it from source.
#
#   scripts/install.sh                    # the latest release, into ~/.local/bin
#   scripts/install.sh --version v0.4.65  # a particular one
#   scripts/install.sh --prefix /usr/local/bin
#   scripts/install.sh --from-source      # build it here instead
#   scripts/install.sh --no-desktop       # the binary alone, no menu entry
#
# On Linux it also puts Desdec in the desktop menu, with the icon the binary
# writes itself — the same three things `insl.sh` gives a checkout. Windows
# gets the same from `install.ps1`, which writes a Start menu shortcut; this
# script installs the binary alone there, since it only runs under MSYS in the
# first place. On macOS it writes an application bundle into `~/Applications`,
# which is what puts a program in the Dock, in Spotlight and in Launchpad.
# `--no-desktop` skips whichever of the three this machine would have got.
#
# It downloads the archive for this machine, checks its SHA-256, and only then
# puts the binary anywhere. A release whose checksum does not match is thrown
# away rather than installed with a warning printed above it.
#
# The checksum says the download is intact and nothing more. Releases are not
# signed from v0.4.1 on, so that is the whole of the check; up to v0.4.0 they
# were, and those archives keep the detached `.asc` next to them for anyone
# who wants to check one with `gpg` and the key at the root of the repository.
#
# Nothing here needs root unless the prefix does, nothing is written outside
# the prefix, and no shell profile is edited: if the prefix is not on the
# PATH, the script says so and leaves the line for you to add.
set -euo pipefail

REPO="fredza/Desdec"
BINARY="desdec-app"          # what the archive holds
DEFAULT_NAME="desdec"        # what it is called once installed

prefix="${DESDEC_PREFIX:-$HOME/.local/bin}"
name="$DEFAULT_NAME"
tag=""
from_source=0
allow_prerelease=0
desktop=yes
# Where a desktop looks for menu entries and icons, as the XDG base directory
# specification names it.
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
# The checkout this script was run from, when it was run from one. It is also
# fetched on its own and piped to a shell, and then there is no `packaging/`
# next to it — hence the two other ways of finding the menu entry below.
script_dir=""
# Set by install_desktop_entry when it wrote one, and read by the last message.
menu_entry_added=""
# What the installed binary answers to `--version`, without the `desdec v`.
reported_version=""
if [ -n "${BASH_SOURCE[0]:-}" ]; then
    script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd)" || script_dir=""
fi

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'USAGE'
Usage: install.sh [options]

  --version <tag>   Install this release (default: the latest one)
  --pre             Consider pre-releases when choosing the latest
  --prefix <dir>    Install into this directory (default: ~/.local/bin)
  --name <name>     Install under this name (default: desdec)
  --from-source     Build from source with cargo instead of downloading
  --no-desktop      Do not add the menu entry, the icon or the macOS
                    application bundle
  -h, --help        Show this message

DESDEC_PREFIX sets the default prefix, XDG_DATA_HOME the directory the icon
and the menu entry are written under, and DESDEC_APPLICATIONS the directory
the macOS bundle goes into.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) tag="${2:-}"; [ -n "$tag" ] || die "--version needs a tag"; shift 2 ;;
        --prefix)  prefix="${2:-}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift 2 ;;
        --name)    name="${2:-}"; [ -n "$name" ] || die "--name needs a name"; shift 2 ;;
        --pre)     allow_prerelease=1; shift ;;
        --from-source) from_source=1; shift ;;
        --no-desktop) desktop=no; shift ;;
        # Accepted and ignored: it was the way to install an unsigned release
        # back when a missing signature stopped the script. Nothing is signed
        # now, so a command that still carries it keeps working rather than
        # failing on an unknown option.
        --skip-signature) shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown option: $1" ;;
    esac
done

need() { command -v "$1" >/dev/null 2>&1 || die "$1 is needed and is not installed"; }

# One downloader, chosen once. `curl` on macOS and most Linux images, `wget`
# where curl is absent; asking for both would rule out machines that can
# perfectly well fetch a file.
if command -v curl >/dev/null 2>&1; then
    fetch() { curl --fail --silent --show-error --location --retry 3 --output "$2" "$1"; }
    fetch_stdout() { curl --fail --silent --show-error --location --retry 3 "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget --quiet --output-document "$2" "$1"; }
    fetch_stdout() { wget --quiet --output-document - "$1"; }
else
    die "neither curl nor wget is installed"
fi

# Which archive belongs to this machine. The workflow publishes three, and a
# platform that has none is told so by name instead of being handed the wrong
# one: --from-source builds it here in a few minutes.
platform_asset() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os/$arch" in
        Linux/x86_64|Linux/amd64)          echo "desdec-linux-x86_64-release.tar.gz" ;;
        Darwin/arm64|Darwin/aarch64)       echo "desdec-macos-aarch64-release.zip" ;;
        MINGW*/x86_64|MSYS*/x86_64|CYGWIN*/x86_64) echo "desdec-windows-x86_64-release.zip" ;;
        *) return 1 ;;
    esac
}

# GitHub answers with JSON and this script will not depend on jq being
# installed to read one field out of it. jq if it is there, python3 if it is
# not, and a plain sed as the last resort.
json_field() {
    local body="$1"
    if command -v jq >/dev/null 2>&1; then
        printf '%s' "$body" \
            | jq -er 'if type == "array" then .[0].tag_name else .tag_name end' 2>/dev/null \
            && return 0
    fi
    if command -v python3 >/dev/null 2>&1; then
        printf '%s' "$body" | python3 -c '
import json, sys
data = json.load(sys.stdin)
if isinstance(data, list):
    data = data[0] if data else {}
print(data.get("tag_name", ""))
' 2>/dev/null && return 0
    fi
    printf '%s' "$body" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1
}

latest_tag() {
    local url body
    if [ "$allow_prerelease" -eq 1 ]; then
        url="https://api.github.com/repos/$REPO/releases?per_page=1"
    else
        url="https://api.github.com/repos/$REPO/releases/latest"
    fi
    body="$(fetch_stdout "$url")" || die "could not ask GitHub for the latest release"
    json_field "$body"
}

# BSD mktemp — the one on macOS — wants a template where GNU mktemp is happy
# without one, and this script has to work on both.
workspace="$(mktemp -d 2>/dev/null || mktemp -d -t desdec)"
trap 'rm -rf "$workspace"' EXIT

# Puts one built binary in place. Written to a temporary name in the target
# directory and then moved, so a running copy is replaced atomically rather
# than truncated under its own feet.
install_binary() {
    local built="$1" target="$prefix/$name"
    mkdir -p "$prefix"
    chmod +x "$built"
    local staged="$prefix/.$name.$$"
    cp "$built" "$staged"
    chmod +x "$staged"
    mv -f "$staged" "$target"

    # Ask it what it is. Up to v0.4.1 the first argument was taken as a file
    # to analyse whatever it said, so this opened a window instead of
    # answering, and the installer had no way to tell a good archive from a
    # truncated one. An older binary that still behaves that way must not
    # leave a window on the screen, hence the timeout and the discarded
    # output; and the check only ever adds to the message, never stops the
    # install, because a binary this script has just verified by checksum is
    # not made wrong by an unhelpful `--version`.
    local reported=""
    if command -v timeout >/dev/null 2>&1; then
        reported="$(timeout 5 "$target" --version 2>/dev/null || true)"
    fi
    # Kept for the macOS bundle's plist, which states a version and has no
    # other honest source for one: the tag names the release, the binary names
    # itself. Empty when the answer was not obtained, and the keys are then
    # left out rather than filled with a guess.
    case "$reported" in
        desdec\ v*)
            reported_version="${reported#desdec v}"
            reported_version="${reported_version%% *}"
            ;;
        *) reported_version="" ;;
    esac
    case "$reported" in
        desdec\ v*) say "Installed $target — $reported" ;;
        *)           say "Installed $target" ;;
    esac

    case ":$PATH:" in
        *":$prefix:"*) ;;
        *)
            say ""
            warn "$prefix is not on your PATH. To reach it by name, add:"
            warn "    export PATH=\"$prefix:\$PATH\""
            ;;
    esac
}

# The menu entry, from the closest place that has one, and nothing written
# into this script.
#
# Three sources, in the order of how surely each matches the binary just
# installed: the archive itself, which carries `Desdec.desktop` from the first
# release built after 2026-09-01; the checkout this script was run from; and,
# failing both, the copy the repository publishes **at the tag that was
# installed** — not at `main`, whose entry may already describe a Desdec this
# machine does not have.
#
# Spelling the entry out here instead would be a second copy of
# `packaging/Desdec.desktop`, and the two would drift apart with nobody
# noticing: the file carries the application id a Wayland compositor pairs the
# window by, the MIME types, and three languages of its own.
desktop_entry_source() {
    local candidate
    for candidate in "$@"; do
        if [ -n "$candidate" ] && [ -f "$candidate" ]; then
            printf '%s' "$candidate"
            return 0
        fi
    done

    [ -n "$tag" ] || return 1
    fetch "https://raw.githubusercontent.com/$REPO/$tag/packaging/Desdec.desktop" \
        "$workspace/Desdec.desktop" 2>/dev/null || return 1
    # It has to look like one. A tag whose tree has no `packaging/` answers
    # with a page rather than nothing at all on some mirrors, and a menu given
    # that file shows an entry that does nothing, without a word anywhere.
    grep -q '^\[Desktop Entry\]' "$workspace/Desdec.desktop" 2>/dev/null || return 1
    printf '%s' "$workspace/Desdec.desktop"
}

# The icon and the entry that names it, on Linux and nowhere else.
#
# The icon is asked of the binary that was just installed rather than carried
# beside the entry: it is the only arrangement in which the menu cannot come
# to show an older mark than the window. Every failure here is a warning —
# the binary is installed and runs, and a desktop that would not take the
# entry is no reason to call the install failed.
install_desktop_entry() {
    [ "$desktop" = yes ] || return 0
    [ "$(uname -s)" = Linux ] || return 0

    local entry
    entry="$(desktop_entry_source "$@")" || {
        warn ""
        warn "No menu entry: this release carries no Desdec.desktop and none could be"
        warn "fetched. The binary is installed; re-run from a checkout, or use"
        warn "scripts/insl.sh, to get the icon and the entry as well."
        return 0
    }

    # The file's own name is what a Wayland compositor matches against the
    # window's application id, which is `Desdec`: the ordinary install must
    # therefore be `Desdec.desktop`, or the window opens under a generic icon
    # and the dock pins a second, empty tile beside the running one. An
    # install under another name gets a file and a menu name of its own so it
    # does not overwrite the ordinary one.
    local desktop_file menu_name
    if [ "$name" = "$DEFAULT_NAME" ]; then
        desktop_file="Desdec.desktop"
        menu_name="Desdec"
    else
        desktop_file="$name.desktop"
        menu_name="Desdec ($name)"
    fi

    local target="$prefix/$name"
    local applications="$data_home/applications"
    local icons="$data_home/icons/hicolor/128x128/apps"
    mkdir -p "$applications" "$icons" || { warn "cannot write under $data_home"; return 0; }

    if ! "$target" --write-icon "$icons/$name.png" 2>/dev/null; then
        warn "$target could not write its icon — skipping the menu entry"
        warn "    (a Desdec older than v0.4.1 does not know --write-icon)"
        return 0
    fi

    # `Exec` and `TryExec` point at what was actually installed, by absolute
    # path: a dock launches an entry without a login shell, so a prefix this
    # script has only just told you to add to your profile is not on the PATH
    # there — and would not be for a pinned tile until the next login. `Icon`
    # follows the install name too, so `--name desdec-dev` does not overwrite
    # the release's icon.
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
    menu_entry_added="$menu_name"
}

# The macOS application bundle.
#
# A binary in `~/.local/bin` is reachable from a terminal and from nowhere
# else: macOS puts a program in the Dock, in Spotlight and in Launchpad by way
# of a bundle, which is a directory with a plist in it. This writes the
# smallest one that is a real application — the executable, the icon, and the
# statement of what the two are.
#
# The binary is copied into the bundle rather than symlinked out of it. A
# bundle whose executable points outside itself is not what any part of macOS
# expects to find, and a reader who removed the prefix would be left with an
# application that opens onto nothing. Both copies are written by this same
# run, so they cannot start out disagreeing.
#
# Every failure here is a warning: the binary is installed and runs from a
# terminal, and a Finder that would not take the bundle is no reason to call
# the install failed.
install_app_bundle() {
    [ "$desktop" = yes ] || return 0
    [ "$(uname -s)" = Darwin ] || return 0

    local target="$prefix/$name"
    local label="Desdec"
    # The bundle identifier follows the install name, because macOS identifies
    # an application by it and by nothing else: two bundles sharing one id are
    # one application as far as Launch Services is concerned, and it may open
    # either when asked for the other. Anything a bundle id may not hold
    # becomes a dash, so `--name desdec.dev` cannot invent a level of the
    # reversed domain.
    local identifier="io.github.fredza.desdec"
    if [ "$name" != "$DEFAULT_NAME" ]; then
        label="Desdec ($name)"
        identifier="io.github.fredza.$(printf '%s' "$name" | tr -c 'A-Za-z0-9-' '-')"
    fi
    local app="${DESDEC_APPLICATIONS:-$HOME/Applications}/$label.app"
    local contents="$app/Contents"

    mkdir -p "$contents/MacOS" "$contents/Resources" || {
        warn "cannot write $app — skipping the application bundle"
        return 0
    }

    # The icon is asked of the binary that was just installed, as an `.icns`,
    # which is the only kind of file the Dock reads. A Desdec from before
    # 2026-09-01 writes a PNG whatever the extension, and a bundle carrying
    # one shows the blank sheet macOS uses for an application with no icon —
    # so the four bytes are read back, and a bundle is written without an icon
    # rather than with a wrong one.
    local icon="$contents/Resources/$name.icns"
    local icon_key=""
    if "$target" --write-icon "$icon" 2>/dev/null && [ "$(head -c 4 "$icon" 2>/dev/null)" = "icns" ]; then
        icon_key="    <key>CFBundleIconFile</key><string>$name</string>"
    else
        rm -f "$icon"
        warn "$target could not write an .icns — the bundle goes without an icon"
        warn "    (a Desdec from before 2026-09-01 writes a PNG whatever the extension)"
    fi

    local version_keys=""
    if [ -n "$reported_version" ]; then
        version_keys="    <key>CFBundleShortVersionString</key><string>$reported_version</string>
    <key>CFBundleVersion</key><string>$reported_version</string>"
    fi

    cp "$target" "$contents/MacOS/$name" || {
        warn "cannot copy the binary into $app — skipping the application bundle"
        return 0
    }
    chmod +x "$contents/MacOS/$name"

    # `CFBundleIconFile` names the file without its extension, which is the
    # one field of this plist that is easy to write wrongly and that gives no
    # error when it is: the bundle simply shows the blank sheet.
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

    # Nothing here is notarised, and Gatekeeper refuses a downloaded binary
    # that is not. The attribute is only set on files a browser wrote, so this
    # is a no-op when curl fetched the archive.
    xattr -dr com.apple.quarantine "$app" 2>/dev/null || true
    # The Finder notices a bundle whose directory changed. Without this it can
    # keep showing the previous icon until it is asked again.
    touch "$app" 2>/dev/null || true

    say "Added the application $app"
    menu_entry_added="$label"
}

build_from_source() {
    need cargo
    need git
    local source_dir
    # Run from a checkout and it builds that checkout — the point of asking
    # for a source build is usually to install what is in front of you.
    if [ -f "Cargo.toml" ] && grep -q 'desdec-app' Cargo.toml 2>/dev/null; then
        source_dir="$PWD"
        say "Building the checkout in $source_dir"
    else
        source_dir="$workspace/Desdec"
        say "Cloning $REPO"
        if [ -n "$tag" ]; then
            git clone --depth 1 --branch "$tag" "https://github.com/$REPO.git" "$source_dir" --quiet
        else
            git clone --depth 1 "https://github.com/$REPO.git" "$source_dir" --quiet
        fi
    fi
    say "Building with cargo — this takes a few minutes"
    ( cd "$source_dir" && cargo build --locked --release -p "$BINARY" )
    install_binary "$source_dir/target/release/$BINARY"
    # The tree it just built is right here, so its own entry is the one that
    # matches the binary exactly — no download, whatever the tag says.
    install_desktop_entry "$source_dir/packaging/Desdec.desktop"
    install_app_bundle
}

install_from_release() {
    local asset
    asset="$(platform_asset)" || die "no published archive for $(uname -s)/$(uname -m); use --from-source"

    [ -n "$tag" ] || tag="$(latest_tag)"
    [ -n "$tag" ] || die "could not work out which release to install"

    local base="https://github.com/$REPO/releases/download/$tag"
    say "Installing Desdec $tag ($asset)"

    fetch "$base/$asset" "$workspace/$asset" \
        || die "$tag has no $asset — check the release page or use --from-source"
    fetch "$base/$asset.sha256" "$workspace/$asset.sha256" \
        || die "$tag publishes no checksum for $asset; refusing to install it unchecked"

    # The checksum file names the archive with its own path in it, so the
    # check is run from the directory holding both.
    say "Checking the SHA-256"
    (
        cd "$workspace"
        # sha256sum on Linux, shasum on macOS. Neither is asked for an
        # option beyond --check: the output is dropped here instead, because
        # the two disagree about how to be quiet.
        if command -v sha256sum >/dev/null 2>&1; then
            sha256sum --check "$asset.sha256" >/dev/null 2>&1
        elif command -v shasum >/dev/null 2>&1; then
            shasum -a 256 --check "$asset.sha256" >/dev/null 2>&1
        else
            die "neither sha256sum nor shasum is installed"
        fi
    ) || die "the SHA-256 of $asset does not match what $tag published"

    local unpacked="$workspace/unpacked"
    mkdir -p "$unpacked"
    case "$asset" in
        *.tar.gz)
            need tar
            tar -xzf "$workspace/$asset" -C "$unpacked"
            ;;
        *.zip)
            if command -v unzip >/dev/null 2>&1; then
                unzip -q "$workspace/$asset" -d "$unpacked"
            elif command -v python3 >/dev/null 2>&1; then
                python3 -m zipfile -e "$workspace/$asset" "$unpacked"
            else
                die "neither unzip nor python3 is installed, so the archive cannot be opened"
            fi
            ;;
    esac

    # macOS archives keep the enclosing directory, Linux ones do not, so the
    # binary is looked for rather than assumed to be at a fixed path.
    local built
    built="$(find "$unpacked" -type f \( -name "$BINARY" -o -name "$BINARY.exe" \) -print -quit)"
    [ -n "$built" ] || die "the archive does not hold $BINARY"

    case "$(uname -s)" in
        Darwin) name="${name%.exe}" ;;
        MINGW*|MSYS*|CYGWIN*) case "$name" in *.exe) ;; *) name="$name.exe" ;; esac ;;
    esac

    install_binary "$built"

    if [ "$(uname -s)" = "Darwin" ]; then
        # Nothing here is notarised, and Gatekeeper refuses a downloaded
        # binary that is not. The attribute is only set on files a browser
        # wrote, so removing it is a no-op when curl fetched the archive.
        xattr -d com.apple.quarantine "$prefix/$name" 2>/dev/null || true
    fi

    # The archive first — a release that carries the entry needs no second
    # request — then the checkout this script may be sitting in.
    install_desktop_entry \
        "$(find "$unpacked" -type f -name 'Desdec.desktop' -print -quit)" \
        "${script_dir:+$script_dir/packaging/Desdec.desktop}"
    install_app_bundle
}

if [ "$from_source" -eq 1 ]; then
    build_from_source
else
    install_from_release
fi

say ""
say "Run it with:  $name              # open the window"
say "              $name /bin/ls      # or analyse a file straight away"

if [ -n "$menu_entry_added" ] && [ "$(uname -s)" = Darwin ]; then
    say ""
    say "To keep it in the Dock: open $menu_entry_added, then right-click its icon"
    say "there and choose Options, Keep in Dock."
elif [ -n "$menu_entry_added" ]; then
    say ""
    say "To pin it: open $menu_entry_added from the menu, then right-click its icon in"
    say "the dock and choose to pin it. Some desktops read new entries only after"
    say "a log out and back in."
fi
