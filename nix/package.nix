{
  lib,
  rustPlatform,
  stdenv,
  buildPackages,
  installShellFiles,
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

  nativeBuildInputs = [
    installShellFiles
  ]
  ++ lib.optionals hasWild [
    wild
    clang
  ];

  env = lib.optionalAttrs hasWild {
    RUSTFLAGS = "-Clinker=${clang}/bin/${clang.targetPrefix}clang -Clink-arg=--ld-path=wild";
  };

  postInstall = lib.optionalString (stdenv.hostPlatform.emulatorAvailable buildPackages) (
    let
      nixtopsy = "${stdenv.hostPlatform.emulator buildPackages} $out/bin/nixtopsy";
    in
    ''
      installShellCompletion --cmd nixtopsy \
        --bash    <(COMPLETE=bash ${nixtopsy}) \
        --zsh     <(COMPLETE=zsh  ${nixtopsy}) \
        --fish    <(COMPLETE=fish ${nixtopsy}) \
        --nushell <(COMPLETE=nu   ${nixtopsy})
    ''
  );

  meta = {
    description = "Interactively dissect your Nix closures";
    license = lib.licenses.mit;
    maintainers = [ lib.maintainers.amaanq ];
    mainProgram = "nixtopsy";
  };
}
