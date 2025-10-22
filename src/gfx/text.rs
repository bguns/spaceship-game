use std::ffi::OsStr;
use std::path::PathBuf;
use std::str::FromStr;
use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use harfrust::{
    Feature, GlyphBuffer, Language, Shaper, ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};
use skrifa::raw::TableProvider;
use smallvec::SmallVec;
use thiserror::Error;

use skrifa::{Axis, prelude::*};

#[derive(Debug, Error)]
enum FontError {
    #[error(
        "{0}: invalid font file extension ({1}) - accepted extensions are .ttf, .otf, .ttc, and .otc"
    )]
    FileExtension(String, String),
    #[error("{0}: invalid font file: the {1} field or record is required by this application")]
    MissingData(String, String),
    #[error(
        "{0}: invalid font file: named instance has no name defined at its specified subfamily_name_index: {1}"
    )]
    NamedInstanceHasNoName(String, u16),
    #[error("font with family \"{0}\"{} not cached", if let Some(sf) = .subfamily_name { format!(" and subfamily {}", sf) } else { " and no subfamily ".to_string() })]
    NotCached {
        family_name: String,
        subfamily_name: Option<String>,
    },
}

#[derive(Debug, Copy, Clone)]
enum FontFileType {
    Single,
    Collection,
}

impl FontFileType {
    fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        if std::fs::exists(&path)
            .with_context(|| format!("font file might not exist: {}", &path.as_ref().display()))?
        {
            match path.as_ref().extension().and_then(OsStr::to_str) {
                Some("ttc") | Some("otc") => Ok(FontFileType::Collection),
                Some("ttf") | Some("otf") => Ok(FontFileType::Single),
                Some(ext) => Err(FontError::FileExtension(
                    path.as_ref().to_string_lossy().into_owned(),
                    ext.into(),
                )
                .into()),
                None => Err(FontError::FileExtension(
                    path.as_ref().to_string_lossy().into_owned(),
                    "no extension".to_string(),
                )
                .into()),
            }
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("font file not found at path: {}", path.as_ref().display()),
            )
            .into())
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct FontIndex {
    raw_data_index: usize,
    collection_index: u32,
}

#[derive(Debug, Clone)]
pub struct NamedInstanceInfo {
    pub name: String,
    coords: skrifa::instance::Location,
}

impl NamedInstanceInfo {
    fn from_font_ref<P: AsRef<Path>>(
        font_path: P,
        font_ref: &FontRef<'_>,
        named_instance: &skrifa::NamedInstance<'_>,
    ) -> Result<Self> {
        let name: String = font_ref
            .localized_strings(named_instance.subfamily_name_id())
            .english_or_first()
            .map(|l| l.to_string())
            .ok_or_else(|| {
                FontError::NamedInstanceHasNoName(
                    font_path.as_ref().to_string_lossy().into_owned(),
                    named_instance.subfamily_name_id().to_u16(),
                )
            })?;
        let coords = named_instance.location();
        Ok(Self { name, coords })
    }
}

#[derive(Debug, Clone)]
pub struct FontInfo {
    cache_index: usize,
    pub family_name: String,
    pub subfamily_name: Option<String>,
}

pub struct FontCacheRef<'a> {
    font_cache: &'a FontCache,
    cache_index: usize,
}

impl<'a> FontCacheRef<'a> {
    pub fn variation_axes(&'a self) -> &'a [Axis] {
        &self.font_cache.variation_axes[self.cache_index]
    }

    pub fn named_instances(&'a self) -> &'a [NamedInstanceInfo] {
        &self.font_cache.named_instances[self.cache_index]
    }

    pub fn family_name(&'a self) -> &'a str {
        &self.font_cache.family_names[self.cache_index]
    }

    pub fn subfamily_name(&'a self) -> Option<&'a str> {
        self.font_cache.subfamily_names[self.cache_index].as_deref()
    }

    pub fn features(&'a self) -> &'a [String] {
        &self.font_cache.features[self.cache_index]
    }

    pub fn pretty_print(&'a self) -> String {
        format!(
            r#"
Font Family: {},
 Sub Family: {},
 Variations: {},
 Instances : {},
 Features  : {}
            "#,
            self.family_name(),
            self.subfamily_name().unwrap_or("/"),
            self.variation_axes()
                .iter()
                .map(|a| format!(
                    "{} [{}:{}:{}]",
                    a.tag().to_string(),
                    a.min_value(),
                    a.default_value(),
                    a.max_value()
                ))
                .reduce(|acc, el| format!("{}, {}", acc, el))
                .unwrap_or("/".to_string()),
            self.named_instances()
                .iter()
                .map(|n| n.name.clone())
                .reduce(|acc, el| format!("{}, {}", acc, el))
                .unwrap_or("/".to_string()),
            self.features()
                .iter()
                .map(|s| s.clone())
                .reduce(|acc, el| format!("{}, {}", acc, el))
                .unwrap_or("/".to_string())
        )
    }
}

impl<'a> std::fmt::Display for FontCacheRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(sub) = self.subfamily_name() {
            write!(f, "{} - {}", self.family_name(), sub)
        } else {
            write!(f, "{}", self.family_name())
        }
    }
}

impl<'a> std::fmt::Debug for FontCacheRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontCacheRef")
            .field("cache_index", &self.cache_index)
            .field("family_name()", &self.family_name())
            .field("subfamily_name()", &self.subfamily_name())
            .finish()
    }
}

pub struct FontCache {
    all_raw_data: Vec<u8>,
    paths: Vec<PathBuf>,
    paths_to_font_idxs: HashMap<PathBuf, std::ops::Range<usize>>,
    font_file_types: Vec<FontFileType>,
    raw_data_ranges: Vec<std::ops::Range<usize>>,

    font_ref_idxs: Vec<u32>,
    family_names: Vec<String>,
    subfamily_names: Vec<Option<String>>,
    variation_axes: Vec<SmallVec<[Axis; 4]>>,
    named_instances: Vec<SmallVec<[NamedInstanceInfo; 8]>>,
    features: Vec<SmallVec<[String; 4]>>,
}

impl FontCache {
    pub fn new() -> Self {
        Self {
            all_raw_data: Vec::new(),
            paths: Vec::new(),
            paths_to_font_idxs: HashMap::new(),
            font_file_types: Vec::new(),
            raw_data_ranges: Vec::new(),

            font_ref_idxs: Vec::new(),
            family_names: Vec::new(),
            subfamily_names: Vec::new(),
            variation_axes: Vec::new(),
            named_instances: Vec::new(),
            features: Vec::new(),
        }
    }

    pub fn load_font_file(&mut self, path: impl Into<PathBuf>) -> Result<Vec<FontInfo>> {
        let path: PathBuf = path.into();
        let indices = match self.paths_to_font_idxs.get(&path) {
            Some(idx) => idx.clone(),
            None => self.cache_raw_data(&path)?,
        };

        Ok(indices
            .into_iter()
            .map(|idx| FontInfo {
                cache_index: idx,
                family_name: self.family_names[idx].clone(),
                subfamily_name: self.subfamily_names[idx].clone(),
            })
            .collect())
    }

    pub fn find_font<'a>(
        &'a self,
        family_name: impl Into<String>,
        subfamily_name: Option<impl Into<String>>,
    ) -> Result<FontCacheRef<'a>> {
        let fam_name: String = family_name.into();
        let subfam_name: Option<String> = subfamily_name.map(|s| s.into());

        let family_idxs: Vec<usize> = self
            .family_names
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                **a == fam_name
                    && (subfam_name.is_none() || self.subfamily_names[*i] == subfam_name)
            })
            .map(|(i, _)| i)
            .collect();

        if family_idxs.len() == 1 || (subfam_name.is_none() && family_idxs.len() > 0) {
            Ok(FontCacheRef {
                font_cache: &self,
                cache_index: family_idxs[0],
            })
        } else {
            Err(FontError::NotCached {
                family_name: fam_name,
                subfamily_name: subfam_name,
            }
            .into())
        }
    }

    pub fn to_font_ref<'a>(&'a self, font_info: &FontInfo) -> FontCacheRef<'a> {
        FontCacheRef {
            font_cache: &self,
            cache_index: font_info.cache_index,
        }
    }

    fn cache_raw_data(&mut self, path: impl AsRef<Path>) -> Result<std::ops::Range<usize>> {
        let font_file_type = FontFileType::from_path(&path)?;
        let raw_bytes = std::fs::read(&path).with_context(|| {
            format!(
                "unable to read font file at path: {}",
                path.as_ref().display()
            )
        })?;

        let raw_data_len = raw_bytes.len();
        let start_index = self.all_raw_data.len();
        let end_index = start_index + raw_data_len;
        assert_eq!(
            end_index - start_index,
            raw_data_len,
            "font cache: the calculated range is not equal to the length of the inserted data"
        );
        let raw_data_range = start_index..end_index;

        let file_ref: skrifa::raw::FileRef = skrifa::raw::FileRef::new(&raw_bytes)?;

        let mut font_ref_idx: u32 = 0;
        let mut new_font_ref_idxs: Vec<u32> = Vec::new();
        let mut new_raw_data_ranges: Vec<std::ops::Range<usize>> = Vec::new();
        let mut new_family_names: Vec<String> = Vec::new();
        let mut new_subfamily_names: Vec<Option<String>> = Vec::new();
        let mut new_variation_axes: Vec<SmallVec<[Axis; 4]>> = Vec::new();
        let mut new_named_instances: Vec<SmallVec<[NamedInstanceInfo; 8]>> = Vec::new();
        let mut new_features: Vec<SmallVec<[String; 4]>> = Vec::new();

        for font in file_ref.fonts() {
            if font.is_err() {
                return Err(font.err().unwrap().into());
            }
            let font = font.unwrap();

            let family_name = font
                .localized_strings(skrifa::string::StringId::FAMILY_NAME)
                .english_or_first()
                .map(|l| l.to_string())
                .ok_or_else(|| {
                    FontError::MissingData(
                        path.as_ref().to_string_lossy().into_owned(),
                        "FAMILY_NAME".into(),
                    )
                })?;

            let subfamily_name = font
                .localized_strings(skrifa::string::StringId::SUBFAMILY_NAME)
                .english_or_first()
                .map(|l| l.to_string());

            let axes: SmallVec<[Axis; 4]> = font.axes().iter().collect();
            let mut named_instances: SmallVec<[NamedInstanceInfo; 8]> = SmallVec::new();

            for ni in font
                .named_instances()
                .iter()
                .map(|ni| NamedInstanceInfo::from_font_ref(&path, &font, &ni))
            {
                named_instances.push(ni?);
            }

            let features: SmallVec<[String; 4]> = font
                .feat()
                .map(|f| f.names())
                .into_iter()
                .flatten()
                .map(|n| {
                    font.localized_strings(n.name_index())
                        .english_or_first()
                        .map(|s| s.to_string())
                })
                .flatten()
                .collect();

            new_font_ref_idxs.push(font_ref_idx);
            new_raw_data_ranges.push(raw_data_range.clone());
            new_family_names.push(family_name);
            new_subfamily_names.push(subfamily_name);
            new_variation_axes.push(axes);
            new_named_instances.push(named_instances);
            new_features.push(features);

            font_ref_idx += 1;
        }

        let fonts_start_index: usize = self.raw_data_ranges.len();
        let fonts_end_index: usize = fonts_start_index + new_raw_data_ranges.len();

        let new_font_idxs = fonts_start_index..fonts_end_index;

        let raw_data_ranges_extended_len = self.raw_data_ranges.len() + new_raw_data_ranges.len();

        debug_assert_eq!(self.paths.len(), self.font_file_types.len());
        debug_assert_eq!(self.paths.len(), self.paths_to_font_idxs.len());
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.paths_to_font_idxs
                .values()
                .map(std::ops::Range::len)
                .sum::<usize>()
                + new_font_idxs.len()
        );
        debug_assert_eq!(
            self.raw_data_ranges
                .iter()
                .map(std::ops::Range::len)
                .sum::<usize>()
                + raw_data_range.len(),
            self.all_raw_data.len() + &raw_bytes.len()
        );
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.font_ref_idxs.len() + new_font_ref_idxs.len()
        );
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.family_names.len() + new_family_names.len()
        );
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.subfamily_names.len() + new_subfamily_names.len()
        );
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.variation_axes.len() + new_variation_axes.len()
        );
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.named_instances.len() + new_named_instances.len()
        );
        debug_assert_eq!(
            raw_data_ranges_extended_len,
            self.features.len() + new_features.len()
        );

        let path: PathBuf = path.as_ref().into();

        self.all_raw_data.extend(raw_bytes);
        self.paths.push(path.clone());
        self.font_file_types.push(font_file_type);
        self.paths_to_font_idxs.insert(path, new_font_idxs);

        self.raw_data_ranges.extend(new_raw_data_ranges);
        self.font_ref_idxs.extend(new_font_ref_idxs);
        self.family_names.extend(new_family_names);
        self.subfamily_names.extend(new_subfamily_names);
        self.variation_axes.extend(new_variation_axes);
        self.named_instances.extend(new_named_instances);
        self.features.extend(new_features);

        Ok(fonts_start_index..fonts_end_index)
    }

    /*fn cache_metadata(&mut self, index: usize) -> Result<()> {
        let file_type = self.font_file_types[index];
        let font_ref = self.font_ref_by_font_index(index, collection_index)
        Ok(())
    }*/
}

pub struct FontShaper<'a> {
    raw_bytes: Vec<u8>,
    font_ref: FontRef<'a>,
    shaper_data: ShaperData,
    shaper_instance: ShaperInstance,
    features: Option<Vec<Feature>>,
}

impl<'a> FontShaper<'a> {
    pub fn new<V, F>(
        font: FontRef<'a>,
        variations: Option<V>,
        features: Option<F>,
    ) -> FontShaper<'a>
    where
        V: IntoIterator,
        V::Item: Into<Variation>,
        F: IntoIterator<Item = Feature>,
    {
        let raw_bytes: Vec<u8> = font.data().as_bytes().into();
        let shaper_data = ShaperData::new(&font);

        let shaper_instance = if let Some(variations) = variations {
            ShaperInstance::from_variations(&font, variations)
        } else {
            ShaperInstance::from_variations(&font, &[] as &[Variation])
        };

        let features = features.map(|feats| {
            feats
                .into_iter()
                .map(|f| f.into())
                .collect::<Vec<Feature>>()
        });

        Self {
            raw_bytes,
            font_ref: font,
            shaper_data,
            shaper_instance: shaper_instance,
            features,
        }
    }

    pub fn shaper<V>(&'a mut self, variations: Option<V>) -> Shaper<'a>
    where
        V: IntoIterator,
        V::Item: Into<Variation>,
    {
        if let Some(vars) = variations {
            self.shaper_instance.set_variations(&self.font_ref, vars);
        }
        self.shaper_data
            .shaper(&self.font_ref.clone())
            .instance(Some(&self.shaper_instance))
            .point_size(None)
            .build()
    }

    pub fn shape(&'a mut self, line: &str, input_buffer: Option<UnicodeBuffer>) -> GlyphBuffer {
        let mut buffer = if let Some(input_buffer) = input_buffer {
            input_buffer
        } else {
            UnicodeBuffer::new()
        };

        buffer.push_str(line);

        buffer.set_language(Language::from_str("en").unwrap());
        buffer.guess_segment_properties();

        let shaper = self
            .shaper_data
            .shaper(&self.font_ref)
            .instance(Some(&self.shaper_instance))
            .point_size(None)
            .build();
        let result = shaper.shape(
            buffer,
            self.features.as_ref().map_or(&[] as &[Feature], |f| &f),
        );
        eprintln!(
            "{}",
            result.serialize(&shaper, harfrust::SerializeFlags::empty())
        );
        result
    }
}

pub struct Glyph {}
