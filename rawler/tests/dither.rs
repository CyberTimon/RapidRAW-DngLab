//! LookupTable::dither must never wrap past u16::MAX.
//!
//! DxO PureRAW 6 writes DNG 1.7 files with a 1024-entry LinearizationTable
//! whose last two entries are 65407 and 65535. After new_with_bits() pads the
//! table to 16 bits, entry 1023 has base 65503 and delta 128, so the dithered
//! value reaches 65567; cast to u16 without saturation that wrapped roughly
//! half of all fully clipped pixels to 0..31.

use rawler::bits::LookupTable;

fn pureraw6_like_table() -> Vec<u16> {
  // Linear ramp of 1024 entries ending exactly like the DxO table.
  (0..1024u32).map(|i| ((i * 65535 + 511) / 1023) as u16).collect()
}

#[test]
fn dither_saturates_at_last_entry() {
  let points = pureraw6_like_table();
  assert_eq!(points[1022], 65471);
  assert_eq!(points[1023], 65535);

  let table = LookupTable::new_with_bits(&points, 16);
  let mut random = 0x1234_5678u32;
  let mut min = u16::MAX;
  for _ in 0..100_000 {
    min = min.min(table.dither(1023, &mut random));
  }
  assert!(min >= 65_000, "clipped pixel dithered down to {min}: dither wrapped past u16::MAX");
}

#[test]
fn dither_unchanged_below_last_entry() {
  let points = pureraw6_like_table();
  let table = LookupTable::new_with_bits(&points, 16);
  let mut random = 0x1234_5678u32;
  for code in 0..1023u16 {
    let center = points[code as usize] as i32;
    for _ in 0..16 {
      let v = table.dither(code, &mut random) as i32;
      // Dither spreads by at most a quarter step either side of the center.
      assert!((v - center).abs() <= 33, "code {code}: {v} vs center {center}");
    }
  }
  // Padded entries above the table map exactly to the last value.
  assert_eq!(table.dither(65535, &mut random), 65535);
}
