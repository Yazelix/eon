{ pkgs, eonPackage, relink, manifestFile, lockFile }:
let
  name = "eon-${eonPackage.version}-x86_64-linux";
  provenance = pkgs.writeText "eon-bundle-provenance.json" (builtins.toJSON {
    schema = 2;
    target = "x86_64-linux";
    package = toString eonPackage;
    manifest_sha256 = builtins.hashFile "sha256" manifestFile;
    lock_sha256 = builtins.hashFile "sha256" lockFile;
    # The payload is the package's closure relinked for the host glibc, so its store paths
    # name their origins, not their bytes. The excluded paths and what only they reach
    # stay out of it, and each substituted path ships its replacement's files.
    relink = {
      glibc_floor = relink.glibcFloor;
      inherit (relink) excluded substituted;
    };
  });
in
pkgs.runCommand "${name}-bundle" {
  nativeBuildInputs = [ pkgs.gnutar pkgs.zstd ];
} ''
  mkdir -p "$out" ${name}/licenses
  cp -a --reflink=auto ${relink.tree}/nix ${name}/nix
  # The launcher finds the tree from its own path, so the entrypoint links to it.
  ln -s ${pkgs.lib.removePrefix "/" "${eonPackage}/bin/eon"} ${name}/eon
  for path in ${relink.tree}${builtins.storeDir}/*; do
    echo "${builtins.storeDir}/''${path##*/}"
  done > ${name}/closure-paths.txt
  cp ${manifestFile} ${name}/component-manifest.json
  cp ${lockFile} ${name}/flake.lock
  cp ${provenance} ${name}/provenance.json
  cp ${../LICENSE} ${name}/licenses/eon
  tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner \
    -cf - ${name} | zstd -T2 -10 -o "$out/${name}.tar.zst"
  cd "$out"
  sha256sum ${name}.tar.zst > SHA256SUMS
''
