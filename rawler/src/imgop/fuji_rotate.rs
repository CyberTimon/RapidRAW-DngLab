// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::imgop::Dim2;
use crate::pixarray::Color2D;

/// Correct the 45-degree counter-clockwise layout of a rotated Fuji sensor.
///
/// `fuji_rotation_width` is the split point used to calculate the largest
/// rectangular image that fits inside the rotated sensor data.
pub(crate) fn fuji_normalize_rotation(src: &Color2D<f32, 3>, fuji_rotation_width: usize, extra_rotate: bool) -> Color2D<f32, 3> {
  let rotated;
  let src = if extra_rotate {
    rotated = src.rotate_90cw();
    &rotated
  } else {
    src
  };
  rotate_45cw(src, fuji_rotation_width)
}

pub(crate) fn fuji_calc_dimension(width: usize, fuji_rotation_width: usize) -> Dim2 {
  let split = fuji_rotation_width as f64;
  let width = width as f64;
  let first_side = (split * std::f64::consts::SQRT_2).floor() as usize;
  let second_side = ((width - split) * std::f64::consts::SQRT_2).floor() as usize;

  if first_side > second_side {
    Dim2::new(first_side, second_side)
  } else {
    Dim2::new(second_side, first_side)
  }
}

fn rotate_45cw(src: &Color2D<f32, 3>, fuji_rotation_width: usize) -> Color2D<f32, 3> {
  let src_width = src.width;
  let src_height = src.height;
  let src_center_x = src_width as f64 / 2.0;
  let src_center_y = src_height as f64 / 2.0;
  let Dim2 { w: dst_width, h: dst_height } = fuji_calc_dimension(src_width, fuji_rotation_width);
  let dst_center_x = dst_width as f64 / 2.0;
  let dst_center_y = dst_height as f64 / 2.0;
  let mut dst = Color2D::<f32, 3>::new(dst_width, dst_height);

  for row in 0..dst_height {
    for col in 0..dst_width {
      let dx = col as f64 - dst_center_x;
      let dy = row as f64 - dst_center_y;
      let src_x = (dx + dy) * std::f64::consts::FRAC_1_SQRT_2 + src_center_x;
      let src_y = (dy - dx) * std::f64::consts::FRAC_1_SQRT_2 + src_center_y;
      let nearest_x = src_x.round() as isize;
      let nearest_y = src_y.round() as isize;

      if nearest_x < 0 || nearest_x >= src_width as isize || nearest_y < 0 || nearest_y >= src_height as isize {
        continue;
      }

      let x0 = src_x.floor() as isize;
      let y0 = src_y.floor() as isize;
      let x1 = x0 + 1;
      let y1 = y0 + 1;
      let pixel = dst.at_mut(row, col);

      if x0 < 0 || y0 < 0 || x1 >= src_width as isize || y1 >= src_height as isize {
        *pixel = *src.at(nearest_y as usize, nearest_x as usize);
      } else {
        let fraction_x = (src_x - x0 as f64) as f32;
        let fraction_y = (src_y - y0 as f64) as f32;
        let p00 = src.at(y0 as usize, x0 as usize);
        let p10 = src.at(y0 as usize, x1 as usize);
        let p01 = src.at(y1 as usize, x0 as usize);
        let p11 = src.at(y1 as usize, x1 as usize);

        for channel in 0..3 {
          pixel[channel] = p00[channel] * (1.0 - fraction_x) * (1.0 - fraction_y)
            + p10[channel] * fraction_x * (1.0 - fraction_y)
            + p01[channel] * (1.0 - fraction_x) * fraction_y
            + p11[channel] * fraction_x * fraction_y;
        }
      }
    }
  }

  dst
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn s2_pro_dimensions_are_landscape() {
    assert_eq!(fuji_calc_dimension(3664, 2192), Dim2::new(3099, 2081));
    assert_eq!(fuji_calc_dimension(916, 548), Dim2::new(774, 520));
  }
}
