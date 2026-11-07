// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{collections::HashMap, convert::TryFrom};

use crate::imgop::matrix::{multiply_row1, pseudo_inverse, transform_1d};

/// Illuminants for XYZ
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Illuminant {
  Unknown = 0,
  Daylight = 1,
  Fluorescent = 2,
  Tungsten = 3,
  Flash = 4,
  FineWeather = 9,
  CloudyWeather = 10,
  Shade = 11,
  DaylightFluorescent = 12,
  DaylightWhiteFluorescent = 13,
  CoolWhiteFluorescent = 14,
  WhiteFluorescent = 15,
  A = 17,
  B = 18,
  C = 19,
  D55 = 20,
  D65 = 21,
  D75 = 22,
  D50 = 23,
  IsoStudioTungsten = 24,
}

pub type FlatColorMatrix = Vec<f32>;

// Robertson (1968) reciprocal-temperature lines in CIE 1960 UCS space.
// Data from Wyszecki & Stiles, Color Science, 2nd ed., p. 228.
const ROBERTSON_LINES: [(f32, f32, f32, f32); 31] = [
  (0.0, 0.18006, 0.26352, -0.24341),
  (10.0, 0.18066, 0.26589, -0.25479),
  (20.0, 0.18133, 0.26846, -0.26876),
  (30.0, 0.18208, 0.27119, -0.28539),
  (40.0, 0.18293, 0.27407, -0.30470),
  (50.0, 0.18388, 0.27709, -0.32675),
  (60.0, 0.18494, 0.28021, -0.35156),
  (70.0, 0.18611, 0.28342, -0.37915),
  (80.0, 0.18740, 0.28668, -0.40955),
  (90.0, 0.18880, 0.28997, -0.44278),
  (100.0, 0.19032, 0.29326, -0.47888),
  (125.0, 0.19462, 0.30141, -0.58204),
  (150.0, 0.19962, 0.30921, -0.70471),
  (175.0, 0.20525, 0.31647, -0.84901),
  (200.0, 0.21142, 0.32312, -1.0182),
  (225.0, 0.21807, 0.32909, -1.2168),
  (250.0, 0.22511, 0.33439, -1.4512),
  (275.0, 0.23247, 0.33904, -1.7298),
  (300.0, 0.24010, 0.34308, -2.0637),
  (325.0, 0.24702, 0.34655, -2.4681),
  (350.0, 0.25591, 0.34951, -2.9641),
  (375.0, 0.26400, 0.35200, -3.5814),
  (400.0, 0.27218, 0.35407, -4.3633),
  (425.0, 0.28039, 0.35577, -5.3762),
  (450.0, 0.28863, 0.35714, -6.7262),
  (475.0, 0.29685, 0.35823, -8.5955),
  (500.0, 0.30505, 0.35907, -11.324),
  (525.0, 0.31320, 0.35968, -15.628),
  (550.0, 0.32129, 0.36011, -23.325),
  (575.0, 0.32931, 0.36038, -40.770),
  (600.0, 0.33724, 0.36051, -116.45),
];

impl TryFrom<u16> for Illuminant {
  type Error = String;

  fn try_from(v: u16) -> Result<Self, Self::Error> {
    Ok(match v {
      0 => Self::Unknown,
      1 => Self::Daylight,
      2 => Self::Fluorescent,
      3 => Self::Tungsten,
      4 => Self::Flash,
      9 => Self::FineWeather,
      10 => Self::CloudyWeather,
      11 => Self::Shade,
      12 => Self::DaylightFluorescent,
      13 => Self::DaylightWhiteFluorescent,
      14 => Self::CoolWhiteFluorescent,
      15 => Self::WhiteFluorescent,
      17 => Self::A,
      18 => Self::B,
      19 => Self::C,
      20 => Self::D55,
      21 => Self::D65,
      22 => Self::D75,
      23 => Self::D50,
      24 => Self::IsoStudioTungsten,
      _ => {
        return Err(format!("Unknown illuminant value: {}", v));
      }
    })
  }
}

impl From<Illuminant> for u16 {
  fn from(value: Illuminant) -> Self {
    value as u16
  }
}

impl Illuminant {
  pub fn new_from_str(s: &str) -> Result<Self, String> {
    match s {
      "Unknown" => Ok(Self::Unknown),
      "Daylight" => Ok(Self::Daylight),
      "Fluorescent" => Ok(Self::Fluorescent),
      "Tungsten" => Ok(Self::Tungsten),
      "Flash" => Ok(Self::Flash),
      "FineWeather" => Ok(Self::FineWeather),
      "CloudyWeather" => Ok(Self::CloudyWeather),
      "Shade" => Ok(Self::Shade),
      "DaylightFluorescent" => Ok(Self::DaylightFluorescent),
      "DaylightWhiteFluorescent" => Ok(Self::DaylightWhiteFluorescent),
      "CoolWhiteFluorescent" => Ok(Self::CoolWhiteFluorescent),
      "WhiteFluorescent" => Ok(Self::WhiteFluorescent),
      "A" => Ok(Self::A),
      "B" => Ok(Self::B),
      "C" => Ok(Self::C),
      "D55" => Ok(Self::D55),
      "D65" => Ok(Self::D65),
      "D75" => Ok(Self::D75),
      "D50" => Ok(Self::D50),
      "IsoStudioTungsten" => Ok(Self::IsoStudioTungsten),
      _ => Err(format!("Unknown illuminant name: '{}'", s)),
    }
  }

  fn temperature(self) -> Option<f32> {
    match self {
      Self::A | Self::Tungsten => Some(2850.0),
      Self::IsoStudioTungsten => Some(3200.0),
      Self::D50 => Some(5000.0),
      Self::D55 | Self::Daylight | Self::FineWeather | Self::Flash | Self::B => Some(5500.0),
      Self::D65 | Self::C | Self::CloudyWeather => Some(6500.0),
      Self::D75 | Self::Shade => Some(7500.0),
      Self::DaylightFluorescent => Some(6400.0),
      Self::DaylightWhiteFluorescent => Some(5050.0),
      Self::Fluorescent | Self::CoolWhiteFluorescent => Some(4150.0),
      Self::WhiteFluorescent => Some(3525.0),
      Self::Unknown => None,
    }
  }
}

/// Estimate correlated color temperature from a CIE 1931 xy chromaticity.
fn xy_to_temperature(x: f32, y: f32) -> Option<f32> {
  let denominator = 1.5 - x + 6.0 * y;
  if !denominator.is_finite() || denominator <= 0.0 {
    return None;
  }

  let u = 2.0 * x / denominator;
  let v = 3.0 * y / denominator;
  let mut previous: Option<(f32, f32)> = None;

  for &(reciprocal_temperature, line_u, line_v, slope) in &ROBERTSON_LINES {
    let distance = (v - line_v - slope * (u - line_u)) / (1.0 + slope * slope).sqrt();
    if let Some((previous_reciprocal_temperature, previous_distance)) = previous
      && distance <= 0.0
    {
      let fraction = previous_distance / (previous_distance - distance);
      let reciprocal_temperature = previous_reciprocal_temperature + fraction * (reciprocal_temperature - previous_reciprocal_temperature);
      return (reciprocal_temperature > 0.0).then_some(1_000_000.0 / reciprocal_temperature);
    }
    previous = Some((reciprocal_temperature, distance));
  }

  None
}

/// Estimate the camera's as-shot color temperature from its white-balance
/// coefficients and color matrices. This follows the iterative approach used
/// by the DNG SDK: interpolate the color matrix in reciprocal-temperature
/// space, convert the camera neutral to xy, then update the temperature.
pub fn estimate_as_shot_temperature(wb: [f32; 4], color_matrices: &HashMap<Illuminant, FlatColorMatrix>) -> Option<u32> {
  if wb[..3].iter().any(|value| !value.is_finite() || *value <= 0.0) {
    return None;
  }

  let neutral = [1.0 / wb[0], 1.0 / wb[1], 1.0 / wb[2]];
  let mut matrices: Vec<(f32, [[f32; 3]; 3])> = color_matrices
    .iter()
    .filter_map(|(illuminant, matrix)| Some((illuminant.temperature()?, transform_1d::<3, 3>(matrix)?)))
    .collect();
  matrices.sort_by(|left, right| left.0.total_cmp(&right.0));
  let &(low_temperature, low_matrix) = matrices.first()?;
  let &(high_temperature, high_matrix) = matrices.last()?;

  let mut temperature = 5000.0_f32.clamp(low_temperature, high_temperature);
  for _ in 0..30 {
    let weight = if (high_temperature - low_temperature).abs() < f32::EPSILON {
      1.0
    } else if temperature <= low_temperature {
      1.0
    } else if temperature >= high_temperature {
      0.0
    } else {
      ((1.0 / temperature) - (1.0 / high_temperature)) / ((1.0 / low_temperature) - (1.0 / high_temperature))
    };

    let matrix = std::array::from_fn(|row| std::array::from_fn(|column| weight * low_matrix[row][column] + (1.0 - weight) * high_matrix[row][column]));
    let xyz = multiply_row1(&pseudo_inverse(matrix), &neutral);
    let sum = xyz.iter().sum::<f32>();
    if !sum.is_finite() || sum <= 0.0 {
      return None;
    }
    let next_temperature = xy_to_temperature(xyz[0] / sum, xyz[1] / sum)?;
    if !next_temperature.is_finite() || !(1500.0..=50_000.0).contains(&next_temperature) {
      return None;
    }
    if (next_temperature - temperature).abs() < 0.1 {
      return Some(next_temperature.round() as u32);
    }
    temperature = next_temperature;
  }

  Some(temperature.round() as u32)
}

// Constant matrix for converting sRGB to XYZ(D65):
// http://www.brucelindbloom.com/Eqn_RGB_XYZ_Matrix.html
#[allow(clippy::excessive_precision)]
pub const SRGB_TO_XYZ_D65: [[f32; 3]; 3] = [
  [0.4124564, 0.3575761, 0.1804375],
  [0.2126729, 0.7151522, 0.0721750],
  [0.0193339, 0.1191920, 0.9503041],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_ADOBERGB_D65: [[f32; 3]; 3] = [
  [2.0413690, -0.5649464, -0.3446944],
  [-0.9692660, 1.8760108, 0.0415560],
  [0.0134474, -0.1183897, 1.0154096],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_ADOBERGB_D50: [[f32; 3]; 3] = [
  [1.9624274, -0.6105343, -0.3413404],
  [-0.9787684, 1.9161415, 0.0334540],
  [0.0286869, -0.1406752, 1.3487655],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_SRGB_D50: [[f32; 3]; 3] = [
  [3.1338561, -1.6168667, -0.4906146],
  [-0.9787684, 1.9161415, 0.0334540],
  [0.0719453, -0.2289914, 1.4052427],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_SRGB_D65: [[f32; 3]; 3] = [
  [3.2404542, -1.5371385, -0.4985314],
  [-0.9692660, 1.8760108, 0.0415560],
  [0.0556434, -0.2040259, 1.0572252],
];

#[allow(clippy::excessive_precision)]
pub const XYZ_TO_PROFOTORGB_D50: [[f32; 3]; 3] = [
  [1.3459433, -0.2556075, -0.0511118],
  [-0.5445989, 1.5081673, 0.0205351],
  [0.0000000, 0.0000000, 1.2118128],
];

pub const CIE_1931_TRISTIMULUS_A: [f32; 3] = [1.09850, 1.00000, 0.35585]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_B: [f32; 3] = [0.99072, 1.00000, 0.85223]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_C: [f32; 3] = [0.98074, 1.00000, 1.18232]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D50: [f32; 3] = [0.96422, 1.00000, 0.82521]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D55: [f32; 3] = [0.95682, 1.00000, 0.92149]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D65: [f32; 3] = [0.95047, 1.00000, 1.08883]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_D75: [f32; 3] = [0.94972, 1.00000, 1.22638]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_E: [f32; 3] = [1.00000, 1.00000, 1.00000]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_F2: [f32; 3] = [0.99186, 1.00000, 0.67393]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_F7: [f32; 3] = [0.95041, 1.00000, 1.08747]; // X, Y, Z

pub const CIE_1931_TRISTIMULUS_F11: [f32; 3] = [1.00962, 1.00000, 0.64350]; // X, Y, Z

/// incandescent / tungsten
pub const CIE_1931_WHITE_POINT_A: (f32, f32) = (0.44757, 0.40745);
/// obsolete, direct sunlight at noon
pub const CIE_1931_WHITE_POINT_B: (f32, f32) = (0.34842, 0.35161);
/// obsolete, average / North sky daylight
pub const CIE_1931_WHITE_POINT_C: (f32, f32) = (0.31006, 0.31616);
/// horizon light, ICC profile PCS
pub const CIE_1931_WHITE_POINT_D50: (f32, f32) = (0.34567, 0.35850);
/// mid-morning / mid-afternoon daylight
pub const CIE_1931_WHITE_POINT_D55: (f32, f32) = (0.33242, 0.34743);
/// noon daylight: television, sRGB color space
pub const CIE_1931_WHITE_POINT_D65: (f32, f32) = (0.31271, 0.32902);
/// North sky daylight
pub const CIE_1931_WHITE_POINT_D75: (f32, f32) = (0.29902, 0.31485);
/// high-efficiency blue phosphor monitors, BT.2035
pub const CIE_1931_WHITE_POINT_D93: (f32, f32) = (0.28315, 0.29711);
/// equal energy
pub const CIE_1931_WHITE_POINT_E: (f32, f32) = (0.33333, 0.33333);

#[allow(non_snake_case)]
pub fn xyY_to_XYZ(x: f32, y: f32, Y: f32) -> [f32; 3] {
  if y.is_normal() && y.is_sign_positive() {
    [x * Y / y, Y, (1.0 - x - y) * Y / y]
  } else {
    panic!("xy_to_XYZ(): 'y' argument must be greater than zero");
  }
}

#[allow(non_snake_case)]
pub fn xy_to_XYZ(x: f32, y: f32) -> [f32; 3] {
  const Y: f32 = 1.0;
  xyY_to_XYZ(x, y, Y)
}

/// Convert a given xy whitepoint to white balance coefficents,
/// adapted to
pub fn xy_whitepoint_to_wb_coeff(x: f32, y: f32, colormatrix: &[[f32; 3]; 3]) -> [f32; 3] {
  let mut result = [0.0, 0.0, 0.0];
  if y > 0.0 {
    let as_shot_white = xy_to_XYZ(x, y);
    for i in 0..3 {
      let c = colormatrix[i][0] * as_shot_white[0] + colormatrix[i][1] * as_shot_white[1] + colormatrix[i][2] * as_shot_white[2];
      if c > 0.0 {
        result[i] = 1.0 / c;
      }
    }
  }
  result
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::matrix::transform_2d;

  #[test]
  fn estimates_d65_temperature_from_camera_neutral() {
    let matrix = [[0.8, 0.1, 0.0], [0.05, 0.9, 0.05], [0.0, 0.1, 0.7]];
    let matrices = HashMap::from([(Illuminant::D65, transform_2d(&matrix))]);
    let wb = xy_whitepoint_to_wb_coeff(CIE_1931_WHITE_POINT_D65.0, CIE_1931_WHITE_POINT_D65.1, &matrix);
    let wb = [wb[0], wb[1], wb[2], f32::NAN];

    let temperature = estimate_as_shot_temperature(wb, &matrices).unwrap();
    assert!((6400..=6600).contains(&temperature));
  }

  #[test]
  fn interpolates_dual_illuminant_matrices_in_mired_space() {
    let matrix_a = [[0.7, 0.2, 0.0], [0.1, 0.8, 0.1], [0.0, 0.1, 0.8]];
    let matrix_d65 = [[0.8, 0.1, 0.0], [0.05, 0.9, 0.05], [0.0, 0.1, 0.7]];
    let target_temperature = 5000.0;
    let weight = ((1.0 / target_temperature) - (1.0 / 6500.0)) / ((1.0 / 2850.0) - (1.0 / 6500.0));
    let interpolated = std::array::from_fn(|row| std::array::from_fn(|column| weight * matrix_a[row][column] + (1.0 - weight) * matrix_d65[row][column]));
    let matrices = HashMap::from([(Illuminant::A, transform_2d(&matrix_a)), (Illuminant::D65, transform_2d(&matrix_d65))]);
    let wb = xy_whitepoint_to_wb_coeff(CIE_1931_WHITE_POINT_D50.0, CIE_1931_WHITE_POINT_D50.1, &interpolated);
    let wb = [wb[0], wb[1], wb[2], f32::NAN];

    let temperature = estimate_as_shot_temperature(wb, &matrices).unwrap();
    assert!((4950..=5050).contains(&temperature));
  }

  #[test]
  fn estimates_nikon_zf_as_shot_temperature() {
    // Nikon Z f sample with WB_RBLevels 1.4453125/1.689453125 and
    // ColorTemperatureAuto 3890 K.
    let matrices = HashMap::from([
      (Illuminant::A, vec![1.3904, -0.7947, 0.0654, -0.432, 1.2105, 0.2497, -0.0235, 0.083, 0.9243]),
      (Illuminant::D65, vec![1.1607, -0.4491, -0.0977, -0.4522, 1.246, 0.2304, -0.0458, 0.1519, 0.7616]),
    ]);
    let wb = [1.4453125, 1.0, 1.689453125, f32::NAN];

    let temperature = estimate_as_shot_temperature(wb, &matrices).unwrap();
    assert!((3800..=4000).contains(&temperature));
  }

  #[test]
  fn rejects_invalid_white_balance_coefficients() {
    let matrices = HashMap::from([(Illuminant::D65, vec![1.0; 9])]);
    assert_eq!(estimate_as_shot_temperature([f32::NAN, 1.0, 1.0, f32::NAN], &matrices), None);
  }
}
