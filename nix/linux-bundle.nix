{ pkgs, eonPackage, manifestFile, lockFile }:
let
  helper = pkgs.pkgsStatic.bubblewrap;
  launcher = pkgs.pkgsStatic.rustPlatform.buildRustPackage {
    pname = "eon-bundle";
    version = "0.1.0";
    src = ../bundle;
    cargoLock.lockFile = ../bundle/Cargo.lock;
    EON_BUNDLE_PROGRAM = "${eonPackage}/bin/eon";
  };
  closure = pkgs.closureInfo { rootPaths = [ eonPackage ]; };
  name = "eon-${eonPackage.version}-x86_64-linux";
  provenance = pkgs.writeText "eon-bundle-provenance.json" (builtins.toJSON {
    schema = 1;
    target = "x86_64-linux";
    package = toString eonPackage;
    manifest_sha256 = builtins.hashFile "sha256" manifestFile;
    lock_sha256 = builtins.hashFile "sha256" lockFile;
    bubblewrap = {
      inherit (helper) version;
      source = helper.src.rev;
      nar_hash = helper.src.outputHash;
    };
  });
in
pkgs.runCommand "${name}-bundle" {
  nativeBuildInputs = [ pkgs.gnutar pkgs.zstd pkgs.binutils ];
} ''
  mkdir -p "$out" ${name}/libexec ${name}/nix/store ${name}/licenses
  cp ${launcher}/bin/eon-bundle ${name}/eon
  cp ${helper}/bin/bwrap ${name}/libexec/bwrap
  for binary in ${name}/eon ${name}/libexec/bwrap; do
    headers=$(readelf -l "$binary")
    if [[ "$headers" == *INTERP* ]]; then
      echo "bundle bootstrap must be static: $binary" >&2
      exit 1
    fi
  done
  while IFS= read -r path; do
    cp -a --reflink=auto --parents "$path" ${name}
  done < ${closure}/store-paths
  cp ${closure}/store-paths ${name}/closure-paths.txt
  cp ${manifestFile} ${name}/component-manifest.json
  cp ${lockFile} ${name}/flake.lock
  cp ${provenance} ${name}/provenance.json
  cp ${../LICENSE} ${name}/licenses/eon
  cp -R ${helper.src} ${name}/licenses/bubblewrap-source
  chmod -R u+w ${name}/licenses/bubblewrap-source
  tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner \
    -cf - ${name} | zstd -T2 -10 -o "$out/${name}.tar.zst"
  cd "$out"
  sha256sum ${name}.tar.zst > SHA256SUMS
''
