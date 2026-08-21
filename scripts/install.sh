#!/usr/bin/env bash
# Installs Desdec from a published release, or builds it from source.
#
#   scripts/install.sh                    # the latest release, into ~/.local/bin
#   scripts/install.sh --version v0.3.36  # a particular one
#   scripts/install.sh --prefix /usr/local/bin
#   scripts/install.sh --from-source      # build it here instead
#
# It downloads the archive for this machine, checks its SHA-256 *and* its
# signature, and only then puts the binary anywhere. The two checks answer
# different questions — the checksum says the download is intact, the
# signature says who produced it — and a release that fails either one is
# thrown away rather than installed with a warning printed above it.
#
# Nothing here needs root unless the prefix does, nothing is written outside
# the prefix, and no shell profile is edited: if the prefix is not on the
# PATH, the script says so and leaves the line for you to add.
set -euo pipefail

REPO="fredza/Desdec"
KEY_FINGERPRINT="C9A31D0746E065C4E2EA33F608FA1D818A91F329"
BINARY="desdec-app"          # what the archive holds
DEFAULT_NAME="desdec"        # what it is called once installed

prefix="${DESDEC_PREFIX:-$HOME/.local/bin}"
name="$DEFAULT_NAME"
tag=""
from_source=0
allow_prerelease=0
skip_signature=0

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
  --skip-signature  Check the SHA-256 only, not the GPG signature
  -h, --help        Show this message

The environment variable DESDEC_PREFIX sets the default prefix.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) tag="${2:-}"; [ -n "$tag" ] || die "--version needs a tag"; shift 2 ;;
        --prefix)  prefix="${2:-}"; [ -n "$prefix" ] || die "--prefix needs a directory"; shift 2 ;;
        --name)    name="${2:-}"; [ -n "$name" ] || die "--name needs a name"; shift 2 ;;
        --pre)     allow_prerelease=1; shift ;;
        --from-source) from_source=1; shift ;;
        --skip-signature) skip_signature=1; shift ;;
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
    say "Installed $target"

    case ":$PATH:" in
        *":$prefix:"*) ;;
        *)
            say ""
            warn "$prefix is not on your PATH. To reach it by name, add:"
            warn "    export PATH=\"$prefix:\$PATH\""
            ;;
    esac
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

    if [ "$skip_signature" -eq 1 ]; then
        warn "Skipping the signature check: this says the download is intact, not who made it."
    elif ! command -v gpg >/dev/null 2>&1; then
        die "gpg is not installed, so the signature cannot be checked. Install gpg, or pass --skip-signature to accept the checksum alone."
    else
        say "Checking the signature"
        fetch "$base/$asset.asc" "$workspace/$asset.asc" \
            || die "$tag publishes no signature for $asset; pass --skip-signature to install it anyway"
        # The key is verified against a fingerprint written into this script,
        # not against whatever the release happens to ship: a key downloaded
        # beside a signature only proves the two came from the same place.
        fetch "https://raw.githubusercontent.com/$REPO/main/desdec-signing-key.asc" \
              "$workspace/desdec-signing-key.asc" \
            || die "could not fetch the public key"

        local keyring="$workspace/keyring.gpg"
        gpg --batch --quiet --no-default-keyring --keyring "$keyring" \
            --import "$workspace/desdec-signing-key.asc" 2>/dev/null \
            || die "the public key could not be read"
        gpg --batch --no-default-keyring --keyring "$keyring" \
            --with-colons --fingerprint 2>/dev/null \
            | grep -q "^fpr:::::::::$KEY_FINGERPRINT:" \
            || die "the published key is not $KEY_FINGERPRINT — stopping"
        gpg --batch --quiet --no-default-keyring --keyring "$keyring" \
            --verify "$workspace/$asset.asc" "$workspace/$asset" 2>/dev/null \
            || die "$asset is not signed by $KEY_FINGERPRINT — stopping"
        say "Signed by ${KEY_FINGERPRINT}"
    fi

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
}

if [ "$from_source" -eq 1 ]; then
    build_from_source
else
    install_from_release
fi

say ""
say "Run it with:  $name              # open the window"
say "              $name /bin/ls      # or analyse a file straight away"
