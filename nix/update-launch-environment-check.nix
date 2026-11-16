{ pkgs, rustToolchain, eonEnvironment, eonComponents, eonHelix, eonNoHelix, eonLaunchWrapper, wrapperArgs }:
let
  inherit (pkgs) lib;
  source = pkgs.writeText "eon-update-launch-probe.rs" (
    lib.replaceStrings [ "@launchModule@" "@nativeModule@" ]
      [ "${../runtime/crates/eon-runtime/src/launch_environment.rs}" "${../venus/src/native_command.rs}" ]
      (builtins.readFile ./update-launch-environment-check.rs)
  );
  probe = pkgs.runCommand "eon-update-launch-probe" { nativeBuildInputs = [ rustToolchain ]; } ''
    rustc --edition=2024 ${source} -o "$out"
  '';
  fixture = name: full: legacy:
    let
      components = {
        environment = lib.mapAttrs (key: _: "/fixture/${name}/${key}") eonComponents.environment;
        terminfo = "/fixture/${name}/terminfo";
      };
      tools = if full then {
        environment.set = lib.mapAttrs (key: _: "/fixture/${name}/${key}") eonHelix.environment.set;
      } else eonNoHelix;
      environment = eonEnvironment components tools (placeholder "out");
      changed = environment // {
        set = environment.set // lib.optionalAttrs (lib.hasPrefix "A-" name) { EON_REMOVED = "/fixture/${name}/removed"; };
      };
      rendered = eonLaunchWrapper { program = "${placeholder "out"}/bin/eon"; environment = changed; };
    in
    # A doubled dollar here captures Bash's PID instead of the user's value.
    assert lib.hasInfix "export EON_LAUNCH_SAVED_EON_ORBIT=\"+\${EON_ORBIT}\"" rendered;
    assert lib.hasInfix "export EON_LAUNCH_SAVED_EON_HX=\"+\${EON_HX}\"" rendered;
    assert !(lib.hasInfix "$$" rendered);
    assert lib.hasInfix "if [[ \${EON_ORBIT+x} ]]" rendered;
    assert lib.hasInfix "case \":\${TERMINFO_DIRS-}:\" in" rendered;
    assert lib.hasInfix "\"\${TERMINFO_DIRS+:}\${TERMINFO_DIRS-}\"" rendered;
    pkgs.runCommand "eon-launch-${name}${lib.optionalString legacy "-legacy"}" { nativeBuildInputs = [ pkgs.makeWrapper ]; } (''
      install -Dm755 ${probe} "$out/bin/eon"
      ln -s eon "$out/bin/eon-bash"
      ln -s eon "$out/bin/hx"
    '' + rendered + lib.optionalString legacy ''
      # Keep identical facts, but replace only the producer with the former renderer.
      rm "$out/bin/eon"
      makeWrapper "$out/bin/.eon-wrapped" "$out/bin/eon" --inherit-argv0 ${wrapperArgs changed}
    '');
  fixtures = [
    (fixture "A-full" true false) (fixture "B-none" false false) (fixture "C-full" true false)
    (fixture "A-none" false false) (fixture "B-full" true false) (fixture "C-none" false false)
    (fixture "A-old" true true)
  ];
  native = pkgs.runCommand "eon-native-launch-fixture" { nativeBuildInputs = [ pkgs.makeWrapper ]; } ''
    makeWrapper ${probe} "$out/bin/venus" --set EON_VENUS_FONT_DIRS /fixture/native/fonts
  '';
in
assert lib.all (name: !(builtins.tryEval (eonLaunchWrapper {
  program = "/fixture/bin/eon";
  environment.set = { ${name} = "invalid"; };
})).success) [ "_" "PWD" "SHLVL" ];
pkgs.runCommand "eon-update-launch-environment" { nativeBuildInputs = [ rustToolchain ]; } ''
  rustc --edition=2024 --test ${../runtime/crates/eon-runtime/src/launch_environment.rs} -o unit-check
  ./unit-check
  ${probe} driver ${pkgs.bash}/bin/bash ${lib.escapeShellArgs (map (root: "${root}/bin/eon") fixtures)} ${native}/bin/venus
  touch "$out"
''
