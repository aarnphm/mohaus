{
  description = "mohaus: a maturin-shaped build backend for mixed Python and Mojo libraries";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    git-hooks-nix.url = "github:cachix/git-hooks.nix";
    git-hooks-nix.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    git-hooks-nix,
    nixpkgs,
    rust-overlay,
  }: let
    systems = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-linux"
    ];

    forAllSystems = fn:
      nixpkgs.lib.genAttrs systems (
        system:
          fn system (
            import nixpkgs {
              inherit system;
              overlays = [(import rust-overlay)];
            }
          )
      );

    rustToolchain = pkgs:
      pkgs.rust-bin.stable."1.93.0".default.override {
        extensions = [
          "clippy"
          "rust-src"
          "rustfmt"
        ];
      };

    commonPackages = pkgs: [
      (rustToolchain pkgs)
      pkgs.alejandra
      pkgs.deadnix
      pkgs.maturin
      pkgs.openssl
      pkgs.pkg-config
      pkgs.python311
      pkgs.statix
      pkgs.uv
    ];

    mkCommandApp = pkgs: name: text: {
      type = "app";
      program = "${
        pkgs.writeShellApplication {
          inherit name text;
          runtimeInputs = commonPackages pkgs;
        }
      }/bin/${name}";
    };
  in {
    devShells = forAllSystems (
      system: pkgs: let
        python = pkgs.python311;
        preCommit = self.checks.${system}.pre-commit;
      in {
        default = pkgs.mkShell {
          packages = (commonPackages pkgs) ++ preCommit.enabledPackages;

          env = {
            PYO3_PYTHON = "${python}/bin/python";
            UV_PYTHON = "${python}/bin/python";
          };

          shellHook = ''
            if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
              ${preCommit.shellHook}
            fi

            export VIRTUAL_ENV="$PWD/.venv"
            if [ ! -x "$VIRTUAL_ENV/bin/python" ]; then
              uv venv --python "${python}/bin/python" "$VIRTUAL_ENV"
            fi

            export PATH="$VIRTUAL_ENV/bin:$PWD/target/debug:$PATH"
            export PYO3_PYTHON="$VIRTUAL_ENV/bin/python"
            export UV_PYTHON="$VIRTUAL_ENV/bin/python"

            stamp="$VIRTUAL_ENV/.mohaus-editable-stamp"
            if [ ! -f "$stamp" ] \
              || [ pyproject.toml -nt "$stamp" ] \
              || [ Cargo.toml -nt "$stamp" ] \
              || [ Cargo.lock -nt "$stamp" ] \
              || find crates python -type f -newer "$stamp" -print -quit | grep -q .; then
              maturin develop --uv --extras dev
              touch "$stamp"
            fi

            completion_root="$VIRTUAL_ENV/share"
            mkdir -p \
              "$completion_root/bash-completion/completions" \
              "$completion_root/fish/vendor_completions.d" \
              "$completion_root/zsh/site-functions"
            mohaus completions bash > "$completion_root/bash-completion/completions/mohaus"
            mohaus completions fish > "$completion_root/fish/vendor_completions.d/mohaus.fish"
            mohaus completions zsh > "$completion_root/zsh/site-functions/_mohaus"

            export XDG_DATA_DIRS="$completion_root''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
            export FPATH="$completion_root/zsh/site-functions''${FPATH:+:$FPATH}"

            if [ -n "''${BASH_VERSION:-}" ] && type complete >/dev/null 2>&1; then
              . "$completion_root/bash-completion/completions/mohaus"
            fi
          '';
        };
      }
    );

    checks = forAllSystems (
      system: pkgs: let
        cargoFmtHook = pkgs.writeShellApplication {
          name = "mohaus-cargo-fmt";
          runtimeInputs = [(rustToolchain pkgs)];
          text = "cargo fmt --check";
        };

        mojoFormatHook = pkgs.writeShellApplication {
          name = "mohaus-mojo-format-check";
          runtimeInputs = [
            pkgs.coreutils
            pkgs.diffutils
          ];
          text = ''
            resolve_mojo() {
              if [ -n "''${MOHAUS_MOJO:-}" ] && [ -x "''${MOHAUS_MOJO:-}" ]; then
                printf '%s\n' "$MOHAUS_MOJO"
                return 0
              fi

              if command -v mojo >/dev/null 2>&1; then
                command -v mojo
                return 0
              fi

              if [ -n "''${MODULAR_HOME:-}" ] && [ -x "''${MODULAR_HOME:-}/bin/mojo" ]; then
                printf '%s\n' "$MODULAR_HOME/bin/mojo"
                return 0
              fi

              return 1
            }

            if [ "$#" -eq 0 ]; then
              exit 0
            fi

            if ! mojo="$(resolve_mojo)"; then
              printf '%s\n' "mojo format skipped: no executable found via \$MOHAUS_MOJO, \$PATH, or \$MODULAR_HOME/bin/mojo" >&2
              exit 0
            fi

            tmp="$(mktemp -d)"
            trap 'rm -rf "$tmp"' EXIT

            failed=0
            index=0
            for source in "$@"; do
              if [ ! -f "$source" ]; then
                continue
              fi

              case "$source" in
                *.mojo | *.🔥) ;;
                *) continue ;;
              esac

              copy="$tmp/$index.mojo"
              cp "$source" "$copy"
              "$mojo" format --line-length 119 --quiet "$copy"
              if ! diff -u "$source" "$copy" >&2; then
                echo "mojo format check failed: $source" >&2
                failed=1
              fi
              index=$((index + 1))
            done

            if [ "$failed" -ne 0 ]; then
              echo "run: mojo format --line-length 119 <files>" >&2
            fi
            exit "$failed"
          '';
        };
      in {
        pre-commit = git-hooks-nix.lib.${system}.run {
          src = ./.;
          hooks = {
            cargo-fmt = {
              enable = true;
              name = "cargo fmt";
              entry = "${cargoFmtHook}/bin/mohaus-cargo-fmt";
              pass_filenames = false;
            };

            mojo-format = {
              enable = true;
              name = "mojo format";
              entry = "${mojoFormatHook}/bin/mohaus-mojo-format-check";
              files = "\\.(mojo|🔥)$";
            };

            ruff-format = {
              enable = true;
              name = "ruff format";
              entry = "${pkgs.ruff}/bin/ruff format --config indent-width=2 --config line-length=119 --config preview=true --check";
              files = "\\.(py|pyi)$";
            };

            ruff-check = {
              enable = true;
              name = "ruff check";
              entry = "${pkgs.ruff}/bin/ruff check";
              files = "\\.(py|pyi)$";
            };

            alejandra = {
              enable = true;
              name = "alejandra";
              entry = "${pkgs.alejandra}/bin/alejandra --check";
              files = "\\.nix$";
            };

            deadnix = {
              enable = true;
              name = "deadnix";
              entry = "${pkgs.deadnix}/bin/deadnix --fail flake.nix";
              pass_filenames = false;
            };

            statix = {
              enable = true;
              name = "statix";
              entry = "${pkgs.statix}/bin/statix check flake.nix";
              pass_filenames = false;
            };

            check-added-large-files.enable = true;
            check-merge-conflicts.enable = true;
            check-toml.enable = true;
            end-of-file-fixer.enable = true;
            trim-trailing-whitespace.enable = true;
          };
        };
      }
    );

    apps = forAllSystems (
      _system: pkgs: {
        fmt = mkCommandApp pkgs "mohaus-fmt" ''
          cargo fmt
          uvx ruff format --config "indent-width=2" --config "line-length=119" --config "preview=true"
          alejandra flake.nix
        '';

        clippy = mkCommandApp pkgs "mohaus-clippy" ''
          cargo clippy --all --benches --tests --examples --all-features
        '';

        test = mkCommandApp pkgs "mohaus-test" ''
          cargo test --all-features
        '';

        ruff = mkCommandApp pkgs "mohaus-ruff" ''
          uvx ruff format --config "indent-width=2" --config "line-length=119" --config "preview=true" --check
          uvx ruff check python tests
        '';

        check = mkCommandApp pkgs "mohaus-check" ''
          cargo fmt --check
          cargo clippy --all --benches --tests --examples --all-features
          cargo test --all-features
          uvx ruff format --config "indent-width=2" --config "line-length=119" --config "preview=true" --check
          uvx ruff check python tests
          alejandra --check flake.nix
          deadnix --fail flake.nix
          statix check flake.nix
        '';
      }
    );

    formatter = forAllSystems (_system: pkgs: pkgs.alejandra);
  };
}
