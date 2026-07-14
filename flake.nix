{
  description = "AdaptTUI Rust development environment";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
  };

  outputs = {nixpkgs, ...}: let
    forAllSystems = function:
      nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed
      (system: function nixpkgs.legacyPackages.${system});
  in {
    formatter = forAllSystems (pkgs: pkgs.alejandra);

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer
          alejandra
          git
          just
        ];

        shellHook = ''
          export PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
          echo "adapt-tui development environment"
          echo "  cargo run       # launch the scaffold"
          echo "  cargo test      # run tests"
          echo "  just check      # format, lint, and test"
        '';
      };
    });
  };
}

