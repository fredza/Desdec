#!/usr/bin/env bash
# Signs the assets of a published release, and attaches the signatures.
#
# The private key never leaves this machine: the workflow builds and publishes,
# and the signing happens here afterwards. A key held by a build service is a
# key held by whoever can reach that service, and the point of signing a
# release is to say that *this* person stands behind these bytes.
#
#   scripts/sign-release.sh v0.2.3
#
# Every asset gains a detached `.asc` next to it, and the public key is
# attached too, so a reader has both halves without hunting for either.
set -euo pipefail

KEY="C9A31D0746E065C4E2EA33F608FA1D818A91F329"
SIGNATURE="Frédéric Zawalski @2026 bdom"

tag="${1:-}"
if [ -z "$tag" ]; then
    echo "usage: $0 <tag>   e.g. $0 v0.2.3" >&2
    exit 2
fi

for tool in gh gpg; do
    command -v "$tool" >/dev/null || { echo "$tool is not installed" >&2; exit 1; }
done
gpg --list-secret-keys "$KEY" >/dev/null 2>&1 \
    || { echo "the signing key $KEY is not in this keyring" >&2; exit 1; }

workspace="$(mktemp -d)"
trap 'rm -rf "$workspace"' EXIT

echo "Downloading the assets of $tag"
gh release download "$tag" --dir "$workspace" --pattern '*' --clobber

signed=0
for asset in "$workspace"/*; do
    case "$asset" in
        *.asc|*.sha256) continue ;;
    esac
    echo "Signing $(basename "$asset")"
    gpg --batch --yes --local-user "$KEY" \
        --detach-sign --armor --output "$asset.asc" "$asset"
    gpg --verify "$asset.asc" "$asset"
    signed=$((signed + 1))
done

if [ "$signed" -eq 0 ]; then
    echo "no asset to sign in $tag" >&2
    exit 1
fi

cp desdec-signing-key.asc "$workspace/"
gh release upload "$tag" "$workspace"/*.asc --clobber

echo
echo "$signed assets signed as $SIGNATURE"
echo "Verify with:  gpg --import desdec-signing-key.asc"
echo "              gpg --verify <asset>.asc <asset>"
