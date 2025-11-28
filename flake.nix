{
  description = "Battery monitor for Linux/NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        app = pkgs.rustPlatform.buildRustPackage {
          pname = "battery-monitor";
          version = "0.2.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.fontconfig ];
        };
      in {
        packages.default = app;
        apps.default = flake-utils.lib.mkApp { drv = app; };
        devShells.default = pkgs.mkShell {
          shell = pkgs.fish;
          inputsFrom = [ app ];
          buildInputs = [
            pkgs.rustc
            pkgs.cargo
            pkgs.clippy
            pkgs.rustfmt
            pkgs.gcc
            pkgs.pkg-config
            pkgs.fontconfig
            pkgs.pre-commit
            pkgs.typos
            pkgs.hyperfine
          ];
          shellHook = ''
            export RUST_LOG=info
          '';
        };
      });
}
