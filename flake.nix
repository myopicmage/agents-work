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
      source = builtins.path {
        path = ./.;
        name = "agents-work-source";
        filter =
          path: type:
          let
            name = builtins.baseNameOf path;
          in
          !builtins.elem name [
            ".direnv"
            ".git"
            "__pycache__"
            "result"
            "target"
          ]
          && builtins.match ".*[.]pyc" name == null;
      };
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
            src = "${source}/rust";

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
          install-tests = pkgs.runCommand "agents-work-install-tests" {
            nativeBuildInputs = [
              pkgs.coreutils
              pkgs.python3
            ];
          } ''
            cp -R ${source} ./source
            chmod -R u+w ./source
            cd ./source
            export HOME="$TMPDIR/home"
            mkdir -p "$HOME"
            ./tests/install_test.sh --implementation python
            touch "$out"
          '';

          python-tests = pkgs.runCommand "agents-work-python-tests" {
            nativeBuildInputs = [ pkgs.python3 ];
          } ''
            cp -R ${./python} ./python
            chmod -R u+w ./python
            python3 -m unittest discover -s python -p 'test_*.py'
            touch "$out"
          '';

          rust = self.packages.${system}.rust;

          shell-scripts = pkgs.runCommand "agents-work-shell-scripts" {
            nativeBuildInputs = [ pkgs.shellcheck ];
          } ''
            shellcheck \
              ${./install.sh} \
              ${./uninstall.sh} \
              ${./scripts/check} \
              ${./tests/install_test.sh}
            touch "$out"
          '';
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
              pkgs.shellcheck
            ];

            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        }
      );
    };
}
