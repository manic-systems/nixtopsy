# nixtopsy

Interactively dissect your Nix closures.

## Install

Add this flake as an input to your system config:

```nix
{
  inputs.nixtopsy = {
    url = "github:amaanq/nixtopsy";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, nixtopsy, ... }: {
    nixosConfigurations.${host} = nixpkgs.lib.nixosSystem {
      modules = [
        ({ pkgs, system, ... }: {
          environment.systemPackages = [ nixtopsy.packages.${system}.default ];
        })
      ];
    };
  };
}
```

Or build locally:

```sh
nix build
./result/bin/nixtopsy
```

Or, if you just want it on your system imperatively:

```sh
nix profile install github:amaanq/nixtopsy
```

## Usage

```sh
# Default roots: ~/.nix-profile and /var/run/current-system
nixtopsy

# A specific store path
nixtopsy /nix/store/...-hello-2.12.3

# A flake reference (this will evaluate and uses the realized output)
nixtopsy 'nixpkgs#hello'
nixtopsy '.#nixosConfigurations.<host>.config.system.build.toplevel'

# Print the dependency graph as Graphviz DOT
nixtopsy --dot 'nixpkgs#hello'

# Inspect build-time deps instead of the realized closure
nixtopsy --derivation '.#'

# Query a remote/binary cache without downloading
nix eval --raw 'nixpkgs#<some-package>.outPath' \
  | xargs -o nixtopsy --store https://cache.nixos.org
```

## Keybindings

| Key             | Action                                                      |
| --------------- | ----------------------------------------------------------- |
| `hjkl` / arrows | Navigate panes                                              |
| `j`/`k`         | Move selection up/down                                      |
| `h`/`l`         | Move between parent/child panes                             |
| `PgUp`/`PgDn`   | Page through long lists                                     |
| `s`             | Cycle sort order (closure size → added size → alphabetical) |
| `w`             | Show why-depends modal                                      |
| `/`             | Search by name                                              |
| `y`             | Yank selected store path to clipboard                       |
| `?`             | Show help                                                   |
| `q` / `Esc`     | Quit or close modal                                         |

## Glossary

- **NAR Size** - size of the store path itself.
- **Closure Size** - total size of the store path and all transitive dependencies.
- **Added Size** - size that disappears from the closure if this path is removed.
  This is useful for finding what's actually pulling weight in your system.

## Development

```sh
nix develop
cargo build --release
cargo test
cargo clippy --all-targets
```

## License

[MIT](LICENSE)
