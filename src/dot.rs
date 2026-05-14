use std::{
   collections::BTreeSet,
   fmt::Write as _,
};

pub fn render(edges: &BTreeSet<(String, String)>) -> String {
   let mut out = String::from("strict digraph {\n");
   for (from, to) in edges {
      let _ = writeln!(out, "  {} -> {}", quote(from), quote(to));
   }
   out.push('}');
   out
}

fn quote(s: &str) -> String {
   if needs_quoting(s) {
      let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
      format!("\"{escaped}\"")
   } else {
      s.to_string()
   }
}

fn needs_quoting(s: &str) -> bool {
   if s.is_empty() {
      return true;
   }
   let mut chars = s.chars();
   let first = chars.next().unwrap();
   if !(first.is_ascii_alphabetic() || first == '_') {
      return true;
   }
   if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
      return true;
   }
   matches!(
      s.to_ascii_lowercase().as_str(),
      "node" | "edge" | "graph" | "digraph" | "subgraph" | "strict"
   )
}

#[cfg(test)]
mod keyword_tests {
   use super::*;

   #[test]
   fn quotes_dot_keywords() {
      let mut edges = std::collections::BTreeSet::new();
      edges.insert(("node".to_string(), "child".to_string()));
      let out = render(&edges);
      assert!(out.contains("\"node\" -> child"), "output was: {out}");
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn renders_empty() {
      let edges = BTreeSet::new();
      assert_eq!(render(&edges), "strict digraph {\n}");
   }

   #[test]
   fn quotes_special_chars() {
      let mut edges = BTreeSet::new();
      edges.insert(("a-b".to_string(), "c.d".to_string()));
      let out = render(&edges);
      assert!(out.contains("\"a-b\" -> \"c.d\""));
   }
}
