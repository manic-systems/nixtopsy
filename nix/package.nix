{
  lib,
  rustPlatform,
  stdenv,
  buildPackages,
}:
let
  inherit (buildPackages) clang wild;
  # wild + clang are only used on Linux tier-1 arches
  hasWild =
    stdenv.hostPlatform.isLinux && (stdenv.hostPlatform.isx86_64 || stdenv.hostPlatform.isAarch64);
in
rustPlatform.buildRustPackage {
  pname = "nixtopsy";
  version = (lib.importTOML ../Cargo.toml).package.version;

  src = ../.;
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = lib.optionals hasWild [
    wild
    clang
  ];

  env = lib.optionalAttrs hasWild {
    RUSTFLAGS = "-Clinker=${clang}/bin/${clang.targetPrefix}clang -Clink-arg=--ld-path=wild";
  };

  meta = {
    description = "Interactively dissect your Nix closures";
    license = lib.licenses.mit;
    maintainers = [ lib.maintainers.amaanq ];
    mainProgram = "nixtopsy";
  };
}
