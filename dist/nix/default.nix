{ lib, stdenv, fetchurl, autoPatchelfHook }:

let
  version = "1.0.0";
  pname   = "three-va";

  assets = {
    "x86_64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-unknown-linux-gnu.tar.gz";
      sha256 = "SHA256_LINUX_X64";
    };
    "aarch64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-unknown-linux-gnu.tar.gz";
      sha256 = "SHA256_LINUX_ARM64";
    };
    "x86_64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-apple-darwin.tar.gz";
      sha256 = "SHA256_DARWIN_X64";
    };
    "aarch64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-apple-darwin.tar.gz";
      sha256 = "SHA256_DARWIN_ARM64";
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
