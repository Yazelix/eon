# Reference binaries under test, outside the product closure.
{ pkgs, venusFonts, venusFontsConf }:
let
  inherit (pkgs) lib;
  fontsConf = pkgs.writeText "eon-bench-fonts.conf" ''
    <?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd">
    <fontconfig>
      ${lib.concatMapStringsSep "\n" (font: "<dir>${font}</dir>") venusFonts}
      <cachedir prefix="xdg">fontconfig</cachedir>
      <match target="pattern"><edit name="dpi" mode="assign"><double>96</double></edit></match>
    </fontconfig>
  '';
  herdr = pkgs.stdenv.mkDerivation {
    pname = "herdr-app-bench";
    version = "0.1.14";
    src = pkgs.fetchurl {
      url = "https://releases.herdr.dev/app/0.1.14/herdr-app-linux-x86_64.tar.gz";
      sha256 = "475a383f8f439e3191a86635b5c1286ce4dfd2d9fcdf1e7cbdfdd6c7176f8ff3";
    };
    core = pkgs.fetchurl {
      url = "https://releases.herdr.dev/core/0.1.19/herdr-core-linux-x86_64.tar.gz";
      sha256 = "4adcfb423d52f24c5cc4a1e2cd2fcad1f62d28bb77876ac472c29082a3eaa424";
    };
    nativeBuildInputs = [ pkgs.autoPatchelfHook pkgs.makeWrapper ];
    buildInputs = [ pkgs.libxcb pkgs.libxkbcommon pkgs.stdenv.cc.cc.lib ];
    dontBuild = true;
    installPhase = ''
      runHook preInstall
      mkdir -p "$out/bin" "$out/share/licenses/herdr-app"
      cp herdr-app "$out/bin/"
      cp LICENSE THIRD-PARTY-NOTICES "$out/share/licenses/herdr-app/"
      tar -xf "$core" -C "$TMPDIR"
      cp "$TMPDIR/herdr-core" "$out/bin/"
      ln -s herdr-core "$out/bin/hc"
      runHook postInstall
    '';
    postFixup = ''
      wrapProgram "$out/bin/herdr-app" \
        --prefix PATH : "$out/bin" \
        --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ pkgs.vulkan-loader pkgs.wayland pkgs.libGL pkgs.fontconfig ]}
    '';
    meta.platforms = [ "x86_64-linux" ];
  };
in
{
  inherit herdr;
  environment = {
    EON_BENCH_FONTCONFIG = "${fontsConf}";
    EON_BENCH_PRODUCT_FONTCONFIG = "${venusFontsConf}";
    EON_BENCH_FOOT = "${pkgs.foot}/bin/foot";
    EON_BENCH_ALACRITTY = "${pkgs.alacritty}/bin/alacritty";
    EON_BENCH_KITTY = "${pkgs.kitty}/bin/kitty";
    EON_BENCH_GHOSTTY = "${pkgs.ghostty}/bin/ghostty";
    EON_BENCH_WEZTERM = "${pkgs.wezterm}/bin/wezterm";
    EON_BENCH_HERDR_APP = "${herdr}/bin/herdr-app";
    EON_BENCH_FONT = "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSansMono.ttf";
    EON_BENCH_FC_MATCH = "${pkgs.fontconfig}/bin/fc-match";
  };
}
