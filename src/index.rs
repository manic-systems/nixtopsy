use std::collections::{
   BTreeMap,
   HashMap,
   HashSet,
};

pub struct InvertedIndex<A> {
   elems:    BTreeMap<String, A>,
   unigrams: HashMap<char, HashSet<String>>,
   bigrams:  HashMap<(char, char), HashSet<String>>,
   trigrams: HashMap<(char, char, char), HashSet<String>>,
}

impl<A: Clone> InvertedIndex<A> {
   pub fn new() -> Self {
      Self {
         elems:    BTreeMap::new(),
         unigrams: HashMap::new(),
         bigrams:  HashMap::new(),
         trigrams: HashMap::new(),
      }
   }

   pub fn insert(&mut self, key: String, value: A) {
      let lower = key
         .chars()
         .flat_map(char::to_lowercase)
         .collect::<Vec<char>>();

      for c in &lower {
         self.unigrams.entry(*c).or_default().insert(key.clone());
      }
      for w in lower.windows(2) {
         self
            .bigrams
            .entry((w[0], w[1]))
            .or_default()
            .insert(key.clone());
      }
      for w in lower.windows(3) {
         self
            .trigrams
            .entry((w[0], w[1], w[2]))
            .or_default()
            .insert(key.clone());
      }
      self.elems.insert(key, value);
   }

   pub fn search(&self, needle: &str) -> BTreeMap<String, A> {
      let lower = needle
         .chars()
         .flat_map(char::to_lowercase)
         .collect::<Vec<char>>();
      let lower_str = lower.iter().collect::<String>();

      let candidates = match lower.len() {
         0 => return self.elems.clone(),
         1 => self.unigrams.get(&lower[0]).cloned(),
         2 => self.bigrams.get(&(lower[0], lower[1])).cloned(),
         _ => {
            let mut sets = Vec::<&HashSet<String>>::new();
            for w in lower.windows(3) {
               match self.trigrams.get(&(w[0], w[1], w[2])) {
                  Some(s) => sets.push(s),
                  None => return BTreeMap::new(),
               }
            }
            if sets.is_empty() {
               None
            } else {
               let mut iter = sets.into_iter();
               let first = iter.next().unwrap().clone();
               Some(iter.fold(first, |acc, s| acc.intersection(s).cloned().collect()))
            }
         },
      };

      let Some(candidates) = candidates else {
         return BTreeMap::new();
      };

      let mut out = BTreeMap::new();
      for key in candidates {
         if key.to_lowercase().contains(&lower_str)
            && let Some(v) = self.elems.get(&key)
         {
            out.insert(key, v.clone());
         }
      }
      out
   }
}

impl<A: Clone> Default for InvertedIndex<A> {
   fn default() -> Self {
      Self::new()
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   fn naive_search(haystack: &BTreeMap<String, i32>, needle: &str) -> BTreeMap<String, i32> {
      let needle_l = needle.to_lowercase();
      haystack
         .iter()
         .filter(|(k, _)| k.to_lowercase().contains(&needle_l))
         .map(|(k, v)| (k.clone(), *v))
         .collect()
   }

   fn check(haystack: BTreeMap<String, i32>, needle: &str) {
      let mut idx = InvertedIndex::new();
      for (k, v) in &haystack {
         idx.insert(k.clone(), *v);
      }
      let expected = naive_search(&haystack, needle);
      let actual = idx.search(needle);
      assert_eq!(actual, expected, "needle={needle:?} haystack={haystack:?}");
   }

   fn map(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
      pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
   }

   #[test]
   fn empty_haystack() {
      check(BTreeMap::new(), "abc");
   }

   #[test]
   fn empty_needle_returns_all() {
      check(map(&[("foo", 1), ("bar", 2)]), "");
   }

   #[test]
   fn single_char_needle() {
      check(map(&[("foo", 1), ("bar", 2), ("baz", 3)]), "f");
      check(map(&[("foo", 1), ("bar", 2), ("baz", 3)]), "a");
      check(map(&[("foo", 1), ("bar", 2), ("baz", 3)]), "z");
   }

   #[test]
   fn case_insensitive() {
      check(map(&[("Foo", 1), ("BAR", 2)]), "F");
      check(map(&[("Foo", 1), ("BAR", 2)]), "f");
      check(map(&[("Foo", 1), ("BAR", 2)]), "bar");
      check(map(&[("Foo", 1), ("BAR", 2)]), "BaR");
   }

   #[test]
   fn substring_match() {
      check(map(&[("alphabet", 1), ("beta", 2), ("gamma", 3)]), "ab");
      check(map(&[("alphabet", 1), ("beta", 2), ("gamma", 3)]), "et");
   }

   #[test]
   fn long_needle() {
      check(
         map(&[("hello world", 1), ("hellow", 2), ("hell", 3)]),
         "hellow",
      );
   }

   #[test]
   fn no_match() {
      check(map(&[("foo", 1), ("bar", 2)]), "xyz");
   }
}
