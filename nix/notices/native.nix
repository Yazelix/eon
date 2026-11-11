{ pkgs }:
let
  inherit (pkgs) lib;
  catalogue = builtins.fromJSON (builtins.readFile ./native-licenses.json);
  noticeEntry = id: spec:
    let
      inherit (spec) crate;
      source = pkgs.fetchurl {
        url = "https://static.crates.io/crates/${crate.name}/${crate.name}-${crate.version}.crate";
        sha256 = crate.checksum;
      };
      notices = pkgs.runCommand "${id}-original-notices" {
        nativeBuildInputs = [ pkgs.gnutar pkgs.gzip ];
      } (''mkdir "$out"'' + "\n" + lib.concatStringsSep "\n" (lib.imap0 (index: file: ''
        tar -xOf ${source} ${lib.escapeShellArg "${crate.name}-${crate.version}/${file}"} \
          > "$out/LICENSE-${toString index}"
      '') spec.files));
    in {
      inherit id;
      inherit (spec) version licenses;
      source = "https://crates.io/crates/${crate.name}/${crate.version} (${crate.checksum}); bundled native code";
      source_roots = [ "${notices}" ];
    };
  binding = id: spec: {
    name = spec.crate.name;
    version = spec.crate.version;
    notice = id;
  } // (spec.binding or { });
  forHost = host: lib.mapAttrsToList binding
    (lib.filterAttrs (_: spec: builtins.elem host spec.hosts) catalogue);
  exclusion = versions: reason: { inherit versions reason; };
  withoutFeatures = versions: forbidden_features: reason: { inherit versions forbidden_features reason; };
in {
  entries = lib.mapAttrsToList noticeEntry catalogue;
  bindings = lib.genAttrs [ "nushell" "atuin" "yazi" "helix" ] forHost // {
    nushell = forHost "nushell" ++ [
      { name = "libz-sys"; version = "1.1.25"; notice = pkgs.zlib.name; }
    ];
    orbit = [ { name = "libghostty-vt-sys"; version = "0.2.1"; notice = "ghostty-vt"; } ];
    installer = [ { name = "zstd-sys"; version = "2.1.1+zstd.1.5.7"; notice = "installer-zstd"; } ];
  };
  exclusions = {
    linux-raw-sys = exclusion [ "0.4.15" "0.12.1" ] "Rust Linux ABI bindings; no bundled native library.";
    dirs-sys = exclusion [ "0.4.1" "0.5.0" ] "Rust platform-directory queries; no bundled native library.";
    inotify-sys = exclusion [ "0.1.5" ] "Rust Linux syscall bindings; no bundled native library.";
    renderdoc-sys = exclusion [ "1.1.0" ] "Rust RenderDoc API declarations; no RenderDoc binary is shipped.";
    wayland-sys = exclusion [ "0.31.10" "0.31.11" ] "Rust bindings to the host Wayland library, excluded by the archive host boundary.";
    rayon-core = exclusion [ "1.13.0" ] "Pure Rust thread pool; links enforces a single Cargo runtime instance.";
    tree-sitter-language = exclusion [ "0.1.7" ] "Rust/C ABI declarations; compiled parsers and the C runtime have separate notice entries.";
    tree-sitter-bash = exclusion [ "0.25.1" ] "Crate-owned generated C parser/scanner uses the crate's MIT terms, accounted by its Cargo notice.";
    tree-sitter-fish = exclusion [ "3.6.0" ] "Crate-owned generated C parser/scanner declares MIT; the Cargo report records its stock original-notice gap.";
    signal-hook = withoutFeatures [ "0.3.18" "0.4.4" ] [ "extended-siginfo-raw" ] "The C signal-info extractor is disabled; the selected crate is Rust-only.";
    wayland-backend = withoutFeatures [ "0.3.14" "0.3.16" ] [ "log" ] "The C logging shims are disabled in the selected features.";
    av-scenechange = withoutFeatures [ "0.14.1" ] [ "asm" ] "Native assembly is disabled; cc does not produce shipped objects.";
    rav1e = withoutFeatures [ "0.8.1" ] [ "asm" ] "Native assembly is disabled; cc does not produce shipped objects.";
  };
}
