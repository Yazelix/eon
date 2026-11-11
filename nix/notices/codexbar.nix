{ pkgs, package, identity, source, muslId }:
let
  inherit (pkgs) lib;
  sdk = pkgs.fetchurl {
    url = "https://download.swift.org/swift-6.3.3-release/static-sdk/swift-6.3.3-RELEASE/swift-6.3.3-RELEASE_static-linux-0.1.0.artifactbundle.tar.gz";
    hash = "sha256-h8Pq+QjmfA4TqENnEZ4SJzzsHSzT2B99dLs2ci1rYHs=";
  };
  sdkBom = pkgs.runCommand "swift-6.3.3-static-sdk-sbom.json" {
    nativeBuildInputs = [ pkgs.gnutar pkgs.gzip ];
  } ''
    tar -xOf ${sdk} swift-6.3.3-RELEASE_static-linux-0.1.0.artifactbundle/sbom.spdx.json > "$out"
  '';
  recipe = pkgs.fetchurl {
    url = "https://raw.githubusercontent.com/swiftlang/swift-docker/cdfdf30bef6f1529ad34662274db00781d87ab61/swift-ci/sdks/static-linux/scripts/fetch-source.sh";
    hash = "sha256-PRcc1wOge40GtxFQfYb5wAIMBZ72MSDNVL7L4bHEi50=";
  };
  sqliteSource = pkgs.fetchurl {
    url = "https://www.sqlite.org/2026/sqlite-amalgamation-3530400.zip";
    hash = "sha256-HnHd+ThJxqbs9YuCfAaSBz0t1+5AGWFYBo97KfQi6H0=";
  };
  sqliteNotice = pkgs.runCommand "sqlite-3.53.4-public-domain" {
    nativeBuildInputs = [ pkgs.unzip ];
  } ''
    unzip -p ${sqliteSource} sqlite-amalgamation-3530400/sqlite3.h | sed -n '1,10p' > "$out"
    grep -q 'disclaims copyright' "$out"
  '';
  pinnedNotice = entry: {
    inherit (entry) id version licenses;
    source = lib.concatMapStringsSep " " (notice: notice.url) entry.notices;
    notice_files = map (notice: "${pkgs.fetchurl notice}") entry.notices;
  };
  swiftPackage = owner: repo: revision: version: license: hash:
    let
      upstream = pkgs.fetchFromGitHub { inherit owner repo hash; rev = revision; };
    in {
      id = "codexbar-${lib.toLower repo}";
      inherit version;
      source = "https://github.com/${owner}/${repo}@${revision}";
      licenses = [ license ];
      source_roots = [ "${upstream}" ];
    };
  dependencies = map pinnedNotice (builtins.fromJSON (builtins.readFile ./codexbar-licenses.json)) ++ [
    (swiftPackage "steipete" "Commander" "bd219c4ee9032fee3e009856f81fcc6ec09a85f4" "0.2.4" "MIT"
      "sha256-ifvtIi8dazkjgNL2oP78ULNaqHYZLMxv4RGfNtMOS68=")
    (swiftPackage "steipete" "SweetCookieKit" "7c1dbb68c1d989cdcecfe43dd0b09ffc95255ea6" "0.5.5" "MIT"
      "sha256-PP+htW4n2A3X7DWQUzip+O2Lf54FHYCbZxn2ZU4r0Lw=")
    (swiftPackage "apple" "swift-crypto" "da9d28d69ebe3894b18376c8f2395c2f37b8448f" "4.5.2" "Apache-2.0"
      "sha256-el/xcTRZhTtyoTC8njY0ygsvRvQYeiRVFGDIntcP/IU=")
    (swiftPackage "apple" "swift-log" "9c6fb14227f55d8f711ce3847dc2f419fb0ecacb" "1.15.1" "Apache-2.0"
      "sha256-r8qF7bM+Fwbc/rdtfqUdspztztcXL8xPftn9gs43qo0=")
    (swiftPackage "apple" "swift-asn1" "d9a5b37470adc940d22c3bcd5ca6953a516b727f" "1.7.2" "Apache-2.0"
      "sha256-p38YLHFz0CTctI7Xi3Apprg7sMVGknp8wlZo5Lb0xL4=")
    {
      id = "codexbar-sqlite";
      version = "3.53.4";
      source = sqliteSource.url;
      licenses = [ "blessing" ];
      notice_files = [ "${sqliteNotice}" ];
    }
    {
      id = "codexbar-quickjs";
      version = identity.revision;
      source = "${identity.source.url}@${identity.revision}:Sources/CQuickJS";
      licenses = [ "MIT" ];
      notice_files = [ "${source}/Sources/CQuickJS/LICENSE" ];
    }
    {
      id = "codexbar-sucrase";
      version = "3.35.1";
      source = "${identity.source.url}@${identity.revision}:docs/THIRD_PARTY_LICENSES.md";
      licenses = [ "MIT" ];
      notice_files = [ "${source}/docs/THIRD_PARTY_LICENSES.md" ];
    }
  ];
in dependencies ++ [
  {
    id = "codexbar-static-dependencies";
    version = identity.version;
    source = "${identity.source.url}@${identity.revision}, Swift SDK 6.3.3, prebuilt CLI in ${package}";
    licenses = [ "MIT" ];
    notice_files = [
      "${source}/LICENSE" "${source}/Package.resolved" "${source}/Package.swift"
      "${source}/.github/workflows/release-cli.yml" "${sdkBom}" "${recipe}"
    ];
    # The build recipe and lockfile are evidence for this exact prebuilt Linux CLI.
    notice_hashes = {
      ${builtins.unsafeDiscardStringContext "${package}/bin/.CodexBarCLI-wrapped"} =
        "4704ad3cf5bf2363643833c8631e10c59cb604f6ba9a02c4a5ea62f194c568d3";
    };
    embedded = [ muslId ] ++ map (entry: entry.id) dependencies;
  }
]
