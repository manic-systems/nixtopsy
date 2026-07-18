{
  description = "Interactively dissect your Nix closures.";

  outputs =
    { self, ... }@args:
    let
      inputs = (import ./.tack) { overrides = args.tackOverrides or { }; };
      inherit (inputs) fenix nixpkgs;
      inherit (nixpkgs) lib;
      forAllSystems = lib.genAttrs (lib.systems.doubles.linux ++ lib.systems.doubles.darwin);
      pkgsFor = system: nixpkgs.legacyPackages.${system} or (import nixpkgs { inherit system; });

      # Fenix only ships binaries for tier-1 arches, so fall back to nixpkgs's rustc
      # everywhere else.
      hasFenix = system: fenix.packages ? ${system};
      rustPlatformFor =
        system:
        let
          pkgs = pkgsFor system;
        in
        if hasFenix system then
          pkgs.makeRustPlatform { inherit (fenix.packages.${system}.latest) cargo rustc; }
        else
          pkgs.rustPlatform;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = self.packages.${system}.nixtopsy;
          nixtopsy = pkgs.callPackage ./nix/package.nix {
            rustPlatform = rustPlatformFor system;
          };
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          toolchain =
            if hasFenix system then
              fenix.packages.${system}.complete.withComponents [
                "cargo"
                "clippy"
                "rust-src"
                "rustc"
                "rustfmt"
                "rust-analyzer"
              ]
            else
              pkgs.rustc;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.nixtopsy ];
            packages = [
              toolchain
              pkgs.nixfmt
              pkgs.taplo
            ];
          };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          nixtopsy = self.packages.${system}.nixtopsy;
          rustTools =
            if hasFenix system then
              with fenix.packages.${system}.latest;
              [
                cargo
                rustfmt
              ]
            else
              [
                pkgs.cargo
                pkgs.rustfmt
              ];
          clippy = if hasFenix system then fenix.packages.${system}.latest.clippy else pkgs.clippy;
        in
        {
          default = pkgs.linkFarmFromDrvs "nixtopsy-checks" [
            nixtopsy
            self.checks.${system}.fmt
            self.checks.${system}.clippy
          ];

          inherit nixtopsy;

          fmt =
            pkgs.runCommand "nixtopsy-fmt-check"
              {
                nativeBuildInputs = rustTools ++ [
                  pkgs.nixfmt
                  pkgs.taplo
                ];
                src = ./.;
              }
              ''
                cp -r $src ./tree
                chmod -R +w ./tree
                cd ./tree
                cargo fmt --all -- --check
                taplo fmt --check
                find . -name '*.nix' -exec nixfmt --check {} +
                touch $out
              '';

          clippy = nixtopsy.overrideAttrs (old: {
            pname = "nixtopsy-clippy";
            nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ clippy ];
            buildPhase = ''
              runHook preBuild
              cargo clippy --all-targets --offline --locked -- -D warnings
              runHook postBuild
            '';
            checkPhase = "true";
            doCheck = false;
            installPhase = ''
              runHook preInstall
              touch $out
              runHook postInstall
            '';
          });
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
