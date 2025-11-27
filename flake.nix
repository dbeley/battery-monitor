{
  description = "Battery monitor for Linux/NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        pythonPackages = pkgs.python3Packages;
        app = pythonPackages.buildPythonApplication {
          pname = "battery-monitor";
          version = "0.1.0";
          format = "pyproject";
          src = ./.;
          nativeBuildInputs = [ pythonPackages.hatchling ];
          propagatedBuildInputs = with pythonPackages; [ typer rich matplotlib ];
        };
      in {
        packages.default = app;
        apps.default = flake-utils.lib.mkApp { drv = app; };
        devShells.default = pkgs.mkShell {
          shell = pkgs.fish;
          buildInputs = [
            pkgs.python3
            pkgs.pre-commit
            pkgs.ruff
            pkgs.typos
            pythonPackages.hatchling
            pythonPackages.typer
            pythonPackages.rich
            pythonPackages.matplotlib
            pythonPackages.pytest
          ];
        };
      });
}
