{ pkgs, bundle, installer, noticeCheck }:
# The files a GitHub release carries, less the maintainer's SHA256SUMS.sig: the bundle
# archive, the static installer and one SHA256SUMS over both.
pkgs.runCommand "eon-release" { } ''
  test -e ${noticeCheck}
  mkdir "$out"
  cp ${bundle}/*.tar.zst "$out"/
  cp ${installer}/bin/eon-install "$out"/eon-install-x86_64-linux
  cd "$out"
  sha256sum -- * > SHA256SUMS
''
