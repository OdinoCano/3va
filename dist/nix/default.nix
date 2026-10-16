{ lib, stdenv, fetchurl, autoPatchelfHook }:

let
  version = "2.13.0";
  pname   = "three-va";

  assets = {
    "x86_64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-unknown-linux-gnu.tar.gz";
      sha256 = "57a67b848393eda94bc36f223332caf780ea213d11f8e54addac7393e22392e8";
    };
    "aarch64-linux" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-unknown-linux-gnu.tar.gz";
      sha256 = "17d2960b0d492b5ae35830537de941ffc82dbaf9f04293a05aa3d9ad7dd91167";
    };
    "x86_64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-x86_64-apple-darwin.tar.gz";
      sha256 = "b253c91759e6f8a054940e281edc6ee7e8ef542c16d5dbd0f6f19437fbfa6449";
    };
    "aarch64-darwin" = {
      url    = "https://github.com/OdinoCano/3va/releases/download/v${version}/3va-v${version}-aarch64-apple-darwin.tar.gz";
      sha256 = "1dfffd4e9ea95eec37ef7013b8cfe7c54386a52b48bf4f59677ac393c2c2b212";
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
