{ pkgs, yaziPackage }:
let
  inherit (pkgs) lib;
  revision = "2d52c0b8b399aa476bfeff36a6256cffe76eb2a5";
  vendor = "${yaziPackage.cargoDeps}/source-registry-0/yazi-prebuilt-0.1.0";
  batUrl = "https://raw.githubusercontent.com/sharkdp/bat/v0.25.0";
  definitions = builtins.fromJSON (builtins.readFile ./yazi-syntaxes.json);
  unverified = "redistribution terms not identified; distributed unmodified as part of Yazi's upstream syntax pack (yazi-prebuilt ${revision})";
  acknowledgements = pkgs.fetchurl {
    url = "${batUrl}/assets/acknowledgements.bin";
    hash = "sha256-K+af/SBiFoRTkDaA+AFhnAlRIuJmq9WIL9BZAoDtg6I=";
  };
  batNotice = pkgs.runCommand "bat-0.25.0-syntax-acknowledgements.txt" {
    nativeBuildInputs = [ pkgs.pigz ];
  } ''
    pigz -zdc ${acknowledgements} | tail -c +9 > acknowledgements
    printf '%s\n' 'bc2976c316f02ee18a9f4137c8643b259d8ddd537e20b9b9f758d1fa25513338  acknowledgements' | sha256sum -c -
    sed '/^## themes\//,$d' acknowledgements > "$out"
  '';
  sublimeNotice = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/sublimehq/Packages/759d6eed9b4beed87e602a23303a121c3a6c2fb3/LICENSE";
    hash = "sha256-aqOQFBaQXW7Q/GsPzKdeZkn0Bm2z9hDpZT0OAEnhOec=";
  };
  glslNotice = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/euler0/sublime-glsl/59a5f8a3120358657cefdc4b830b4a883ebfbf77/LICENSE";
    hash = "sha256-fhLl30uuEsshWBuhV87SDhmGoFCN0Q0Oikq5pM+U6Fw=";
  };
  definitionNotice = pkgs.writeText "yazi-syntax-definition-notices.txt" (''
    Yazi syntax definitions and ANSI theme

    yazi-prebuilt 0.1.0 at ${revision} embeds Bat 0.25.0's syntax set.
    The entries below group the file-backed definitions by source. The plain
    text definition is built in to Syntect. Bat's syntax acknowledgements
    and the other identified license texts accompany this list.

    ANSI theme (${batUrl}/assets/themes/ansi.tmTheme)
    Template by Chris Kempson. Scheme by Mitchell Kember.
    ${unverified}.
  '' + lib.concatMapStrings (entry: ''

    ${entry.group}
    Source ${entry.source}
    ${if entry.licenses == [ ] then unverified else lib.concatStringsSep ", " entry.licenses}
    ${lib.concatMapStringsSep "\n" (name: "  ${name}") entry.definitions}
  '') definitions);
in
{
  id = "yazi-upstream-syntax-pack";
  version = "yazi-prebuilt 0.1.0 @ ${revision}, Bat 0.25.0";
  source = "https://github.com/yazi-rs/prebuilt/tree/${revision} (${vendor}/built)";
  licenses = lib.unique (lib.concatMap (entry: entry.licenses) definitions);
  notice_files = map toString [ definitionNotice batNotice sublimeNotice glslNotice ];
  notice_hashes = {
    ${builtins.unsafeDiscardStringContext "${vendor}/built/syntaxes"} =
      "50384288b5493bc2ec14347b866a484feb027e5236f4687b6253d0ddc6de20fe";
    ${builtins.unsafeDiscardStringContext "${vendor}/built/ansi.tmTheme"} =
      "3551ceb88a95f19ccad074e9a160467f6ffa8491a9e39f81ae42917989711f27";
  };
}
