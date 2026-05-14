#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeUnits {
   /// 1024-base, IEC suffixes (KiB, MiB, GiB, TiB). Default.
   Binary,
   /// 1000-base, SI suffixes (KB, MB, GB, TB).
   Si,
}

pub fn pretty_size(bytes: u64, units: SizeUnits) -> String {
   let (base, suffixes) = match units {
      SizeUnits::Binary => (1024_u64, ["KiB", "MiB", "GiB", "TiB"]),
      SizeUnits::Si => (1000_u64, ["KB", "MB", "GB", "TB"]),
   };
   let k = base;
   let m = k * base;
   let g = m * base;
   let t = g * base;

   let (value, unit) = if bytes < k {
      return format!("{bytes}");
   } else if bytes < m {
      (bytes as f64 / k as f64, suffixes[0])
   } else if bytes < g {
      (bytes as f64 / m as f64, suffixes[1])
   } else if bytes < t {
      (bytes as f64 / g as f64, suffixes[2])
   } else {
      (bytes as f64 / t as f64, suffixes[3])
   };

   let rounded = (value * 100.0).round() / 100.0;
   format!("{rounded} {unit}")
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn bytes() {
      assert_eq!(pretty_size(0, SizeUnits::Binary), "0");
      assert_eq!(pretty_size(1023, SizeUnits::Binary), "1023");
   }

   #[test]
   fn kib() {
      assert_eq!(pretty_size(1024, SizeUnits::Binary), "1 KiB");
      assert_eq!(pretty_size(1536, SizeUnits::Binary), "1.5 KiB");
   }

   #[test]
   fn mib() {
      assert_eq!(pretty_size(1024 * 1024, SizeUnits::Binary), "1 MiB");
   }

   #[test]
   fn si_kb() {
      assert_eq!(pretty_size(1000, SizeUnits::Si), "1 KB");
      assert_eq!(pretty_size(1500, SizeUnits::Si), "1.5 KB");
   }

   #[test]
   fn si_mb() {
      assert_eq!(pretty_size(1_000_000, SizeUnits::Si), "1 MB");
   }
}
