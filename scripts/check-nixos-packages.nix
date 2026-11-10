# Check that every nixpkgs attribute the registry's NixOS package list names
# exists in a given nixpkgs and evaluates.
#
# registry/generated/nixos/packages.nix is generated. Each entry names an
# attribute path (`attr`, possibly dotted) and where it comes from (`source`).
# Only `source = "nixpkgs"` entries are checked: `nixos-modules` entries are
# packaged by JorisJonkers-dev/nixos-modules and are not in nixpkgs.
#
# Usage, from the repository root, against the nixpkgs revision that
# nixos-modules pins (see the `nixos-packages` job in .github/workflows/ci.yml):
#
#   nix --extra-experimental-features 'nix-command flakes' eval --json --impure \
#     --expr 'import ./scripts/check-nixos-packages.nix {
#       nixpkgs = builtins.fetchTarball "https://github.com/NixOS/nixpkgs/archive/<rev>.tar.gz";
#     }'
#
# It returns { ok, failures, checked }: `ok` is false when any entry is
# missing or does not evaluate, `failures` names each one as
# "<group>/<attr>: missing", "<group>/<attr>: not a package" or "<group>/<attr>: does not evaluate", and
# `checked` counts the nixpkgs entries that were looked at. Pass `packagesFile`
# to check a list other than the generated one.
{ nixpkgs, packagesFile ? ../registry/generated/nixos/packages.nix }:

let
  lib = pkgs.lib;

  pkgs = import nixpkgs {
    system = "x86_64-linux";
    config.allowUnfree = true;
  };

  packages = import packagesFile;

  # Every entry of every group, tagged with the group it came from.
  entries = lib.concatMap
    (group: map (entry: entry // { inherit group; }) packages.${group})
    (builtins.attrNames packages);

  checkedEntries = lib.filter (entry: (entry.source or "nixpkgs") == "nixpkgs") entries;

  # A path that resolves to nothing is reported as missing. One that resolves
  # but whose derivation cannot be instantiated (broken, unsupported, throwing)
  # is reported as not evaluating. tryEval keeps one bad entry from aborting
  # the run, so every failure is reported in a single pass.
  check = entry:
    let
      label = "${entry.group}/${entry.attr}";
      value = lib.attrByPath (lib.splitString "." entry.attr) null pkgs;
      evaluated = builtins.tryEval (builtins.seq value.drvPath true);
    in
    if value == null then [ "${label}: missing" ]
    else if !(lib.isDerivation value) then [ "${label}: not a package" ]
    else if evaluated.success then [ ]
    else [ "${label}: does not evaluate" ];

  failures = lib.concatMap check checkedEntries;
in
{
  ok = failures == [ ];
  inherit failures;
  checked = lib.length checkedEntries;
}
