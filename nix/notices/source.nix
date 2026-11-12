{ pkgs, tool, notices, version }:
let
  name = "eon-${version}-x86_64-linux-sources";
  tree = pkgs.runCommand "${name}-tree" {
    nativeBuildInputs = [ tool pkgs.gnutar ];
  } ''
    archive-notices sources ${notices} "$out"
  '';
  archive = pkgs.runCommand name {
    nativeBuildInputs = [ pkgs.gnutar pkgs.zstd ];
  } ''
    mkdir "$out"
    tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner \
      --transform='s,^,${name}/,' -C ${tree} -cf - . \
      | zstd -T2 -10 -o "$out/${name}.tar.zst"
    sha256sum "$out/${name}.tar.zst"
  '';
  check = pkgs.runCommand "${name}-check" {
    nativeBuildInputs = [ tool pkgs.gnutar pkgs.zstd ];
  } ''
    tar --use-compress-program=zstd -xf ${archive}/${name}.tar.zst
    archive-notices check-sources ${notices} ${name}
    touch "$out"
  '';
in { inherit tree archive check; }
