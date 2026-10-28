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
   let mut unit = base;
   let mut suffix = suffixes[0];
   if bytes < unit {
      return format!("{bytes}");
   }
   for &next in &suffixes[1..] {
      if bytes < unit * base {
         break;
      }
      unit *= base;
      suffix = next;
   }

   let hundredths = (u128::from(bytes) * 100 + u128::from(unit) / 2) / u128::from(unit);
   let (whole, frac) = (hundredths / 100, hundredths % 100);
   if frac == 0 {
      format!("{whole} {suffix}")
   } else if frac % 10 == 0 {
      format!("{whole}.{} {suffix}", frac / 10)
   } else {
      format!("{whole}.{frac:02} {suffix}")
   }
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
