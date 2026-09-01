#!/usr/bin/env bash
# Checks that a published release carries everything an installer needs.
#
#   scripts/verify-release.sh              # the latest release
#   scripts/verify-release.sh v0.4.60      # a particular one
#   scripts/verify-release.sh --quick      # names and sizes only, no download
#   scripts/verify-release.sh --partial    # a pre-release carrying Linux alone
#
# A release is not published or not published: it is published with the six
# files a reader needs, or it is published broken. Both have happened here.
# v0.3.0 went out empty, and its job went green — the publish step deleted the
# archives between downloading them and attaching them. Releases 0.4.12 and
# 0.4.13 went out carrying one bare executable, because a tag filter did not
# match a name spelled without its leading `v`. Neither was noticed by the
# forge; both were noticed by someone trying to install them.
#
# So this asks the questions an installer asks, in the order it asks them:
#
#   1. Does the release exist, and is it neither a draft nor a pre-release?
#   2. Does it carry the three archives — Linux, Windows, macOS?
#   3. Does each one have its `.sha256` beside it?
#   4. Does each archive actually hash to what its `.sha256` says?
#
# The fourth is the one that needs the bytes, and it is the only one that can
# tell a release that is *complete* from a release that is *correct*. It is
# what `--quick` leaves out, and the reason `--quick` is not the default.
#
# Nothing here is written outside a temporary directory, which is removed on
# the way out however this exits.
set -euo pipefail

REPO="${DESDEC_REPO:-fredza/Desdec}"

# What every release must carry: three archives, and a checksum for each.
ARCHIVES=(
    "desdec-linux-x86_64-release.tar.gz"
    "desdec-windows-x86_64-release.zip"
    "desdec-macos-aarch64-release.zip"
)

quick=0
# Whether a Linux-only pre-release is what we came to check, rather than a
# release that lost four of its files.
partial=0
tag=""

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'USAGE'
Usage: verify-release.sh [tag] [--quick] [--partial]

  tag       The release to check (default: the latest published one)
  --quick   Check the files are there, without downloading them
  --partial Expect a pre-release carrying the Linux archive alone, which is
            what the workflow publishes before macOS and Windows are built
  -h        Show this message

Exits non-zero when the release is missing a file or an archive does not
hash to what its checksum says.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --quick) quick=1; shift ;;
        --partial) partial=1; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) die "unknown option: $1" ;;
        *) tag="$1"; shift ;;
    esac
done

command -v gh >/dev/null || die "gh is needed to ask GitHub about a release"
command -v sha256sum >/dev/null || command -v shasum >/dev/null \
    || die "neither sha256sum nor shasum is available"

# `gh release view` without a tag is the latest published release, which is
# what a reader running the installer gets.
if [ -z "$tag" ]; then
    tag="$(gh release view --repo "$REPO" --json tagName -q .tagName)" \
        || die "no published release in $REPO"
    say "latest release: $tag"
else
    say "release: $tag"
fi

# A draft is invisible to an installer and a pre-release is not offered as an
# update, so either one silently means "nothing was published".
state="$(gh release view "$tag" --repo "$REPO" --json isDraft,isPrerelease \
    -q '"\(.isDraft) \(.isPrerelease)"')" || die "$tag is not a release of $REPO"
case "$state" in
    "true "*) die "$tag is a draft: nothing can install it" ;;
    *" true")
        # Being a pre-release is the *point* under `--partial`: it is what
        # keeps a Linux-only release out of `/releases/latest`, where a Mac
        # would find it and have nothing to install. Said plainly either way,
        # because "pre-release" means two very different things depending on
        # whether anyone meant it.
        if [ "$partial" -eq 1 ]; then
            say "pre-release, as a partial one must be: no running copy is offered it"
        else
            warn "note: $tag is a pre-release, so no running copy is offered it"
        fi
        ;;
    *)
        [ "$partial" -eq 0 ] ||
            die "$tag is a full release: a partial one must stay a pre-release"
        ;;
esac

# What a partial release is allowed to carry, which is the platform the
# workflow builds first and nothing else.
if [ "$partial" -eq 1 ]; then
    ARCHIVES=("desdec-linux-x86_64-release.tar.gz")
fi

published="$(gh release view "$tag" --repo "$REPO" --json assets -q '.assets[].name')"
say ""

missing=0
for archive in "${ARCHIVES[@]}"; do
    for wanted in "$archive" "$archive.sha256"; do
        if printf '%s\n' "$published" | grep -qxF "$wanted"; then
            size="$(gh release view "$tag" --repo "$REPO" --json assets \
                -q ".assets[] | select(.name == \"$wanted\") | .size")"
            printf '  %-46s %10s bytes\n' "$wanted" "$size"
            # A file of nothing is attached exactly like a file of something.
            [ "${size:-0}" -gt 0 ] || { warn "    ^ empty"; missing=$((missing + 1)); }
        else
            printf '  %-46s %s\n' "$wanted" "MISSING"
            missing=$((missing + 1))
        fi
    done
done

extra="$(printf '%s\n' "$published" | grep -vxF -f <(
    for archive in "${ARCHIVES[@]}"; do printf '%s\n%s.sha256\n' "$archive" "$archive"; done
) || true)"
if [ -n "$extra" ]; then
    say ""
    say "also attached, which this does not check:"
    printf '  %s\n' $extra
fi

say ""
expected=$((${#ARCHIVES[@]} * 2))
if [ "$missing" -gt 0 ]; then
    die "$missing of the $expected files this release must carry are missing or empty"
fi
say "all $expected files are there"

if [ "$quick" -eq 1 ]; then
    say "--quick: the archives were not downloaded, so nothing was hashed"
    exit 0
fi

# The bytes themselves. Downloaded into a directory of their own, removed on
# the way out: this checks a release, it does not install one.
workspace="$(mktemp -d)"
trap 'rm -rf "$workspace"' EXIT
say ""
say "downloading and hashing:"

hash_of() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

wrong=0
for archive in "${ARCHIVES[@]}"; do
    gh release download "$tag" --repo "$REPO" --dir "$workspace" \
        --pattern "$archive" --pattern "$archive.sha256" --clobber 2>/dev/null \
        || { warn "  $archive: could not be downloaded"; wrong=$((wrong + 1)); continue; }

    # The checksum file names the archive it describes, and a release carries
    # one per platform: taking the first word of the first line would happily
    # check an archive against another platform's hash.
    expected="$(awk -v name="$archive" \
        '$NF == name || $NF == "*" name { print tolower($1); exit }' \
        "$workspace/$archive.sha256")"
    if [ -z "$expected" ]; then
        warn "  $archive: its .sha256 does not name it"
        wrong=$((wrong + 1))
        continue
    fi
    found="$(hash_of "$workspace/$archive")"
    if [ "$found" = "$expected" ]; then
        printf '  %-46s ok\n' "$archive"
    else
        printf '  %-46s MISMATCH\n' "$archive"
        warn "    published: $expected"
        warn "    found:     $found"
        wrong=$((wrong + 1))
    fi
done

say ""
if [ "$wrong" -gt 0 ]; then
    die "$wrong of the three archives do not match what the release publishes"
fi
say "$tag is complete and every archive matches its checksum"
