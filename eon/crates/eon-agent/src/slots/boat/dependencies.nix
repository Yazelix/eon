{ doc }:
assert doc.version == 4;
let
  drvs = doc.derivations;
  inputs = builtins.concatMap (
    drv:
    builtins.concatMap (
      path:
      if builtins.hasAttr path drvs then [ ] else
      let input = drv.inputs.drvs.${path}; outputs = input.outputs; in
      assert input.dynamicOutputs == { };
      assert builtins.isList outputs;
      map (output: "${builtins.storeDir}/${path}^${output}") outputs
    ) (builtins.attrNames drv.inputs.drvs)
  ) (builtins.attrValues drvs);
in
builtins.concatStringsSep "\n" inputs
