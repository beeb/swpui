{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
    }:
    let
      forAllSystems = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ fenix.overlays.default ];
          };
          toolchain = fenix.packages.${system}.fromToolchainFile { dir = ./.; };
        in
        {
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              cargo-deny
              cargo-dist
              toolchain
            ];

            RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          };
        }
      );
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "swpui";
            inherit ((lib.importTOML ./Cargo.toml).package) version;

            src = lib.cleanSource ./.;

            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };

            doCheck = false;
            meta.mainProgram = "swp";
          };
        }
      );
    };
}
