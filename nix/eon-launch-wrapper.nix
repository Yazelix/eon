# Capture only this composition's mutations; values are never evaluated as shell text.
{ lib, wrapperArgs }:
{ program, environment }:
let
  set = environment.set or { };
  unset = environment.unset or [ ];
  prefix = environment.prefix or { };
  names = builtins.attrNames set ++ unset ++ builtins.attrNames prefix;
  root = dirOf (dirOf program);
  facts = "${root}/share/eon/launch-environment";
  operations = lib.concatLists (
    lib.mapAttrsToList (name: value: [ "set" name value ]) set
    ++ map (name: [ "unset" name "" ]) unset
    ++ lib.mapAttrsToList (name: values: [ "prefix" name (lib.concatStringsSep ":" values) ]) prefix
  );
  capture = ''
    for EON_LAUNCH_NAME in ''${!EON_LAUNCH_@}; do unset "$EON_LAUNCH_NAME"; done
    unset EON_LAUNCH_NAME
    ${lib.concatMapStringsSep "\n" (name: ''
      if [[ ''${${name}+x} ]]; then
        export EON_LAUNCH_SAVED_${name}="+''${${name}}"
      else
        export EON_LAUNCH_SAVED_${name}=-
      fi
    '') names}
    export EON_LAUNCH_FACTS=${lib.escapeShellArg facts}
  '';
  prefixes = lib.mapAttrsToList (name: values: [
    "--run"
    ''case ":''${${name}-}:" in
      *${lib.escapeShellArg ":${lib.concatStringsSep ":" values}:"}*) ;;
      *) export ${name}=${lib.escapeShellArg (lib.concatStringsSep ":" values)}"''${${name}+:}''${${name}-}" ;;
    esac''
  ]) prefix;
in
assert !(environment ? setDefault);
assert builtins.length names == builtins.length (lib.unique names);
assert lib.all (name: builtins.match "[A-Za-z_][A-Za-z0-9_]*" name != null && !(lib.hasPrefix "EON_LAUNCH_" name) && !(builtins.elem name [ "_" "PWD" "SHLVL" ])) names;
''
  mkdir -p ${lib.escapeShellArg "${root}/share/eon"}
  printf '%s\0' ${lib.escapeShellArgs ([ "EON-LAUNCH-1" program "${dirOf program}/.${baseNameOf program}-wrapped" ] ++ operations)} > ${lib.escapeShellArg facts}
  wrapProgram ${lib.escapeShellArg program} --run ${lib.escapeShellArg capture} \
    ${wrapperArgs { inherit set unset; }} ${lib.escapeShellArgs (lib.concatLists prefixes)}
''
