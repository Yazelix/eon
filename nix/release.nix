{ pkgs, bundle, installer, noticeCheck, source, sourceCheck }:
# The files a GitHub release carries, less the maintainer's SHA256SUMS.sig: the bundle
# archive, its source companion, the static installer and their SHA256SUMS.
pkgs.runCommand "eon-release" {
  nativeBuildInputs = [ pkgs.gnutar pkgs.zstd pkgs.jq ];
} ''
  test -e ${noticeCheck}
  test -e ${sourceCheck}
  # Compare to the inventory actually shipped, not a second evaluation's report.
  tar --use-compress-program=zstd -xOf ${bundle}/*.tar.zst \
    --wildcards '*/THIRD-PARTY-NOTICES/inventory.json' > binary-inventory.json
  tar --use-compress-program=zstd -xOf ${source}/*.tar.zst \
    --wildcards '*/source-index.json' > source-index.json
  if ! jq -e --arg hash "$(sha256sum binary-inventory.json | cut -d ' ' -f 1)" \
    '.inventory_sha256 == $hash' source-index.json > /dev/null; then
    echo "source companion inventory does not match the binary archive notices" >&2
    exit 1
  fi
  mkdir "$out"
  cp ${bundle}/*.tar.zst "$out"/
  cp ${source}/*.tar.zst "$out"/
  cp ${installer}/bin/eon-install "$out"/eon-install-x86_64-linux
  cd "$out"
  sha256sum -- * > SHA256SUMS
''
