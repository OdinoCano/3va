{ lib, stdenv, fetchurl, autoPatchelfHook }:

let
  version = "2.6.0";
  pname   = "three-va";

  assets = {
    "x86_64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-unknown-linux-gnu.tar.gz";
      sha256 = "c32f807beaa0b7a51cd5f3e0778db88068390f8c301530ac8f8aa8751eb2c669";
    };
    "aarch64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-unknown-linux-gnu.tar.gz";
      sha256 = "777fa59930d3ce503afa7122119571fa8d8d24f4c6b4a28a1fd0c6bcfa6a8a47";
    };
    "x86_64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-apple-darwin.tar.gz";
      sha256 = "1343a06bd0dde5de19014dabe8573202b74ef588baab62767c0c25c990ea4e8c";
    };
    "aarch64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-apple-darwin.tar.gz";
      sha256 = "cd38872eeadd1d3a5237f1640fbbc1df9d293de4f22572e12e417651d14a115f";
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
