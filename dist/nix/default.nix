{ lib, stdenv, fetchurl, autoPatchelfHook }:

let
  version = "2.11.0";
  pname   = "three-va";

  assets = {
    "x86_64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-unknown-linux-gnu.tar.gz";
      sha256 = "a95a1bea4c2cb4b631b050c6ac963d3505bafff6039d32a2a980cde6a4f61e2e";
    };
    "aarch64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-unknown-linux-gnu.tar.gz";
      sha256 = "c4cc223c99088d7b15c9a3df9232a70ee586bb128e6f92bcd8f714ffc83bad47";
    };
    "x86_64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-apple-darwin.tar.gz";
      sha256 = "63e6d4281a6681c56b70438dd0a79f5fe4bc38c498f2eefe9933e62691172cbf";
    };
    "aarch64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-apple-darwin.tar.gz";
      sha256 = "7be8e7c156449d26957c1a144bffbfe94879b1f15b702308fefe86802282dcdd";
    };
  };

  system = stdenv.hostPlatform.system;
  asset  = assets.${system} or (throw "3va: unsupported system ${system}");

in stdenv.mkDerivation {
  inherit pname version;

  src = fetchurl {
    inherit (asset) url sha256;
  };

  nativeBuildInputs = lib.optionals stdenv.isLinux [ autoPatchelfHook ];

  # The archive contains only the bare `3va` binary.
  unpackPhase = ''
    tar xzf $src
  '';

  installPhase = ''
    install -Dm755 3va $out/bin/3va
  '';

  meta = {
    description = "Secure-by-default JavaScript and TypeScript runtime. Deny-by-default permissions, no pm2 needed, post-install scripts blocked unconditionally.";
    homepage    = "https://github.com/OdinoCano/3va";
    license     = lib.licenses.mit;
    maintainers = [];
    platforms   = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
    mainProgram = "3va";
  };
}
