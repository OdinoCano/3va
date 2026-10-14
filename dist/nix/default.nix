{ lib, stdenv, fetchurl, autoPatchelfHook }:

let
  version = "2.12.3";
  pname   = "three-va";

  assets = {
    "x86_64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-unknown-linux-gnu.tar.gz";
      sha256 = "a805d04746f44ba926dd8d0b308102586eafa6fe53c211c4ed1b9f19656b28f0";
    };
    "aarch64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-unknown-linux-gnu.tar.gz";
      sha256 = "aeddcb4c988335156e37e4e76c12716e79bd1e2dc8dee56014e0f15adf27d9ea";
    };
    "x86_64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-apple-darwin.tar.gz";
      sha256 = "51ec26d53425a56259afd3f7227afbd888d5db41fa617783403bd7b7e92cde96";
    };
    "aarch64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-apple-darwin.tar.gz";
      sha256 = "f9679697ac3efd722e2f8f6fb84ffe86cb83f4de47e4c9d0e3529529e5dd5a2b";
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
