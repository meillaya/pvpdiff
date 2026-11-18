{
  pkgs,
  config,
  ...
}:
{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  # The Rust toolchain comes from rust-overlay through `languages.rust`; these
  # packages are the development and test tooling around it. GHC/cabal are
  # deliberately absent: local-mode checks live in `devShells.local` so the
  # default shell and CI stay small.
  packages = with pkgs; [
    cacert
    cmake
    curl
    git
    jq
    nix
    perl
    pkg-config
    (python3.withPackages (ps: [
      ps.jsonschema
      ps.pyyaml
    ]))
    actionlint
    cargo-deny
    cargo-insta
    shellcheck
  ];

  env = {
    CARGO_HOME = "${config.devenv.root}/.cargo-home";
    CARGO_TERM_COLOR = "always";
    RUST_BACKTRACE = "1";
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    PVPDIFF_HACKAGE_URL = "https://hackage.haskell.org";
  };

  scripts = {
    test.exec = "cargo test --workspace --all-features";
    check.exec = "cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features";
    fixtures.exec = "bash scripts/fetch-fixtures.sh";
    oracle.exec = "bash scripts/check-oracle.sh";
    deny.exec = "cargo deny check";
    lint-actions.exec = "actionlint .github/workflows/*.yml && shellcheck scripts/*.sh";
    live.exec = "cargo test --workspace --all-features --test live_hackage -- --ignored";
    schema.exec = "cargo run -p pvpdiff-cli -- schema > schemas/pvpdiff-v1.schema.json";
    survey-smoke.exec = "cargo run -p pvpdiff-cli -- survey run --limit 10 --offline";
  };

  enterShell = ''
    printf 'rustc  %s\n' "$(rustc --version)"
    printf 'cargo  %s\n' "$(cargo --version)"
    printf 'nix    %s\n' "$(nix --version)"
    echo
    echo 'check:  devenv test'
    echo 'tests:  cargo test --workspace --all-features'
    echo 'local:  nix develop .#local   (GHC/cabal for pvpdiff check)'
  '';

  enterTest = ''
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
  '';
}
