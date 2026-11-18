{
  description = "pvpdiff - PVP-aware public API diff for Hackage packages";

  # `devenv.nix` is the single source for the development shell. devenv and
  # rust-overlay follow the project nixpkgs, so the package, `devenv shell`, and
  # `nix develop` build the same tools. Full revisions keep lock resolution off
  # the GitHub API. The revisions match the working `hoard` scaffold; the
  # committed locks are copied from there as well.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/d5cc6833999746cffdfe0598f7bc801a57b887a9";
    devenv = {
      url = "github:cachix/devenv/b904dcb51fe48c30db250038241507f60752f222";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay/67f1944c7c99d8dbb57fa3d83043bc0a92cbcc56";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      ...
    }@inputs:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

      mkPvpdiff =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "pvpdiff";
          inherit version;
          src = self;

          # `cargoLock` makes the dependency set offline and reproducible: each
          # crate comes from the lockfile. Tests that require Hackage access
          # are `#[ignore]`d; the default suite runs fully offline.
          cargoLock.lockFile = ./Cargo.lock;

          # reqwest and aws-lc-rs: nixpkgs sets AWS_LC_SYS_USE_SYSTEM=1 for
          # Rust builds, so provide the library instead of letting aws-lc-sys
          # try to build its own.
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.aws-lc ];
          env = {
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            SSL_CERT_DIR = "${pkgs.cacert}/etc/ssl/certs";
          };

          cargoBuildFlags = [ "--workspace" ];
          cargoTestFlags = [ "--workspace" ];

          meta = with pkgs.lib; {
            description = "PVP-aware public API diff for two Hackage versions";
            license = licenses.asl20;
            mainProgram = "pvpdiff";
            platforms = platforms.unix;
          };
        };
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          pvpdiff = mkPvpdiff pkgs;
        in
        {
          pvpdiff = pvpdiff;
          default = pvpdiff;
        }
      );

      devShells = forAllSystems (pkgs: {
        default = devenv.lib.mkShell {
          inherit inputs pkgs;
          modules = [
            # Flakes evaluate purely by default, where PWD is not available.
            # Use `nix develop --impure` for the real working directory; the
            # fallback lets `nix flake check` evaluate the shell without
            # failing.
            ({ lib, ... }: {
              devenv.root = lib.mkForce (
                let
                  pwd = builtins.getEnv "PWD";
                in
                if pwd != "" then pwd else "/tmp/pvpdiff-devenv"
              );
            })
            ./devenv.nix
          ];
        };

        # Local mode (`pvpdiff check --package-dir .`) shells out to
        # `cabal haddock --haddock-hoogle`. Keeping GHC out of the default
        # shell avoids pulling a compiler into every CI run; this shell is
        # used by the opt-in local-mode job and by maintainers.
        local = pkgs.mkShell {
          packages = with pkgs; [
            cabal-install
            cacert
            cargo
            ghc
            git
            jq
            pkg-config
            rustc
          ];
          shellHook = ''
            echo "pvpdiff local-mode shell"
            echo "  ghc:   $(ghc --version)"
            echo "  cabal: $(cabal --version | head -1)"
            echo "  cargo: $(cargo --version)"
          '';
        };
      });

      apps = forAllSystems (
        pkgs:
        let
          pvpdiff = mkPvpdiff pkgs;
        in
        {
          default = {
            type = "app";
            program = "${pvpdiff}/bin/pvpdiff";
          };
        }
      );

      checks = forAllSystems (pkgs: {
        default = mkPvpdiff pkgs;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
