#!/usr/bin/env bash
# Point latest-release-<platform> at this release's default runtime, keeping one
# revision, when this run's tag is the newest vX.Y.Z release tag. A run for any
# other tag, such as a rerun of an older release or a release candidate,
# uploads but leaves the pin alone.
#
# Usage: pin-latest-release.sh PLATFORM RUNTIME_STORE_PATH
# Needs GITHUB_REF, CACHIX_AUTH_TOKEN and an `origin` remote. The caller's
# per-platform concurrency group keeps two release runs from pinning at once.
set -euo pipefail

platform=$1
runtime=$2
ref=${GITHUB_REF:?}
pin=latest-release-$platform
release='^refs/tags/v[0-9]+[.][0-9]+[.][0-9]+$'

case $runtime in
  /nix/store/?*) ;;
  *)
    echo "::error::No runtime store path to pin as $pin."
    exit 1
    ;;
esac

if ! [[ $ref =~ $release ]]; then
  echo "::notice::Skipping $pin: $ref is not a vX.Y.Z release tag."
  exit 0
fi

# Read the remote's tags now, so a tag pushed after this run started counts.
# Only exact top-level vX.Y.Z tags qualify: ls-remote patterns match ref tails,
# and other v* names (rc tags, vendor-x) would otherwise outrank releases.
newest=$(git ls-remote --tags --refs --sort=-v:refname origin 'refs/tags/v*' |
  awk -F '\t' -v release="$release" '$2 ~ release { print $2; exit }')
if [ -z "$newest" ]; then
  echo '::error::Found no vX.Y.Z tags on origin; cannot tell whether this release is the newest.'
  exit 1
fi
if [ "$ref" != "$newest" ]; then
  echo "::notice::Skipping $pin: ${ref#refs/tags/} is not the newest release tag (${newest#refs/tags/} is)."
  exit 0
fi

cachix pin eon-yazelix "$pin" "$runtime" --keep-revisions 1
