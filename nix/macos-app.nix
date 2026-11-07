# Nix-only app resources; Eon's install phase supplies the opaque CLI launch input.
{ pkgs, version, nushellPackage, bashPackage }:
let
  inherit (pkgs) lib;
  launcher = pkgs.rustPlatform.buildRustPackage {
    pname = "eon-macos-launcher";
    version = "0.1.0";
    src = lib.fileset.toSource {
      root = ../launcher;
      fileset = lib.fileset.unions [
        ../launcher/Cargo.toml
        ../launcher/Cargo.lock
        ../launcher/src/bin/eon-macos-launcher.rs
      ];
    };
    cargoLock.lockFile = ../launcher/Cargo.lock;
    cargoBuildFlags = [ "--bin" "eon-macos-launcher" ];
    cargoTestFlags = [ "--bin" "eon-macos-launcher" ];
    nativeCheckInputs = [ nushellPackage bashPackage ];
  };
  plist = pkgs.writeText "Eon-Info.plist" (lib.generators.toPlist { escape = true; } {
    CFBundleIdentifier = "com.yazelix.eon";
    CFBundleName = "Eon";
    CFBundleDisplayName = "Eon";
    CFBundlePackageType = "APPL";
    CFBundleExecutable = "Eon";
    CFBundleIconFile = "Eon.icns";
    CFBundleShortVersionString = version;
    CFBundleVersion = version;
    LSMinimumSystemVersion = "14.0";
    NSHighResolutionCapable = true;
  });
in
pkgs.runCommand "eon-macos-app-${version}" { } ''
  app="$out/Applications/Eon.app/Contents"
  mkdir -p "$app/MacOS" "$app/Resources"
  cp ${plist} "$app/Info.plist"
  cp ${launcher}/bin/eon-macos-launcher "$app/MacOS/Eon"
  for size in 16 32 48 128 256 512 1024; do
    ${pkgs.imagemagick}/bin/magick ${../assets/eon.png} \
      -filter Box -resize "''${size}x''${size}" "$TMPDIR/$size.png"
  done
  ${pkgs.libicns}/bin/png2icns "$app/Resources/Eon.icns" \
    "$TMPDIR/"{16,32,48,128,256,512,1024}.png
  test -s "$app/Resources/Eon.icns"
  test -x "$app/MacOS/Eon"
''
