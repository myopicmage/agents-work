{
  description = "Append-only working papers for asynchronous agent collaboration";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs, ... }:
    let
      supportedSystems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          pythonPackage = pkgs.writeShellApplication {
            name = "agents-work";
            runtimeInputs = [ pkgs.python3 ];
            text = ''
              exec python3 ${./python/agents_work.py} "$@"
            '';
          };
          rustPackage = pkgs.rustPlatform.buildRustPackage {
            pname = "agents-work";
            version = "0.1.0";
            src = ./rust;

            cargoLock.lockFile = ./rust/Cargo.lock;

            meta.mainProgram = "agents-work";
          };
        in
        {
          default = pythonPackage;
          python = pythonPackage;
          rust = rustPackage;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          python-tests = pkgs.runCommand "agents-work-python-tests" {
            nativeBuildInputs = [ pkgs.python3 ];
          } ''
            cp -R ${./python} ./python
            chmod -R u+w ./python
            python3 -m unittest discover -s python -p 'test_*.py'
            touch "$out"
          '';

          rust = self.packages.${system}.rust;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShellNoCC {
            packages = [
              pkgs.cargo
              pkgs.clippy
              pkgs.python3
              pkgs.rust-analyzer
              pkgs.rustc
              pkgs.rustfmt
            ];

            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        }
      );
    };
}
