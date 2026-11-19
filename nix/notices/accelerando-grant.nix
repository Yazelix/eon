# The Accelerando grant record, read once for every place that ships or checks
# it: the archive notices, the Nix package's licenses directory and the
# presence check in nix/accelerando-check.nix. The record must cover exactly
# the Anima files Eon ships, so a record edit that drops or adds one fails
# evaluation instead of passing with a narrower grant.
{ lib }:
let
  path = ./accelerando-grant.json;
  record = builtins.fromJSON (builtins.readFile path);
  # Sorted, as builtins.attrNames returns them.
  shipped = [
    "fonts/accelerando-terminal-bold.ttf"
    "fonts/accelerando-terminal.ttf"
    "src/accelerando-glyphs.bin"
  ];
in
assert lib.assertMsg (builtins.attrNames record.assets == shipped)
  "nix/notices/accelerando-grant.json must cover exactly ${lib.concatStringsSep ", " shipped}";
{
  inherit path;
  inherit (record) license;
  # Anima-relative path to the sha256 of the bytes the grant covers.
  inherit (record) assets;
}
