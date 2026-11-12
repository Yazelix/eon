# Building and replacing the covered components

This companion belongs to the binary archive identified by `inventory_sha256` in
`source-index.json`. Compare that hash with the binary's
`THIRD-PARTY-NOTICES/inventory.json`. The index maps each component and selected
license to its source identity and `producer` (sources, ordered patches, and build
configuration). Each original path maps to a local file under `materials` with
its SHA-256 and format. Those original store paths are provenance, not locations
you must have on your machine. `build_support` identifies the `nixpkgs`, `eon`
and `helix` source trees by role. Each value is an original path whose entry in
`materials` gives the local tar filename; use these roles when selecting the trees
named below. The companion is ordinary tar/zstd data.

Directory sources are nested deterministic tar files. Extract each into its own
directory with `tar -xf MATERIAL -C DIRECTORY`; original upstream archives retain
their original compression. Keep original copyright, license and exception files.
The full pinned Nixpkgs source tree is build support: it includes package recipes,
local patches and stdenv hooks. It is not a prebuilt dependency closure. Building
can fetch the recipe's normal compiler/tool dependencies and requires Nix; reading,
modifying and extracting the supplied source does not.

## Rebuild from the original recipes

Extract the Nixpkgs build-support material as `nixpkgs/`. The selected native
producers are `bash`, `bashInteractive`, `readline`, `bc`, `keyutils`,
`stdenv.cc.cc` (GCC runtimes), `glibc` (startup source), and `cacert` in that tree.
For example, to build Readline with your modified source, preserving the pinned
recipe's patches and settings:

```sh
nix build --impure --expr '(import ./nixpkgs { system = "x86_64-linux"; }).readline.overrideAttrs (_: { src = ./modified-readline; })'
```

The index's `producer.build` JSON records evaluated Eon-selected flags, hooks and
dependencies where a package overrides its upstream recipe. Producer output
references use `@output:NAME@`, with declared names in `output_names`. Substitute
the producer's selected output directory for each such token when using these
settings; it never names the metadata file's output. Shell `$out` and other output
variables in recorded script bodies retain their normal build-script meaning.
Its `producer.patches`
lists the exact additional fetched and local patches in application order. For
Bash and Readline this includes the upstream patchlevels, not just the base
tarball. Use unpatched source with the recipe's patch phase; do not apply patches
twice. The included Eon `flake.nix` records composition overrides and
`nix/bundle-relink.nix` records the actual archive transformation. It is supplied
with the existing standalone `relink/` source, Cargo manifest/lock and launcher
source. These are the scripts actually used, not a separate replacement installer.

The Helix source trees include workspace Cargo manifests/locks, runtime data,
`languages.toml` and `grammars.nix`. Eon's selected Helix build disables default
features and enables `git`, without Steel. Each covered registry crate is retained
from the pinned Cargo vendor root, including its checksum metadata. Its original
registry checksum and exact version are in the index. Atuin's local covered crates
retain the parent workspace source. The libgit2 source is the complete pinned
`libgit2-sys` crate with native source and build scripts, not just its notice files.

Build the Vala grammar using the supplied `grammars.nix` build phase and its exact
source: compile `src/parser.c` and any `src/scanner.c`/`.cc` with `-Isrc -O3 -g
-fPIC -fno-exceptions -Wl,-z,relro,-z,now`, then link their objects into `vala.so`
with the C++ compiler's `-shared` flag. Preserve the target ABI and parser entry
point `tree_sitter_vala`. Grammar source is already generated; no replacement
parser generator or moving grammar checkout is required.

## Replace a shared library in your own bundle copy

Work on a separate extracted copy, not a running installation. Files may initially
be read-only; as their owner, make the copy writable. Use the binary inventory's
`binaries` and component `outputs` to identify the library and every consumer.
Replace the actual file reached by its existing SONAME symlink, preserving that
name and symlink arrangement. An interface-compatible modified Readline,
keyutils, libquadmath or Vala grammar can be substituted this way. Retain executable
and read permissions.

Inspect the consumers and replacement with `readelf -d FILE`: retain the required
SONAME, NEEDED entries and ABI. The bundled loader paths use `$ORIGIN`-relative
RUNPATH entries; they are not a signature or hash restriction. A replacement that
introduces extra dependencies needs those dependencies and appropriate relative
RUNPATHs too. The supplied `bundle-relink.nix` and `relink/` describe the original
process. `patchelf --set-rpath '$ORIGIN:RELATIVE_DEPENDENCY_DIRECTORY' LIBRARY`
can adjust a replacement's paths to its actual layout; do not copy a Nix-store
absolute RUNPATH into a Nix-free tree. ELF interpreters of executable replacements
use the host `/lib64/ld-linux-x86-64.so.2` and the archive's glibc 2.39 floor.

Start a fresh consumer against your copy to load the modified library. Do not
restart existing Eon supervisors or overwrite a library in use. The LGPL permits
modification and reverse engineering for debugging those modifications; Eon adds
no restriction on these rights. Library licenses and their original notices apply
to source changes and redistribution.

## Startup code and changes

`runtime_exception_basis` in the index records the glibc startup linking exception
and GCC Runtime Library Exception eligibility for this composition. Their exact
source and original exception texts ship here. This does not turn ordinary LGPL
libraries such as libquadmath into excepted code. The host supplies the dynamic
glibc; glibc startup objects can still be embedded in dynamically linked binaries.
The static installer and launcher use musl.

Eon changes, 2026-11-11: the archive changes ELF interpreters and library search
paths and makes absolute store symlinks relative. Native producer patches and
build settings are separately enumerated in the index and included recipes.
These transformations preserve the upstream licenses; source is provided under
its original terms. For MPL-covered code, the companion supplies the covered
Source Code Form and its license, and the index retains the pinned upstream
identities. Runtime exceptions do not relicense the combined application.
