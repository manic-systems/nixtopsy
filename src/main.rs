use std::{
   env,
   error::Error,
   fs,
   path::PathBuf,
   process::ExitCode,
};

use lexopt::prelude::*;
use nixtopsy::{
   hrfsize::SizeUnits,
   stats::calculate_path_stats,
   store::{
      StoreEnvOptions,
      build_env,
      to_dot,
   },
   tui,
};

const HELP: &str = "Usage: nixtopsy [INSTALLABLE]... [--store STORE] [--file FILE] [--version] [--derivation] [--impure] [--si] [--dot]

  Interactively dissect your Nix closures.

Available options:
  INSTALLABLE              A store path or a flake reference.
                           Paths default to \"~/.nix-profile\" and \"/var/run/current-system\"
  --store STORE            The URL of the Nix store, e.g. \"daemon\" or \"https://cache.nixos.org\"
                           See \"nix help-stores\" for supported store types and settings.
  --file FILE              Interpret installables as attribute paths relative to the Nix expression stored in file.
  --version                Show the nixtopsy version
  --derivation             Operate on the store derivation rather than its outputs
  --impure                 Allow access to mutable paths and repositories
  --si                     Show sizes in SI units (KB, MB, GB) instead of binary (KiB, MiB, GiB)
  --dot                    Print the dependency graph in dot format
  -h,--help                Show this help text

Keybindings:
  hjkl/Arrow Keys : Navigate
  w               : Open why-depends modal
  /               : Open search modal
  s               : Change sort order
  y               : Yank selected path to clipboard
  ?               : Show help
  q/Esc           : Quit / close modal
";

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default)]
struct Opts {
   installables: Vec<String>,
   store:        Option<String>,
   file:         Option<String>,
   version:      bool,
   derivation:   bool,
   impure:       bool,
   si:           bool,
   dot:          bool,
}

fn parse_args() -> Result<Opts, Box<dyn Error + Send + Sync>> {
   let mut opts = Opts::default();
   let mut parser = lexopt::Parser::from_env();
   while let Some(arg) = parser.next()? {
      match arg {
         Long("store") => opts.store = Some(parser.value()?.string()?),
         Long("file") => opts.file = Some(parser.value()?.string()?),
         Long("version") => opts.version = true,
         Long("derivation") => opts.derivation = true,
         Long("impure") => opts.impure = true,
         Long("si") => opts.si = true,
         Long("dot") => opts.dot = true,
         Long("help") | Short('h') => {
            print!("{HELP}");
            std::process::exit(0);
         },
         Value(v) => opts.installables.push(v.string()?),
         _ => return Err(arg.unexpected().into()),
      }
   }
   Ok(opts)
}

fn default_roots() -> Vec<String> {
   let mut candidates = Vec::<PathBuf>::new();

   if let Some(home) = env::var_os("HOME") {
      candidates.push(PathBuf::from(&home).join(".nix-profile"));
   }
   if let Some(xdg_state) = env::var_os("XDG_STATE_HOME") {
      let p = PathBuf::from(xdg_state);
      candidates.push(p.join("nix/profile"));
      candidates.push(p.join("nix/profiles/home-manager"));
   } else if let Some(home) = env::var_os("HOME") {
      let p = PathBuf::from(home).join(".local/state");
      candidates.push(p.join("nix/profile"));
      candidates.push(p.join("nix/profiles/home-manager"));
   }
   candidates.push(PathBuf::from("/var/run/current-system"));

   candidates
      .into_iter()
      .filter(is_valid_root)
      .filter_map(|p| p.into_os_string().into_string().ok())
      .collect()
}

fn is_valid_root(path: &PathBuf) -> bool {
   // It has to be a directory AND symlink, since nix interprets a plain
   // directory as a flake.
   let Ok(meta) = fs::symlink_metadata(path) else {
      return false;
   };
   if !meta.file_type().is_symlink() {
      return false;
   }
   fs::metadata(path).is_ok_and(|m| m.is_dir())
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
   let opts = parse_args()?;

   if opts.version {
      println!("nixtopsy {VERSION}");
      return Ok(());
   }

   let installables = if opts.installables.is_empty() {
      let roots = default_roots();
      if roots.is_empty() {
         return Err("No store path given.".into());
      }
      roots
   } else {
      opts.installables
   };

   let seo = StoreEnvOptions {
      derivation: opts.derivation,
      impure:     opts.impure,
      store_url:  opts.store.as_deref(),
      file:       opts.file.as_deref(),
   };

   let env = build_env(&seo, &installables)?;
   let env = calculate_path_stats(&env);

   if opts.dot {
      println!("{}", to_dot(&env));
      return Ok(());
   }

   let units = if opts.si {
      SizeUnits::Si
   } else {
      SizeUnits::Binary
   };
   tui::run(env, units)?;
   Ok(())
}

fn main() -> ExitCode {
   match run() {
      Ok(()) => ExitCode::SUCCESS,
      Err(e) => {
         eprintln!("Error: {e}");
         ExitCode::FAILURE
      },
   }
}
