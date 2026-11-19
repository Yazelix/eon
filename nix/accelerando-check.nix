# Fails unless a shipped Eon tree carries Anima's Accelerando program and the
# Accelerando Terminal font, with the exact bytes whose sha256 the grant record
# (nix/notices/accelerando-grant.nix) lists. `root` is the tree's prefix: empty
# for a Nix package, the relinked tree for the Nix-free archive. `wiring` proves
# the checked program and fonts are the ones the product launches. Overridable,
# so a probe can reuse the exact check with one argument changed.
{ pkgs, anima, noticeTool, grant }:
let
  inherit (grant) assets;
  glyphs = "src/accelerando-glyphs.bin";
in
pkgs.lib.makeOverridable ({ name, root ? "", program, fontsConf, wiring ? "" }:
  pkgs.runCommand "eon-accelerando-${name}-check" {
    nativeBuildInputs = [ pkgs.fontconfig ];
    passthru = { inherit root program fontsConf; };
  } ''
    fail() {
      echo "${name}: $*" >&2
      exit 1
    }
    digest() {
      sha256sum < "$1" | cut -d ' ' -f 1
    }
    # Anima embeds these bitmaps for its Accelerando program only.
    test "$(digest ${anima}/${glyphs})" = ${assets.${glyphs}} \
      || fail "Anima's ${glyphs} is not the bitmap set the Accelerando grant covers"
    ${noticeTool}/bin/archive-notices require-embedded \
      ${root}${program} ${anima}/${glyphs} \
      || fail "${program} lacks the Anima Accelerando program"
    export HOME="$TMPDIR" XDG_CACHE_HOME="$TMPDIR/cache"
    # Venus's fonts.conf names its font directories relative to itself, so the
    # archive's copy resolves inside the archive tree.
    FONTCONFIG_FILE=${root}${fontsConf} fc-list --format '%{family[0]}|%{style[0]}|%{file}\n' > fonts
    for face in Regular:${assets."fonts/accelerando-terminal.ttf"} \
      Bold:${assets."fonts/accelerando-terminal-bold.ttf"}; do
      style=''${face%%:*}
      # Only the checked tree's own fonts count, never a host font directory.
      files=$(awk -F '|' -v style="$style" -v prefix="${root}${builtins.storeDir}/" \
        '$1 == "Accelerando Terminal" && $2 == style && index($3, prefix) == 1 { print $3 }' fonts)
      test -n "$files" || fail "lacks the Accelerando Terminal $style font"
      for file in $files; do
        test "$(digest "$file")" = "''${face#*:}" \
          || fail "$file is not the font the Accelerando grant covers"
      done
    done
    ${wiring}
    touch "$out"
  '')
