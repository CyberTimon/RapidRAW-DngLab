use crate::RawImage;
use crate::cfa::*;
use crate::decoders::*;
use crate::formats::tiff::Entry;
use crate::formats::tiff::Rational;
use crate::formats::tiff::Value;
use crate::imgop::Dim2;
use crate::imgop::Point;
use crate::imgop::Rect;
use crate::imgop::matrix::*;
use crate::imgop::xyz::*;
use crate::tags::DngTag;
use crate::tags::TiffCommonTag;

#[derive(Debug, Clone)]
pub struct DngDecoder<'a> {
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
}

impl<'a> DngDecoder<'a> {
  pub fn new(_file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DngDecoder<'a>> {
    Ok(DngDecoder { tiff, rawloader })
  }
}

/// DNG format encapsulation for analyzer
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DngFormat {
  tiff: GenericTiffReader,
}

impl<'a> Decoder for DngDecoder<'a> {
  fn raw_image(&self, file: &RawSource, params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let full_raw = self.get_raw_ifd()?;
    let proxy = params.proxy_min_dim.and_then(|min_dim| self.get_proxy_ifd(min_dim));
    let raw = proxy.unwrap_or(full_raw);
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    let bits = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_u32(0);
    let orientation = Orientation::from_tiff(self.tiff.root_ifd());

    let mut cam = self.make_camera(raw, width, height)?;
    if proxy.is_some() && cam.crop_area.is_none() {
      cam.crop_area = self.get_scaled_full_crop(full_raw, width, height);
    }
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self
      .rawloader
      .check_supported_with_mode(self.tiff.root_ifd(), "dng")
      .or_else(|_| self.rawloader.check_supported(self.tiff.root_ifd()))
    {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
      cam.hints.extend_from_slice(&known_cam.hints);
      cam.params.extend(known_cam.params.iter().map(|(k, v)| (k.clone(), v.clone())));
    } else {
      log::debug!("DNG: camera {} / {} is not in the camera catalog", cam.make, cam.model);
      // panic!("Camera {} / {} is not in the camera catalog", cam.make, cam.model);
    }

    let blacklevel = self.get_blacklevels(raw)?;
    let whitelevel = self.get_whitelevels(raw)?.or(Some(WhiteLevel::new_bits(bits, cpp)));

    let photometric = match fetch_tiff_tag!(raw, TiffCommonTag::PhotometricInt).force_u32(0) {
      1 => RawPhotometricInterpretation::BlackIsZero,
      32803 => RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&cam)),
      34892 => RawPhotometricInterpretation::LinearRaw,
      _ => todo!(),
    };

    let raw_data = if dummy {
      RawImageData::Integer(Vec::new())
    } else {
      let mut data = plain_image_from_ifd(raw, file)?;
      if proxy.is_some() {
        apply_map_polynomial_opcodes(raw, &mut data, width, cpp, bits);
      }
      data
    };
    let wb_coeffs = self.get_wb(&cam)?;
    let mut image = RawImage::new_with_data(cam, raw_data, width * cpp, height, cpp, wb_coeffs, photometric, blacklevel, whitelevel, dummy);
    image.orientation = orientation;
    Ok(image)
  }

  fn format_dump(&self) -> FormatDump {
    FormatDump::Dng(DngFormat { tiff: self.tiff.clone() })
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let raw = self.get_raw_ifd()?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let mut cam = self.make_camera(raw, width, height)?;
    // If we know the camera, re-use the clean names
    if let Ok(known_cam) = self.rawloader.check_supported(self.tiff.root_ifd()) {
      cam.clean_make = known_cam.clean_make;
      cam.clean_model = known_cam.clean_model;
    }
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&cam, exif);
    Ok(mdata)
  }

  fn thumbnail_image(&self, file: &RawSource, _params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if let Some(thumb_ifd) = Some(self.tiff.root_ifd()).filter(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u16(0)) == Some(1)) {
      Ok(Some(dynamic_image_from_ifd(thumb_ifd, file)?))
    } else {
      Ok(None)
    }
  }

  fn full_image(&self, file: &RawSource, params: &RawDecodeParams) -> Result<Option<DynamicImage>> {
    if params.image_index != 0 {
      return Ok(None);
    }
    if let Some(sub_ifds) = self.tiff.root_ifd().get_sub_ifd_all(TiffCommonTag::SubIFDs) {
      let first_ifd = sub_ifds
        .iter()
        .find(|ifd| ifd.get_entry(TiffCommonTag::NewSubFileType).map(|entry| entry.force_u32(0)) == Some(1));
      if let Some(preview_ifd) = first_ifd {
        return Ok(Some(dynamic_image_from_ifd(preview_ifd, file)?));
      }
    }
    Ok(None)
  }

  fn ifd(&self, wk_ifd: WellKnownIFD) -> Result<Option<Rc<IFD>>> {
    Ok(match wk_ifd {
      WellKnownIFD::Root => Some(Rc::new(self.tiff.root_ifd().clone())),
      WellKnownIFD::Raw => Some(Rc::new(self.get_raw_ifd()?.clone())),
      WellKnownIFD::Exif => self
        .tiff
        .root_ifd()
        .get_sub_ifd_all(ExifTag::ExifOffset)
        .and_then(|list| list.get(0))
        .cloned()
        .map(Rc::new),
      WellKnownIFD::ExifGps => self
        .tiff
        .root_ifd()
        .get_sub_ifd_all(ExifTag::GPSInfo)
        .and_then(|list| list.get(0))
        .cloned()
        .map(Rc::new),
      WellKnownIFD::VirtualDngRawTags => {
        let mut ifd = IFD::default();
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::OpcodeList1);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::OpcodeList2);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::OpcodeList3);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::NoiseProfile);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::BayerGreenSplit);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ChromaBlurRadius);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::AntiAliasStrength);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::NoiseReductionApplied);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ProfileGainTableMap);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::CameraCalibration1);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::CameraCalibration2);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::CameraCalibration3);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ForwardMatrix1);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ForwardMatrix2);
        IFD::copy_tag(&mut ifd, self.get_raw_ifd()?, DngTag::ForwardMatrix3);
        Some(Rc::new(ifd))
      }
      WellKnownIFD::VirtualDngRootTags => {
        let mut ifd = IFD::default();
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileEmbedPolicy);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData1);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData2);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData3);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapDims);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData1);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData2);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapData3);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileHueSatMapEncoding);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileLookTableData);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileLookTableDims);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileLookTableEncoding);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileName);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileCopyright);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::ProfileToneCurve);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DNGPrivateData);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::MakerNoteSafety);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AnalogBalance);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineExposure);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineNoise);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineSharpness);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::LinearResponseLimit);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::CameraSerialNumber);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AsShotICCProfile);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AsShotPreProfileMatrix);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::CurrentICCProfile);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::CurrentPreProfileMatrix);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::AsShotProfileName);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DefaultBlackRender);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::BaselineExposureOffset);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthFormat);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthNear);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthFar);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthUnits);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::DepthMeasureType);
        IFD::copy_tag(&mut ifd, self.tiff.root_ifd(), DngTag::RGBTables);
        Some(Rc::new(ifd))
      }
      _ => return Ok(None),
    })
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::DNG
  }
}

impl<'a> DngDecoder<'a> {
  fn get_raw_ifd(&self) -> Result<&IFD> {
    let ifds = self
      .tiff
      .find_ifds_with_tag(TiffCommonTag::Compression)
      .into_iter()
      .filter(|ifd| {
        let compression = (**ifd)
          .get_entry(TiffCommonTag::Compression)
          .expect("This IFD must contains this tag")
          .force_u32(0);
        let subsampled = match (**ifd).get_entry(TiffCommonTag::NewSubFileType) {
          Some(e) => e.force_u32(0) & 1 != 0,
          None => false,
        };
        !subsampled && (compression == 7 || compression == 8 || compression == 1 || compression == 0x884c || compression == 52546)
      })
      .collect::<Vec<&IFD>>();
    if let Some(first) = ifds.first() {
      Ok(first)
    } else {
      Err(RawlerError::DecoderFailed(format!("TODO: Unsupported DNG compression")))
    }
  }

  fn get_proxy_ifd(&self, min_dim: usize) -> Option<&IFD> {
    self
      .tiff
      .find_ifds_with_tag(TiffCommonTag::Compression)
      .into_iter()
      .filter(|ifd| {
        let reduced = ifd.get_entry(TiffCommonTag::NewSubFileType).map(|e| e.force_u32(0)) == Some(1);
        let linear = ifd.get_entry(TiffCommonTag::PhotometricInt).map(|e| e.force_u32(0)) == Some(34892);
        let compression = ifd.get_entry(TiffCommonTag::Compression).map(|e| e.force_u32(0)).unwrap_or(0);
        reduced && linear && matches!(compression, 1 | 7 | 8 | 52546)
      })
      .filter_map(|ifd| {
        let w = ifd.get_entry(TiffCommonTag::ImageWidth)?.force_usize(0);
        let h = ifd.get_entry(TiffCommonTag::ImageLength)?.force_usize(0);
        Some((w.max(h), ifd))
      })
      .filter(|(max_dim, _)| *max_dim >= min_dim)
      .min_by_key(|(max_dim, _)| *max_dim)
      .map(|(_, ifd)| ifd)
  }

  fn get_scaled_full_crop(&self, full_raw: &IFD, width: usize, height: usize) -> Option<[usize; 4]> {
    let full_w = full_raw.get_entry(TiffCommonTag::ImageWidth)?.force_usize(0);
    let full_h = full_raw.get_entry(TiffCommonTag::ImageLength)?.force_usize(0);
    let crop = self.get_crop(full_raw)?;
    let (x, y) = match self.get_active_area_borders(full_raw) {
      Some(area) => (crop.p.x + area[1], crop.p.y + area[0]),
      None => (crop.p.x, crop.p.y),
    };
    if full_w == 0 || full_h == 0 || (x == 0 && y == 0 && crop.d.w >= full_w && crop.d.h >= full_h) {
      return None;
    }
    let sx = width as f64 / full_w as f64;
    let sy = height as f64 / full_h as f64;
    let left = ((x as f64 * sx).round() as usize).min(width);
    let top = ((y as f64 * sy).round() as usize).min(height);
    let right = (((x + crop.d.w) as f64 * sx).round() as usize).clamp(left, width);
    let bottom = (((y + crop.d.h) as f64 * sy).round() as usize).clamp(top, height);
    Some([left, top, width - right, height - bottom])
  }

  fn make_camera(&self, raw: &IFD, width: usize, height: usize) -> Result<Camera> {
    let mode = String::from("dng");

    let make = self
      .tiff
      .root_ifd()
      .get_entry(TiffCommonTag::Make)
      .and_then(|x| x.as_string())
      .cloned()
      .unwrap_or_default();
    let model = self
      .tiff
      .root_ifd()
      .get_entry(TiffCommonTag::Model)
      .and_then(|x| x.as_string())
      .cloned()
      .unwrap_or_default();

    let active_area = self.get_active_area(raw, width, height);
    let crop_area = if let Some(crops) = self.get_crop(raw) {
      if let Some(active_area) = &active_area {
        let mut full = crops;
        full.p.x += active_area[0]; // left
        full.p.y += active_area[1]; // Top
        Some(full.as_ltrb_offsets(width, height))
      } else {
        Some(crops.as_ltrb_offsets(width, height))
      }
    } else {
      None
    };

    let linear = fetch_tiff_tag!(raw, TiffCommonTag::PhotometricInt).force_usize(0) == 34892;
    let cfa = if linear { CFA::default() } else { self.get_cfa(raw)? };
    let color_matrix = self.get_color_matrix()?;
    let real_bps = if raw.has_entry(TiffCommonTag::Linearization) {
      // If DNG contains linearization table, output is always 16 bits
      16
    } else {
      raw.get_entry(TiffCommonTag::BitsPerSample).map(|v| v.force_usize(0)).unwrap_or(16)
    };

    Ok(Camera {
      clean_make: make.clone(),
      clean_model: model.clone(),
      make,
      model,
      mode,
      whitelevel: None,
      blacklevel: None,
      blackareah: None,
      blackareav: None,
      xyz_to_cam: Default::default(),
      color_matrix,
      cfa,
      active_area,
      crop_area,
      real_bps,
      ..Default::default()
    })
  }

  fn get_wb(&self, cam: &Camera) -> Result<[f32; 4]> {
    if let Some(levels) = self.tiff.get_entry(DngTag::AsShotNeutral) {
      Ok([1.0 / levels.force_f32(0), 1.0 / levels.force_f32(1), 1.0 / levels.force_f32(2), f32::NAN])
    } else if let Some(levels) = self.tiff.get_entry(DngTag::AsShotWhiteXY) {
      // TODO: improve once AnalogBalance and CC is properly implemented
      if let Some(flat_colormatrix) = cam.color_matrix.get(&Illuminant::D65)
        && let Some(colormatrix) = transform_1d::<3, 3>(flat_colormatrix)
      {
        let wb_coeff = xy_whitepoint_to_wb_coeff(levels.force_f32(0), levels.force_f32(1), &colormatrix);
        Ok([wb_coeff[0], wb_coeff[1], wb_coeff[2], f32::NAN])
      } else {
        Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
      }
    } else {
      Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
    }
  }

  fn get_blacklevels(&self, raw: &IFD) -> Result<Option<BlackLevel>> {
    let cpp = raw.get_entry(TiffCommonTag::SamplesPerPixel).map(|entry| entry.force_usize(0)).unwrap_or(1);
    if let Some(entry) = raw.get_entry(TiffCommonTag::BlackLevels) {
      let levels = match &entry.value {
        Value::Short(black) => black.iter().copied().map(Rational::from).collect(),
        Value::Long(black) => black.iter().copied().map(Rational::from).collect(),
        Value::Rational(black) => black.clone(),
        _ => return Err(format!("Unsupported BlackLevel type: {}", entry.value_type_name()).into()),
      };
      let mut repeat = (1, 1);
      if let Some(Entry {
        value: Value::Short(value), ..
      }) = raw.get_entry(DngTag::BlackLevelRepeatDim)
      {
        if value.len() == 2 {
          repeat = (value[0] as usize, value[1] as usize);
        } else {
          // Pentax K-3 Mark III Monochrome is known to has invalid tag
          log::warn!("File has BlackLevelRepeatDim tag but with invalid length: {}", value.len());
        }
      }
      Ok(Some(BlackLevel::new(&levels, repeat.1, repeat.0, cpp)))
    } else {
      Ok(None)
    }
  }

  fn get_whitelevels(&self, raw: &IFD) -> Result<Option<WhiteLevel>> {
    let cpp = fetch_tiff_tag!(raw, TiffCommonTag::SamplesPerPixel).force_usize(0);
    if let Some(levels) = raw.get_entry(TiffCommonTag::WhiteLevel) {
      let mut whitelevels = WhiteLevel((0..levels.count()).map(|i| levels.force_u32(i as usize)).collect());
      // Fixes a bug where only a single whitelevel value is given.
      if whitelevels.0.len() == 1 && cpp > 1 {
        whitelevels.0 = vec![whitelevels.0[0]; cpp];
      }
      return Ok(Some(whitelevels));
    }
    Ok(None)
  }

  fn get_cfa(&self, raw: &IFD) -> Result<CFA> {
    let pattern = fetch_tiff_tag!(raw, TiffCommonTag::CFAPattern);
    let cfa = CFA::new_from_tag(pattern);
    // If DNG has active area, we need to calulate back the CFA pattern,
    // because for DNG the CFAPattern is relative to ActiveArea and we
    // use (0, 0) as starting point.
    if let Some(active_area) = self.get_active_area_borders(raw) {
      let top = active_area[0];
      let left = active_area[1];
      Ok(cfa.shift(left % cfa.width, top % cfa.height))
    } else {
      Ok(cfa)
    }
  }

  fn get_active_area_borders(&self, raw: &IFD) -> Option<[usize; 4]> {
    if let Some(crops) = raw.get_entry(DngTag::ActiveArea) {
      let rect = [crops.force_usize(0), crops.force_usize(1), crops.force_usize(2), crops.force_usize(3)];
      Some(rect)
    } else {
      // Ignore missing crops, at least some pentax DNGs don't have it
      None
    }
  }

  fn get_active_area(&self, raw: &IFD, width: usize, height: usize) -> Option<[usize; 4]> {
    if let Some(rect) = self.get_active_area_borders(raw) {
      Some(Rect::new_with_dng(&rect).as_ltrb_offsets(width, height))
    } else {
      None
    }
  }

  fn get_crop(&self, raw: &IFD) -> Option<Rect> {
    if let Some(crops) = raw.get_entry(DngTag::DefaultCropOrigin) {
      let p = Point::new(crops.force_usize(0), crops.force_usize(1));
      if let Some(size) = raw.get_entry(DngTag::DefaultCropSize) {
        let d = Dim2::new(size.force_usize(0), size.force_usize(1));
        return Some(Rect::new(p, d));
      }
    }
    None
  }

  fn _get_masked_areas(&self, raw: &IFD) -> Vec<Rect> {
    let mut areas = Vec::new();

    if let Some(masked_area) = raw.get_entry(TiffCommonTag::MaskedAreas) {
      for x in (0..masked_area.count() as usize).step_by(4) {
        areas.push(Rect::new_with_points(
          Point::new(masked_area.force_usize(x), masked_area.force_usize(x + 1)),
          Point::new(masked_area.force_usize(x + 2), masked_area.force_usize(x + 3)),
        ));
      }
    }

    areas
  }

  fn get_color_matrix(&self) -> Result<HashMap<Illuminant, FlatColorMatrix>> {
    let mut result = HashMap::new();

    let mut read_matrix = |cal: DngTag, mat: DngTag| -> Result<()> {
      if let Some(c) = self.tiff.get_entry(mat) {
        let illuminant_val = self.tiff.get_entry(cal).map(|e| e.force_u16(0)).unwrap_or(21);
        let illuminant: Illuminant = illuminant_val.try_into().unwrap_or(Illuminant::D65);

        let mut matrix = FlatColorMatrix::new();
        for i in 0..c.count() as usize {
          matrix.push(c.force_f32(i));
        }

        if !matrix.is_empty() && matrix.len() <= 12 {
          result.insert(illuminant, matrix);
        } else {
          log::warn!("Invalid ColorMatrix dimensions for illuminant {:?}: length {}", illuminant, matrix.len());
        }
      }
      Ok(())
    };

    let _ = read_matrix(DngTag::CalibrationIlluminant1, DngTag::ColorMatrix1);
    let _ = read_matrix(DngTag::CalibrationIlluminant2, DngTag::ColorMatrix2);
    // TODO: add 3

    Ok(result)
  }
}

fn apply_map_polynomial_opcodes(ifd: &IFD, data: &mut RawImageData, width: usize, cpp: usize, bits: u32) {
  if let Some(entry) = ifd.get_entry(DngTag::OpcodeList2) {
    apply_map_polynomial(entry.value.get_data(), data, width, cpp, bits);
  }
}

fn apply_map_polynomial(buf: &[u8], data: &mut RawImageData, width: usize, cpp: usize, bits: u32) {
  let u32_at = |o: usize| buf.get(o..o + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
  let f64_at = |o: usize| buf.get(o..o + 8).map(|b| f64::from_be_bytes(b.try_into().unwrap()));

  let Some(count) = u32_at(0) else {
    return;
  };
  let mut offset = 4;
  for _ in 0..count {
    let (Some(id), Some(size)) = (u32_at(offset), u32_at(offset + 12)) else {
      return;
    };
    let p = offset + 16;
    offset = p.saturating_add(size as usize);
    if id != 8 {
      log::debug!("DNG proxy: ignoring OpcodeList2 opcode {}", id);
      continue;
    }
    let params: Option<Vec<u32>> = (0..9).map(|i| u32_at(p + i * 4)).collect();
    let Some([top, left, bottom, right, plane, planes, row_pitch, col_pitch, degree]) = params.and_then(|v| <[u32; 9]>::try_from(v).ok()) else {
      return;
    };
    let Some(coeffs) = (0..=degree as usize).map(|i| f64_at(p + 36 + i * 8)).collect::<Option<Vec<f64>>>() else {
      return;
    };
    let eval = |x: f64| coeffs.iter().rev().fold(0.0, |acc, c| acc * x + c);
    let height = match data {
      RawImageData::Integer(v) => v.len(),
      RawImageData::Float(v) => v.len(),
    } / (width * cpp).max(1);
    let (row_pitch, col_pitch) = (row_pitch.max(1) as usize, col_pitch.max(1) as usize);
    let planes = plane as usize..(plane.saturating_add(planes) as usize).min(cpp);
    let int_max = if (1..=16).contains(&bits) { ((1u32 << bits) - 1) as f64 } else { u16::MAX as f64 };
    for row in (top as usize..(bottom as usize).min(height)).step_by(row_pitch) {
      for col in (left as usize..(right as usize).min(width)).step_by(col_pitch) {
        for c in planes.clone() {
          let idx = (row * width + col) * cpp + c;
          match data {
            RawImageData::Integer(v) => {
              let x = v[idx] as f64 / int_max;
              v[idx] = (eval(x) * int_max).round().clamp(0.0, int_max) as u16;
            }
            RawImageData::Float(v) => v[idx] = eval(v[idx] as f64) as f32,
          }
        }
      }
    }
  }
}

#[cfg(test)]
mod map_polynomial_tests {
  use super::*;

  fn opcode(id: u32, params: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [id, 0x0103_0000, 0, params.len() as u32] {
      v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(params);
    v
  }

  fn map_poly(area: [u32; 4], plane: u32, planes: u32, pitch: (u32, u32), coeffs: &[f64]) -> Vec<u8> {
    let mut p = Vec::new();
    for x in [area[0], area[1], area[2], area[3], plane, planes, pitch.0, pitch.1, coeffs.len() as u32 - 1] {
      p.extend_from_slice(&x.to_be_bytes());
    }
    for c in coeffs {
      p.extend_from_slice(&c.to_be_bytes());
    }
    opcode(8, &p)
  }

  fn list(ops: &[Vec<u8>]) -> Vec<u8> {
    let mut v = (ops.len() as u32).to_be_bytes().to_vec();
    ops.iter().for_each(|o| v.extend_from_slice(o));
    v
  }

  fn ints(data: &RawImageData) -> &[u16] {
    match data {
      RawImageData::Integer(v) => v,
      _ => panic!("expected integer data"),
    }
  }

  #[test]
  fn quadratic_per_plane_16bit() {
    let ops = list(&[map_poly([0, 0, 1, 2], 0, 1, (1, 1), &[0.0, 0.0, 1.0]), map_poly([0, 0, 1, 2], 1, 1, (1, 1), &[0.5])]);
    let mut data = RawImageData::Integer(vec![32768, 1000, 7, 65535, 2000, 9]);
    apply_map_polynomial(&ops, &mut data, 2, 3, 16);
    assert_eq!(ints(&data), &[16384, 32768, 7, 65535, 32768, 9]);
  }

  #[test]
  fn normalizes_to_bit_depth() {
    let ops = list(&[map_poly([0, 0, 1, 1], 0, 1, (1, 1), &[0.0, 0.5])]);
    let mut data = RawImageData::Integer(vec![255]);
    apply_map_polynomial(&ops, &mut data, 1, 1, 8);
    assert_eq!(ints(&data), &[128]);
  }

  #[test]
  fn respects_area_and_pitch() {
    let ops = list(&[map_poly([1, 0, 4, 4], 0, 1, (2, 2), &[0.0])]);
    let mut data = RawImageData::Integer(vec![100; 16]);
    apply_map_polynomial(&ops, &mut data, 4, 1, 16);
    let changed: Vec<usize> = ints(&data).iter().enumerate().filter(|(_, v)| **v == 0).map(|(i, _)| i).collect();
    assert_eq!(changed, vec![4, 6, 12, 14]);
  }

  #[test]
  fn clamps_output_and_handles_float() {
    let ops = list(&[map_poly([0, 0, 1, 2], 0, 1, (1, 1), &[-1.0, 3.0])]);
    let mut data = RawImageData::Integer(vec![0, 65535]);
    apply_map_polynomial(&ops, &mut data, 2, 1, 16);
    assert_eq!(ints(&data), &[0, 65535]);
    let mut data = RawImageData::Float(vec![0.5, 1.0]);
    apply_map_polynomial(&ops, &mut data, 2, 1, 16);
    let RawImageData::Float(v) = data else { panic!("expected float data") };
    assert_eq!(v, vec![0.5, 2.0]);
  }

  #[test]
  fn skips_other_opcodes() {
    let ops = list(&[opcode(1, &[1, 2, 3, 4, 5]), map_poly([0, 0, 1, 1], 0, 1, (1, 1), &[0.0])]);
    let mut data = RawImageData::Integer(vec![500]);
    apply_map_polynomial(&ops, &mut data, 1, 1, 16);
    assert_eq!(ints(&data), &[0]);
  }

  #[test]
  fn malformed_lists_do_not_panic() {
    let good = map_poly([0, 0, 1, 1], 0, 1, (1, 1), &[0.0]);
    let cases: Vec<Vec<u8>> = vec![
      vec![],                                                           // empty
      vec![0, 0, 0, 5],                                                 // count without opcodes
      list(&[good.clone()])[..20].to_vec(),                             // truncated params
      list(&[map_poly([0, 0, 1, 1], u32::MAX, u32::MAX, (1, 1), &[0.0])]), // plane overflow
      list(&[map_poly([0, 0, u32::MAX, u32::MAX], 0, 1, (0, 0), &[0.0])]), // huge area, zero pitch
      list(&[opcode(8, &{
        let mut p = vec![0u8; 32];
        p.extend_from_slice(&u32::MAX.to_be_bytes()); // degree u32::MAX, no coefficients
        p
      })]),
      list(&[opcode(1, &[]), {
        let mut o = opcode(8, &[]);
        o[12..16].copy_from_slice(&u32::MAX.to_be_bytes()); // size pointing far past the end
        o
      }]),
    ];
    for case in cases {
      let mut data = RawImageData::Integer(vec![500; 4]);
      apply_map_polynomial(&case, &mut data, 2, 1, 16);
      let mut empty = RawImageData::Integer(vec![]);
      apply_map_polynomial(&case, &mut empty, 2, 1, 16);
    }
  }
}
