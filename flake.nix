{
  description = "dept: 依存ライブラリの遅れを major/minor/patch の重み付きでスコア化する";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachSystem [ "aarch64-darwin" "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        dept = pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            description = "Score how far your dependencies lag behind their latest releases";
            mainProgram = "dept";
          };
        };
      in
      {
        packages.default = dept;
        apps.default = flake-utils.lib.mkApp { drv = dept; };
        checks.default = dept;
        devShells.default = pkgs.mkShell {
          inputsFrom = [ dept ];
          packages = with pkgs; [
            clippy
            rustfmt
            rust-analyzer
          ];
        };
        formatter = pkgs.nixfmt;
      }
    );
}
