use std::collections::{
   BTreeSet,
   HashMap,
   HashSet,
   VecDeque,
};

use crate::store::{
   NodeId,
   StoreEnv,
   StoreName,
   bottom_up,
};

#[derive(Debug, Clone)]
pub struct PathStats {
   pub total_size:           u64,
   pub added_size:           u64,
   pub immediate_parents:    Vec<NodeId>,
   pub disambiguation_chars: usize,
}

/// For each node, the set of all transitive refs (descendants) along with their
/// NAR sizes, deduplicated. This is used to compute the closure size.
fn intermediate_refs(env: &StoreEnv<()>) -> Vec<HashMap<NodeId, u64>> {
   let mut all_refs = std::iter::repeat_with(HashMap::<NodeId, u64>::new)
      .take(env.paths.len())
      .collect::<Vec<_>>();

   // bottom_up gives us post-order visitation; we populate the side table from
   // the closure.
   let _ = bottom_up::<_, (), _>(env, |id, sp, _| {
      let mut combined = HashMap::<NodeId, u64>::new();
      for &r in &sp.refs {
         combined.insert(r, env.get(r).size);
         for (k, v) in &all_refs[r as usize] {
            combined.insert(*k, *v);
         }
      }
      all_refs[id as usize] = combined;
   });

   all_refs
}

fn calculate_env_size(env: &StoreEnv<()>, refs: &[HashMap<NodeId, u64>]) -> u64 {
   let mut combined = HashMap::<NodeId, u64>::new();
   for &root in &env.roots {
      combined.insert(root, env.get(root).size);
      for (k, v) in &refs[root as usize] {
         combined.insert(*k, *v);
      }
   }
   combined.values().sum()
}

fn immediate_parents_map(env: &StoreEnv<()>) -> Vec<BTreeSet<NodeId>> {
   let mut m = std::iter::repeat_with(BTreeSet::<NodeId>::new)
      .take(env.paths.len())
      .collect::<Vec<_>>();
   for (parent_id, sp) in env.paths.iter().enumerate() {
      let parent_id = parent_id as NodeId;
      for &r in &sp.refs {
         m[r as usize].insert(parent_id);
      }
   }
   m
}

fn disambiguation_chars(env: &StoreEnv<()>) -> Vec<usize> {
   // Bucket NodeIds by their short text. For each bucket, find the smallest n
   // (step of 2) such that `short_with_disambiguation(n)` makes all names in
   // the bucket distinct.
   let mut by_short = HashMap::<String, Vec<NodeId>>::new();
   for (id, sp) in env.paths.iter().enumerate() {
      let (_, short) = sp.name.split_short();
      by_short
         .entry(short.to_string())
         .or_default()
         .push(id as NodeId);
   }

   let mut out = vec![0_usize; env.paths.len()];
   for ids in by_short.values() {
      let names = ids.iter().map(|&id| &env.get(id).name).collect::<Vec<_>>();
      let n = disambiguate(&names);
      for &id in ids {
         out[id as usize] = n;
      }
   }
   out
}

fn disambiguate(names: &[&StoreName]) -> usize {
   let mut n = 0;
   loop {
      if all_unique(names, n) {
         return n;
      }
      n += 2;
      if n > 64 {
         // sane cap; shouldn't realistically happen
         return n;
      }
   }
}

fn all_unique(names: &[&StoreName], n: usize) -> bool {
   let mut seen = HashSet::<String>::with_capacity(names.len());
   for name in names {
      let key = name.short_with_disambiguation(n);
      if !seen.insert(key) {
         return false;
      }
   }
   true
}

pub fn calculate_path_stats(env: &StoreEnv<()>) -> StoreEnv<PathStats> {
   let refs = intermediate_refs(env);
   let total = calculate_env_size(env, &refs);
   let parents = immediate_parents_map(env);
   let disamb = disambiguation_chars(env);

   bottom_up::<_, PathStats, _>(env, |id, sp, _children| {
      // Closure size = own size + sum of all transitive refs.
      let own = sp.size;
      let transitive = refs[id as usize].values().sum::<u64>();

      // Added size = total - size of graph excluding this node.
      let excluded = fetch_refs_size(env, id);
      let added = total.saturating_sub(excluded);

      let immediate = parents[id as usize]
         .iter()
         .copied()
         .collect::<Vec<NodeId>>();
      let chars = disamb[id as usize];

      PathStats {
         total_size:           own + transitive,
         added_size:           added,
         immediate_parents:    immediate,
         disambiguation_chars: chars,
      }
   })
}

fn fetch_refs_size(env: &StoreEnv<()>, exclude: NodeId) -> u64 {
   let collected = env.fetch_refs(|id| id != exclude, &env.roots);
   collected.iter().map(|sp| sp.size).sum()
}

/// Returns the shortest path from some root to `target`, in root-first order.
pub fn shortest_path_to(env: &StoreEnv<PathStats>, target: NodeId) -> Vec<NodeId> {
   let mut best = Option::<Vec<NodeId>>::None;
   for &root in &env.roots {
      if let Some(path) = bfs_path(env, root, target) {
         match &best {
            Some(b) if b.len() <= path.len() => {},
            _ => best = Some(path),
         }
      }
   }
   best.unwrap_or_default()
}

fn bfs_path<P>(env: &StoreEnv<P>, root: NodeId, target: NodeId) -> Option<Vec<NodeId>> {
   let n = env.paths.len();
   let mut prev = vec![NodeId::MAX; n];
   let mut visited = vec![false; n];
   let mut queue = VecDeque::<NodeId>::new();
   queue.push_back(root);
   visited[root as usize] = true;
   while let Some(id) = queue.pop_front() {
      if id == target {
         let mut path = vec![id];
         let mut cur = id;
         while prev[cur as usize] != NodeId::MAX {
            cur = prev[cur as usize];
            path.push(cur);
         }
         path.reverse();
         return Some(path);
      }
      for &r in &env.get(id).refs {
         if !visited[r as usize] {
            visited[r as usize] = true;
            prev[r as usize] = id;
            queue.push_back(r);
         }
      }
   }
   None
}

/// All paths from each root down to `target`. Returns a flat list of paths,
/// with an overall cap of 1,000,000 nodes per root subtree and at most 10,000
/// root subtrees combined.
pub fn why_depends(env: &StoreEnv<PathStats>, target: NodeId) -> Vec<Vec<NodeId>> {
   // First, compute the set of nodes that can transitively reach `target`.
   // We walk backwards via `immediate_parents`. Without this prefilter, the
   // DFS budget can be exhausted by huge irrelevant branches that never reach
   // `target`, hiding real results.
   let n = env.paths.len();
   let mut reaches_target = vec![false; n];
   {
      let mut queue = std::collections::VecDeque::<NodeId>::new();
      queue.push_back(target);
      reaches_target[target as usize] = true;
      while let Some(id) = queue.pop_front() {
         for &p in &env.get(id).payload.immediate_parents {
            if !reaches_target[p as usize] {
               reaches_target[p as usize] = true;
               queue.push_back(p);
            }
         }
      }
   }

   let mut out = Vec::<Vec<NodeId>>::new();
   let cap = 1_000_000;
   let mut subtree_count = 0;

   for &root in &env.roots {
      if subtree_count >= 10_000 {
         break;
      }
      if !reaches_target[root as usize] {
         continue;
      }
      let before = out.len();
      collect_paths(env, root, target, cap, &reaches_target, &mut out);
      if out.len() > before {
         subtree_count += 1;
      }
   }
   out
}

/// Iterative DFS from `start`, recording every root→`target` path into `out`.
/// Truncates after `cap` total nodes pushed onto the path.
fn collect_paths<P>(
   env: &StoreEnv<P>,
   start: NodeId,
   target: NodeId,
   cap: usize,
   reaches_target: &[bool],
   out: &mut Vec<Vec<NodeId>>,
) {
   let mut path = vec![start];
   // (node, next-child-index-to-try)
   let mut frames = vec![(start, 0)];
   let mut nodes_used = 1;

   while let Some(&(cur, idx)) = frames.last() {
      if cur == target {
         out.push(path.clone());
         path.pop();
         frames.pop();
         continue;
      }
      let refs = &env.get(cur).refs;

      // Find the next child that can reach `target`. Pruning here is what
      // keeps the cap accurate: dead branches don't consume budget.
      let next = (idx..refs.len()).find(|&i| reaches_target[refs[i] as usize]);
      let Some(next_idx) = next else {
         path.pop();
         frames.pop();
         continue;
      };

      let child = refs[next_idx];
      // Bump the parent frame so we resume after this child on backtrack.
      frames.last_mut().expect("frame just inspected").1 = next_idx + 1;

      if nodes_used >= cap {
         break;
      }
      nodes_used += 1;
      path.push(child);
      frames.push((child, 0));
   }
}

/// Build a one-line summary of immediate parents for the given node.
pub fn fmt_immediate_parents(env: &StoreEnv<PathStats>, id: NodeId) -> String {
   let parents = &env.get(id).payload.immediate_parents;
   if parents.is_empty() {
      return "Immediate Parents: -".to_string();
   }
   let mut names = parents
      .iter()
      .map(|&pid| env.get(pid).name.short())
      .collect::<Vec<&str>>();
   names.sort_by_key(|a| a.to_lowercase());
   format!(
      "Immediate Parents ({}): {}",
      parents.len(),
      names.join(", ")
   )
}
