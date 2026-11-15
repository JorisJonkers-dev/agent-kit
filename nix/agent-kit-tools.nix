# The release's prebuilt static tools as a package (#807): the same
# agent-kit-tools-<tag>-<target>.tar.gz the bootstrap shell installs, checked
# against the release's SHA256SUMS. Nothing is built; the Linux binaries are
# statically linked (musl), so no patching is needed.
#
#   pkgs.callPackage ./agent-kit-tools.nix {
#     tag = "v1.2.3";
#     sums = builtins.readFile ./SHA256SUMS;   # the release asset of that tag
#   }
{ lib, stdenvNoCC, fetchurl, tag, sums }:

let
  targets = {
    x86_64-linux = "x86_64-unknown-linux-musl";
    aarch64-linux = "aarch64-unknown-linux-musl";
    aarch64-darwin = "aarch64-apple-darwin";
  };
  target = targets.${stdenvNoCC.hostPlatform.system}
    or (throw "agent-kit-tools: no prebuilt release for ${stdenvNoCC.hostPlatform.system}");
  file = "agent-kit-tools-${tag}-${target}.tar.gz";
  # SHA256SUMS lines are "<hex>  <file>".
  hashes = lib.listToAttrs (map
    (line:
      let parts = lib.splitString "  " line;
      in { name = lib.last parts; value = lib.head parts; })
    (lib.filter (line: line != "") (lib.splitString "\n" sums)));
in
stdenvNoCC.mkDerivation {
  pname = "agent-kit-tools";
  version = lib.removePrefix "v" tag;

  src = fetchurl {
    url = "https://github.com/JorisJonkers-dev/agent-kit/releases/download/${tag}/${file}";
    sha256 = hashes.${file} or (throw "agent-kit-tools: SHA256SUMS has no line for ${file}");
  };

  sourceRoot = "agent-kit-tools-${tag}-${target}";
  dontConfigure = true;
  dontBuild = true;
  dontFixup = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    install -m 0755 bin/* $out/bin/
    install -m 0644 VERSION $out/VERSION
    runHook postInstall
  '';

  meta = {
    description = "Prebuilt static agent-kit command-line tools";
    homepage = "https://github.com/JorisJonkers-dev/agent-kit";
    license = lib.licenses.unfree;
    platforms = builtins.attrNames targets;
    mainProgram = "model-gateway";
  };
}
