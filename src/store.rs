use std::{
   collections::{
      BTreeSet,
      HashMap,
   },
   error::Error,
   fmt,
   ops::{
      Index,
      IndexMut,
   },
   process::{
      Command,
      Stdio,
   },
};

use serde::Deserialize;
use serde_json::{
   Map,
   Value,
};

/// A representation of a compact index into [`StoreEnv::paths`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(usize);

impl<T> Index<NodeId> for [T] {
   type Output = T;

   fn index(&self, index: NodeId) -> &T {
      &self[index.0]
   }
}

impl<T> IndexMut<NodeId> for [T] {
   fn index_mut(&mut self, index: NodeId) -> &mut T {
      &mut self[index.0]
   }
}

impl<T> Index<NodeId> for Vec<T> {
   type Output = T;

   fn index(&self, index: NodeId) -> &T {
      &self[index.0]
   }
}

impl<T> IndexMut<NodeId> for Vec<T> {
   fn index_mut(&mut self, index: NodeId) -> &mut T {
      &mut self[index.0]
   }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum NixStore {
   Default,
   Custom(String),
}

impl NixStore {
   pub fn from_path(p: &str) -> Self {
      let mut path = p.to_string();
      if !path.ends_with('/') {
         path.push('/');
      }
      if path == "/nix/store/" {
         Self::Default
      } else {
         Self::Custom(path)
      }
   }

   pub const fn as_str(&self) -> &str {
      match self {
         Self::Default => "/nix/store/",
         Self::Custom(s) => s.as_str(),
      }
   }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct StoreName {
   pub name: String,
}

impl StoreName {
   pub fn from_path(store: &NixStore, path: &str) -> Option<Self> {
      let prefix = store.as_str();
      if !path.starts_with(prefix) {
         return None;
      }
      let rest = &path[prefix.len()..];
      let first = rest.split('/').next().filter(|s| !s.is_empty())?;
      Some(Self {
         name: first.to_string(),
      })
   }

   pub fn to_path(&self, store: &NixStore) -> String {
      let mut s = store.as_str().to_string();
      s.push_str(&self.name);
      s
   }

   /// Returns (hash-with-trailing-dash, rest).
   pub fn split_short(&self) -> (&str, &str) {
      self.name.find('-').map_or((self.name.as_str(), ""), |idx| {
         (&self.name[..idx + 1], &self.name[idx + 1..])
      })
   }

   pub fn short(&self) -> &str {
      let (_, s) = self.split_short();
      if s.is_empty() { self.name.as_str() } else { s }
   }

   pub fn short_with_disambiguation(&self, n: usize) -> String {
      if n == 0 {
         return self.short().to_string();
      }
      let (f, s) = self.split_short();
      if s.is_empty() {
         return self.name.clone();
      }
      let take = f.chars().take(n).collect::<String>();
      format!("{take}...-{s}")
   }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct NixPathSignature {
   pub key_name:  String,
   pub signature: String,
}

#[derive(Debug, Clone)]
pub struct StorePath<P> {
   pub name:       StoreName,
   pub size:       u64,
   pub refs:       Vec<NodeId>,
   pub signatures: Vec<NixPathSignature>,
   pub payload:    P,
}

#[derive(Debug)]
pub struct StoreEnv<P> {
   pub store: NixStore,
   pub paths: Vec<StorePath<P>>,
   pub roots: Vec<NodeId>,
}

#[derive(Debug)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      f.write_str(&self.0)
   }
}

impl Error for StoreError {}

pub type DynErr = Box<dyn Error + Send + Sync>;

fn err(msg: impl Into<String>) -> DynErr {
   Box::new(StoreError(msg.into()))
}

pub fn get_store_dir(store_url: Option<&str>) -> Result<NixStore, DynErr> {
   let mut cmd = Command::new("nix-instantiate");
   cmd.args(["--eval", "--expr", "(builtins.storeDir)", "--json"]);
   if let Some(url) = store_url {
      cmd.args(["--option", "store", url]);
   }
   let out = cmd
      .output()
      .map_err(|e| err(format!("running nix-instantiate failed: {e}")))?;
   if !out.status.success() {
      return Err(err(format!(
         "nix-instantiate exited with status {:?}: {}",
         out.status.code(),
         String::from_utf8_lossy(&out.stderr)
      )));
   }
   let parsed = serde_json::from_slice::<String>(&out.stdout)
      .map_err(|e| err(format!("parsing nix-instantiate output: {e}")))?;
   Ok(NixStore::from_path(&parsed))
}

pub struct PathInfoOpts<'a> {
   pub recursive:  bool,
   pub derivation: bool,
   pub impure:     bool,
   pub store_url:  Option<&'a str>,
   pub file:       Option<&'a str>,
}

fn parse_sig(raw: &str) -> Option<NixPathSignature> {
   let (k, sig) = raw.split_once(':')?;
   if k.is_empty() || sig.is_empty() {
      return None;
   }
   Some(NixPathSignature {
      key_name:  k.to_string(),
      signature: sig.to_string(),
   })
}

#[derive(Deserialize)]
struct RawInfo {
   #[serde(rename = "narSize")]
   nar_size:   u64,
   references: Vec<String>,
   #[serde(default)]
   signatures: Vec<String>,
}

/// Intermediate ingest type: refs are still string `StoreName`s. We convert to
/// `Vec<NodeId>` in [`build_env`] after collecting all paths.
pub struct RawPath {
   pub name:       StoreName,
   pub size:       u64,
   pub refs:       Vec<StoreName>,
   pub signatures: Vec<NixPathSignature>,
}

pub fn get_path_info(
   store: &NixStore,
   opts: &PathInfoOpts,
   installables: &[String],
) -> Result<Vec<RawPath>, DynErr> {
   let mut cmd = Command::new("nix");
   cmd.args(["path-info", "--json", "--json-format", "2"]);
   if opts.impure {
      cmd.arg("--impure");
   }
   if opts.recursive {
      cmd.arg("--recursive");
   }
   if opts.derivation {
      cmd.arg("--derivation");
   }
   if let Some(url) = opts.store_url {
      cmd.args(["--store", url]);
   }
   if let Some(file) = opts.file {
      cmd.args(["--file", file]);
   }
   cmd.args(["--extra-experimental-features", "nix-command flakes"]);
   for inst in installables {
      cmd.arg(inst);
   }

   let out = cmd
      .stderr(Stdio::piped())
      .output()
      .map_err(|e| err(format!("running `nix path-info` failed: {e}")))?;
   if !out.status.success() {
      return Err(err(format!(
         "`nix path-info` exited with status {:?}: {}",
         out.status.code(),
         String::from_utf8_lossy(&out.stderr)
      )));
   }

   let wrapper = serde_json::from_slice::<Map<String, Value>>(&out.stdout)
      .map_err(|e| err(format!("parsing nix path-info output: {e}")))?;
   let Some((_, Value::Object(entries))) = wrapper.into_iter().find(|(k, _)| k == "info") else {
      return Err(err("unexpected nix path-info output: missing `info` field"));
   };

   let from_bare = |bare: &str| -> Option<StoreName> {
      // Bare names have no prefix
      let mut full = store.as_str().to_string();
      full.push_str(bare);
      StoreName::from_path(store, &full)
   };

   let mut result = Vec::with_capacity(entries.len());
   for (bare_path, value) in entries {
      let raw = match value {
         Value::Null => {
            return Err(err(format!(
               "Invalid path: {bare_path}. Make sure that it is built, or pass '--derivation' if \
                you want to work on the derivation."
            )));
         },
         v => {
            serde_json::from_value::<RawInfo>(v)
               .map_err(|e| err(format!("parsing path-info entry for {bare_path}: {e}")))?
         },
      };

      let name = from_bare(&bare_path)
         .ok_or_else(|| err(format!("failed parsing Nix store path: {bare_path}")))?;
      let mut refs = Vec::<StoreName>::with_capacity(raw.references.len());
      for r in raw.references {
         let n = from_bare(&r).ok_or_else(|| err(format!("failed parsing Nix store path: {r}")))?;
         if n != name {
            refs.push(n);
         }
      }
      let signatures = raw.signatures.iter().filter_map(|s| parse_sig(s)).collect();

      result.push(RawPath {
         name,
         size: raw.nar_size,
         refs,
         signatures,
      });
   }

   if result.is_empty() {
      return Err(err("invariant violation: get_path_info returned []"));
   }
   Ok(result)
}

pub struct StoreEnvOptions<'a> {
   pub derivation: bool,
   pub impure:     bool,
   pub store_url:  Option<&'a str>,
   pub file:       Option<&'a str>,
}

pub fn build_env(opts: &StoreEnvOptions, installables: &[String]) -> Result<StoreEnv<()>, DynErr> {
   let store = if opts.store_url.is_some() {
      // User explicitly chose a store, so ask nix-instantiate for its `storeDir`.
      get_store_dir(opts.store_url)?
   } else if let Some(dir) = std::env::var_os("NIX_STORE_DIR") {
      // In a non-default daemon configuration, we trust the env var without spawning
      // nix-instantiate.
      NixStore::from_path(&dir.to_string_lossy())
   } else {
      NixStore::Default
   };

   let roots = get_path_info(
      &store,
      &PathInfoOpts {
         recursive:  false,
         derivation: opts.derivation,
         impure:     opts.impure,
         store_url:  opts.store_url,
         file:       opts.file,
      },
      installables,
   )?;

   let root_paths = roots
      .iter()
      .map(|p| p.name.to_path(&store))
      .collect::<Vec<String>>();

   let all = get_path_info(
      &store,
      &PathInfoOpts {
         recursive:  true,
         derivation: opts.derivation,
         impure:     opts.impure,
         store_url:  opts.store_url,
         file:       None,
      },
      &root_paths,
   )?;

   // Assign a `NodeId` to every path.
   let mut by_name = HashMap::<StoreName, NodeId>::with_capacity(all.len());
   for (i, p) in all.iter().enumerate() {
      by_name.insert(p.name.clone(), NodeId(i));
   }

   // Resolve string refs to `NodeId`s and build the final `StorePath`s.
   let mut paths = Vec::<StorePath<()>>::with_capacity(all.len());
   for p in all {
      let refs = p
         .refs
         .iter()
         .map(|n| {
            by_name
               .get(n)
               .copied()
               .ok_or_else(|| err(format!("unknown reference {}", n.name)))
         })
         .collect::<Result<Vec<NodeId>, _>>()?;
      paths.push(StorePath {
         name: p.name,
         size: p.size,
         refs,
         signatures: p.signatures,
         payload: (),
      });
   }

   let root_ids = roots
      .iter()
      .map(|r| {
         by_name
            .get(&r.name)
            .copied()
            .ok_or_else(|| err(format!("root {} missing from closure", r.name.name)))
      })
      .collect::<Result<Vec<NodeId>, _>>()?;

   Ok(StoreEnv {
      store,
      paths,
      roots: root_ids,
   })
}

impl<P> StoreEnv<P> {
   pub fn get(&self, id: NodeId) -> &StorePath<P> {
      &self.paths[id]
   }

   pub fn enumerate(&self) -> impl Iterator<Item = (NodeId, &StorePath<P>)> {
      self.paths.iter().enumerate().map(|(i, sp)| (NodeId(i), sp))
   }

   pub fn all(&self) -> impl Iterator<Item = &StorePath<P>> {
      self.paths.iter()
   }

   pub fn get_roots(&self) -> Vec<&StorePath<P>> {
      self.roots.iter().map(|&id| self.get(id)).collect()
   }

   /// DFS over the graph collecting all paths reachable from `starts` for which
   /// `predicate(id)` is true. This returns paths in DFS pre-order.
   pub fn fetch_refs<F>(&self, predicate: F, starts: &[NodeId]) -> Vec<&StorePath<P>>
   where
      F: Fn(NodeId) -> bool,
   {
      let mut acc = Vec::<&StorePath<P>>::new();
      let mut visited = vec![false; self.paths.len()];
      let mut stack = starts.iter().rev().copied().collect::<Vec<NodeId>>();
      while let Some(id) = stack.pop() {
         if visited[id] || !predicate(id) {
            continue;
         }
         visited[id] = true;
         let sp = &self.paths[id];
         acc.push(sp);
         for &r in sp.refs.iter().rev() {
            if !visited[r] {
               stack.push(r);
            }
         }
      }
      acc
   }
}

/// A bottom-up fold over the dependency graph.
///
/// Each node's new payload is computed from itself plus the new payloads of its
/// references. The callback receives the current node's `StorePath` (with the
/// old payload) plus references to the already-computed payloads of its direct
/// refs.
pub fn bottom_up<A, B, F>(env: &StoreEnv<A>, mut f: F) -> StoreEnv<B>
where
   F: FnMut(NodeId, &StorePath<A>, &[&B]) -> B,
{
   enum Frame {
      Enter(NodeId),
      Exit(NodeId),
   }

   let mut new_payloads = std::iter::repeat_with(|| Option::<B>::None)
      .take(env.paths.len())
      .collect::<Vec<_>>();

   let mut stack = env
      .roots
      .iter()
      .rev()
      .copied()
      .map(Frame::Enter)
      .collect::<Vec<Frame>>();

   while let Some(frame) = stack.pop() {
      match frame {
         Frame::Enter(id) => {
            if new_payloads[id].is_some() {
               continue;
            }
            let sp = env.get(id);
            stack.push(Frame::Exit(id));
            for &r in sp.refs.iter().rev() {
               if new_payloads[r].is_none() {
                  stack.push(Frame::Enter(r));
               }
            }
         },
         Frame::Exit(id) => {
            if new_payloads[id].is_some() {
               continue;
            }
            let sp = env.get(id);
            let child_payloads = sp
               .refs
               .iter()
               .map(|&r| {
                  new_payloads[r]
                     .as_ref()
                     .expect("invariant: child not computed")
               })
               .collect::<Vec<&B>>();
            let payload = f(id, sp, &child_payloads);
            new_payloads[id] = Some(payload);
         },
      }
   }

   // All paths in env.paths are reachable from roots as we built `env` from a
   // recursive `nix path-info` on roots, meaning every slot must be populated.
   let new_paths = env
      .paths
      .iter()
      .zip(new_payloads)
      .map(|(sp, payload)| {
         StorePath {
            name:       sp.name.clone(),
            size:       sp.size,
            refs:       sp.refs.clone(),
            signatures: sp.signatures.clone(),
            payload:    payload.expect("invariant: unreachable node had no payload"),
         }
      })
      .collect::<Vec<StorePath<B>>>();

   StoreEnv {
      store: env.store.clone(),
      paths: new_paths,
      roots: env.roots.clone(),
   }
}

/// Render the dependency graph as a DOT string. Edges are deduplicated by short
/// name.
pub fn to_dot<P>(env: &StoreEnv<P>) -> String {
   let mut edges = BTreeSet::<(String, String)>::new();
   for sp in &env.paths {
      for &r in &sp.refs {
         edges.insert((
            sp.name.short().to_string(),
            env.get(r).name.short().to_string(),
         ));
      }
   }
   crate::dot::render(&edges)
}
