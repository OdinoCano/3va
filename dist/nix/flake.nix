{
  description = "3va — secure-by-default JavaScript and TypeScript runtime";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in {
      packages = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.callPackage ./default.nix {};
          "3va"   = pkgs.callPackage ./default.nix {};
        }
      );

      apps = forAllSystems (system: {
        default = {
          type    = "app";
          program = "${self.packages.${system}.default}/bin/3va";
        };
      });
    };
}
