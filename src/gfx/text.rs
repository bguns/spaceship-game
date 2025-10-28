use std::cell::{Ref, RefCell, UnsafeCell};
use std::ffi::OsStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Display, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use harfrust::{
    Feature, GlyphBuffer, Language, Shaper, ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};
use rayon::prelude::*;
use skrifa::metrics::Metrics;
use skrifa::raw::TableProvider;
use skrifa::{Axis, prelude::*};
use smallvec::SmallVec;
use thiserror::Error;

use crate::os::font_util;

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
    #[error("font with family \"{family_name}\"{} not cached", if let Some(sf) = .subfamily_name { format!(" and subfamily \"{}\"", sf) } else { " and no subfamily ".to_string() })]
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
            match path
                .as_ref()
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
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

#[derive(Debug, Clone, PartialEq)]
pub struct NamedInstanceInfo {
    pub name: String,
    named_instance_index: usize,
    coords: skrifa::instance::Location,
}

impl NamedInstanceInfo {
    fn from_font_ref<P: AsRef<Path>>(
        font_path: P,
        font_ref: &FontRef<'_>,
        named_instance_index: usize,
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
        Ok(Self {
            name,
            named_instance_index,
            coords,
        })
    }
}

#[derive(Clone)]
pub struct FontCacheRef<'a> {
    font_cache: &'a FontCache,
    cache_index: usize,
}

impl<'a> FontCacheRef<'a> {
    pub fn full_name(&self) -> String {
        format!(
            "{}{}",
            &self.font_cache.font_datas[self.cache_index].family_name,
            if let Some(sf) = self.font_cache.font_datas[self.cache_index]
                .subfamily_name
                .as_deref()
            {
                &format!(" - {}", sf)
            } else {
                ""
            }
        )
    }

    pub fn family_name(&self) -> &str {
        &self.font_cache.font_datas[self.cache_index].family_name
    }

    pub fn subfamily_name(&self) -> Option<&str> {
        self.font_cache.font_datas[self.cache_index]
            .subfamily_name
            .as_deref()
    }

    pub fn variation_axes(&self) -> &[Axis] {
        &self.font_cache.font_datas[self.cache_index].variation_axes
    }

    pub fn named_instances(&self) -> &[NamedInstanceInfo] {
        &self.font_cache.font_datas[self.cache_index].named_instances
    }

    pub fn features(&self) -> &[String] {
        &self.font_cache.font_datas[self.cache_index].features
    }

    pub fn pretty_print(&self) -> String {
        format!(
            r#"
Font Family: {}
 Sub Family: {}
 Variations: {}
 Instances : {}
 Features  : {}
            "#,
            self.family_name(),
            self.subfamily_name().unwrap_or("/"),
            self.variation_axes()
                .iter()
                .map(|a| format!(
                    "{} [{} - {} - {}]",
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

    fn revision(&self) -> &skrifa::raw::types::Fixed {
        &self.font_cache.font_datas[self.cache_index].revision
    }

    fn to_font_ref(&'a self) -> FontRef<'a> {
        let font_data = &self.font_cache.font_datas[self.cache_index];
        FontRef::from_index(
            &self.font_cache.all_raw_data[font_data.raw_data_range.clone()],
            font_data.font_ref_idx,
        )
        .expect("Unable to obtain FontRef for already cached font!")
    }

    /*fn shaper_instance<V>(
        &'a self,
        settings: Option<ShaperInstanceSettings<V>>,
    ) -> Option<Ref<'a, ShaperInstance>>
    where
        V: IntoIterator<Item = Variation>,
    {
        let shaper_instance = &self.font_cache.font_datas[self.cache_index].shaper_instance;
        let font_ref = self.to_font_ref();
        match settings {
            Some(ShaperInstanceSettings::Variations(variations)) => {
                shaper_instance
                    .borrow_mut()
                    .unwrap()
                    .set_variations(&font_ref, variations);
            }
            Some(ShaperInstanceSettings::NamedInstance(named_instance_info)) => {
                shaper_instance
                    .borrow_mut()
                    .unwrap()
                    .set_named_instance(&font_ref, named_instance_info.named_instance_index);
            }
            None => {}
        }

        shaper_instance.borrow()
    }*/

    /*fn shaper_instance_settings<V>(&'a mut self, settings: ShaperInstanceSettings<V>) -> Result<()>
    where
        V: IntoIterator<Item = Variation>,
    {
        {
            //let shaper_instance = self.shaper_instance.as_mut();
            if let None = self.shaper_instance {
                self.shaper_instance = Some(ShaperInstance::from_variations(
                    &self.to_font_ref(),
                    &[] as &[Variation],
                ));
            }
        }

        let font_ref = &self.to_font_ref();

        let shaper_instance = &mut self.shaper_instance.as_mut().unwrap();
        match settings {
            ShaperInstanceSettings::Variations(variations) => {
                shaper_instance.set_variations(&font_ref, variations);
            }
            ShaperInstanceSettings::NamedInstance(named_instance_info) => {
                shaper_instance
                    .set_named_instance(&font_ref, named_instance_info.named_instance_index);
            }
        }

        Ok(())
    }*/

    /*fn shaper<V>(&'a self, settings: Option<ShaperInstanceSettings<V>>) -> Shaper<'a>
    where
        V: IntoIterator<Item = Variation>,
    {
        let shaper_instance = self.font_cache.font_datas[self.cache_index].shaper_instance;
        let font_ref = self.to_font_ref();
        if let Some(settings) = settings {
            match settings {
                ShaperInstanceSettings::Variations(variations) => {
                    shaper_instance
                        .borrow_mut()
                        .as_mut()
                        .map(|si| si.set_variations(&font_ref, variations));
                    //.set_variations(&font_ref, variations);
                }
                ShaperInstanceSettings::NamedInstance(named_instance_info) => {
                    shaper_instance.borrow_mut().as_mut().map(|si| {
                        si.set_named_instance(&font_ref, named_instance_info.named_instance_index)
                    });
                }
            }
        }
        let shaper = self
            .shaper_data()
            .shaper(&self.to_font_ref())
            .instance(shaper_instance.borrow().as_ref())
            .point_size(None)
            .build();

        shaper
    }*/
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
            .field(
                "variation_axes()",
                &self
                    .variation_axes()
                    .iter()
                    .map(|a| {
                        format!(
                            "{} [{} - {} - {}]",
                            a.tag().to_string(),
                            a.min_value(),
                            a.default_value(),
                            a.max_value()
                        )
                    })
                    .collect::<Vec<String>>()
                    .as_slice(),
            )
            .field(
                "named_instances()",
                &self
                    .named_instances()
                    .iter()
                    .map(|ni| ni.name.clone())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .finish()
    }
}

impl<'a> std::cmp::PartialEq for FontCacheRef<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.cache_index == other.cache_index
    }
}

impl<'a> std::cmp::Eq for FontCacheRef<'a> {}

impl<'a> std::cmp::PartialOrd for FontCacheRef<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.cache_index.partial_cmp(&other.cache_index)
    }
}

impl<'a> std::cmp::Ord for FontCacheRef<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cache_index.cmp(&other.cache_index)
    }
}

impl<'a> std::hash::Hash for FontCacheRef<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.cache_index.hash(state);
    }
}

struct FontCacheData {
    font_ref_idx: u32,
    raw_data_range: std::ops::Range<usize>,
    family_name: String,
    subfamily_name: Option<String>,
    revision: skrifa::raw::types::Fixed,
    unscaled_default_metrics: Metrics,
    variation_axes: SmallVec<[Axis; 4]>,
    named_instances: SmallVec<[NamedInstanceInfo; 8]>,
    features: SmallVec<[String; 32]>,
    shaper_data: UnsafeCell<Option<Box<ShaperData>>>,
    shaper_instance: ShaperInstance,
}

impl std::fmt::Debug for FontCacheData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontCacheData")
            .field("font_ref_idx", &self.font_ref_idx)
            //.field("raw_data_range", &self.raw_data_range)
            .field("family_name", &self.family_name)
            .field("subfamily_name", &self.subfamily_name)
            .field("revision", &self.revision)
            .field("unscaled_default_metrics", &self.unscaled_default_metrics)
            .field(
                "variation_axes",
                &self
                    .variation_axes
                    .iter()
                    .map(|a| {
                        format!(
                            "{} [{} - {} - {}]",
                            a.tag().to_string(),
                            a.min_value(),
                            a.default_value(),
                            a.max_value()
                        )
                    })
                    .collect::<Vec<String>>()
                    .as_slice(),
            )
            .field("named_instances", &self.named_instances)
            .field("features", &self.features)
            .finish()
    }
}

struct RawFontCacheData {
    font_ref_idx: u32,
    family_name: String,
    subfamily_name: Option<String>,
    revision: skrifa::raw::types::Fixed,
    unscaled_default_metrics: Metrics,
    variation_axes: SmallVec<[Axis; 4]>,
    named_instances: SmallVec<[NamedInstanceInfo; 8]>,
    features: SmallVec<[String; 32]>,
}

enum RawCacheResult {
    AlreadyCached {
        path: PathBuf,
    },
    New {
        path: PathBuf,
        raw_data: Vec<u8>,
        raw_data_hash: u64,
        font_file_type: FontFileType,
        font_datas: Vec<RawFontCacheData>,
    },
}

#[allow(unused)]
enum CacheResult {
    New {
        path: PathBuf,
        newly_cached: SmallVec<[usize; 16]>,
        replaced: SmallVec<[usize; 16]>,
    },
    AlreadyCached {
        path: PathBuf,
        idxs: SmallVec<[usize; 16]>,
    },
    NoNewData(PathBuf),
}

pub struct FontCache {
    all_raw_data: Vec<u8>,
    raw_data_hashes_to_paths: HashMap<u64, PathBuf>,
    paths: Vec<PathBuf>,
    paths_to_font_idxs: HashMap<PathBuf, SmallVec<[usize; 16]>>,
    paths_to_data_ranges: HashMap<PathBuf, std::ops::Range<usize>>,
    font_file_types: Vec<FontFileType>,

    font_datas: Vec<FontCacheData>,
}

#[allow(unused)]
impl FontCache {
    pub fn new() -> Self {
        Self {
            all_raw_data: Vec::new(),
            raw_data_hashes_to_paths: HashMap::new(),
            paths: Vec::new(),
            paths_to_font_idxs: HashMap::new(),
            paths_to_data_ranges: HashMap::new(),
            font_file_types: Vec::new(),

            font_datas: Vec::new(),
        }
    }

    pub fn list_fonts(&self, show_path: bool) {
        let mut fonts: Vec<String> = self
            .font_datas
            .iter()
            .enumerate()
            .map(|(i, font)| {
                if show_path {
                    let path = self
                        .paths_to_font_idxs
                        .iter()
                        .find(|(_, v)| v.contains(&i))
                        .unwrap()
                        .0;
                    format!(
                        "{} - {} -- [{}]",
                        font.family_name,
                        font.subfamily_name.as_deref().unwrap_or("/"),
                        path.to_string_lossy()
                    )
                } else {
                    format!(
                        "{} - {}",
                        font.family_name,
                        font.subfamily_name.as_deref().unwrap_or("/")
                    )
                }
            })
            .collect();

        fonts.sort();

        for font in fonts {
            eprintln!("{}", font);
        }
    }

    pub fn load_system_fonts(&mut self) -> Result<usize> {
        let system_font_paths = font_util::load_system_font_paths()?;
        self.load_multiple_font_files(system_font_paths)
    }

    pub fn data_size(&self) -> usize {
        self.all_raw_data.len()
    }

    pub fn load_multiple_font_files(&mut self, paths: Vec<impl Into<PathBuf>>) -> Result<usize> {
        let result_count_heuristic = 2 * paths.len();

        let raw_data_hashes_to_paths = Arc::new(&self.raw_data_hashes_to_paths);

        let raw_datas: Vec<Result<RawCacheResult>> = paths
            .into_iter()
            .map(|path| path.into())
            .collect::<Vec<PathBuf>>()
            .into_par_iter()
            .map(|path| Self::load_raw_data(path, raw_data_hashes_to_paths.clone()))
            .collect();

        let mut result_idxs: Vec<usize> = Vec::with_capacity(result_count_heuristic);

        for raw_data in raw_datas.into_iter() {
            let idxs = match self.store_raw_data(raw_data)? {
                CacheResult::New {
                    newly_cached,
                    replaced,
                    ..
                } => newly_cached
                    .into_iter()
                    .chain(replaced.into_iter())
                    .collect::<Vec<usize>>(),
                CacheResult::AlreadyCached { idxs, .. } => idxs.into_iter().collect::<Vec<usize>>(),
                CacheResult::NoNewData(_) => Vec::new(),
            };

            result_idxs.extend(idxs);
        }

        result_idxs.sort();
        result_idxs.dedup();

        Ok(result_idxs.len())
    }

    pub fn load_font_file(&mut self, path: impl Into<PathBuf>) -> Result<()> {
        let path: PathBuf = path.into();
        let raw_data_hashes_to_paths = Arc::new(&self.raw_data_hashes_to_paths);
        self.store_raw_data(Self::load_raw_data(&path, raw_data_hashes_to_paths.clone()))?;
        Ok(())
    }

    pub fn find_font<'a>(
        &'a self,
        family_name: impl Into<String>,
        subfamily_name: Option<impl Into<String>>,
    ) -> Result<FontCacheRef<'a>> {
        fn is_match(
            cached_family_name: &str,
            cached_subfamily_name: Option<&str>,
            family_name: &str,
            subfamily_name: Option<&str>,
        ) -> bool {
            cached_family_name.to_ascii_lowercase() == family_name.to_ascii_lowercase()
                && (subfamily_name.is_none()
                    || cached_subfamily_name
                        .as_ref()
                        .map(|s| s.to_ascii_lowercase())
                        == subfamily_name.map(|s| s.to_ascii_lowercase()))
        }

        let fam_name: String = family_name.into();
        let subfam_name: Option<String> = subfamily_name.map(|s| s.into());

        let family_idxs: Vec<usize> = self
            .font_datas
            .iter()
            .enumerate()
            .filter_map(|(i, fd)| {
                is_match(
                    &fd.family_name,
                    fd.subfamily_name.as_deref(),
                    &fam_name,
                    subfam_name.as_deref(),
                )
                .then_some(i)
            })
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

    pub fn search_fonts<'a>(&'a self, search_string: impl Into<String>) -> Vec<FontCacheRef<'a>> {
        fn is_match(
            cached_family_name: &str,
            cached_subfamily_name: Option<&str>,
            family_name: &str,
            subfamily_name: Option<&str>,
        ) -> bool {
            cached_family_name
                .to_ascii_lowercase()
                .contains(&family_name.to_lowercase())
                && (subfamily_name.is_none()
                    || if let Some(s) = cached_subfamily_name.map(|s| s.to_ascii_lowercase()) {
                        s.contains(&subfamily_name.unwrap().to_lowercase())
                    } else {
                        false
                    })
        }

        let mut results: Vec<FontCacheRef<'a>> = Vec::with_capacity(8);

        let ss: String = search_string.into();
        let terms: Vec<&str> = ss.split(' ').collect();
        let tlen = terms.len();

        for i in 0..=tlen {
            let one = terms[0..i].join(" ");
            let two = terms[i..tlen].join(" ");

            results.extend(
                self.font_datas
                    .iter()
                    .enumerate()
                    .filter(|(_, fd)| {
                        !one.is_empty()
                            && is_match(
                                &fd.family_name,
                                fd.subfamily_name.as_deref(),
                                &one,
                                if two.is_empty() { None } else { Some(&two) },
                            )
                            || !two.is_empty()
                                && is_match(
                                    &fd.family_name,
                                    fd.subfamily_name.as_deref(),
                                    &two,
                                    if one.is_empty() { None } else { Some(&one) },
                                )
                    })
                    .map(|(idx, _)| FontCacheRef {
                        font_cache: &self,
                        cache_index: idx,
                    }),
            );
        }

        results.sort();
        results.dedup();

        results
    }

    fn to_font_ref<'a>(&'a self, cache_index: usize) -> FontCacheRef<'a> {
        FontCacheRef {
            font_cache: &self,
            cache_index: cache_index,
        }
    }

    fn load_raw_data(
        path: impl AsRef<Path>,
        raw_data_hashes_to_paths: Arc<&HashMap<u64, PathBuf>>,
    ) -> Result<RawCacheResult> {
        let font_file_type = FontFileType::from_path(&path)?;
        let raw_bytes = std::fs::read(&path).with_context(|| {
            format!(
                "unable to read font file at path: {}",
                path.as_ref().display()
            )
        })?;

        // Hash the raw bytes
        let mut hasher = DefaultHasher::new();
        raw_bytes.hash(&mut hasher);
        let raw_data_hash = hasher.finish();

        // Check if an already parsed file contained identical data
        if let Some(p) = raw_data_hashes_to_paths.get(&raw_data_hash) {
            return Ok(RawCacheResult::AlreadyCached {
                path: path.as_ref().into(),
            });
        }

        // Load the data with skrifa
        let file_ref: skrifa::raw::FileRef = skrifa::raw::FileRef::new(&raw_bytes)?;

        // the font_ref_idx (used by FontRef::from_index). Will be incremented at the start of the loop,
        // so init to -1
        let mut font_ref_idx: i32 = -1;

        let mut font_datas: Vec<RawFontCacheData> = Vec::new();

        for font in file_ref.fonts() {
            font_ref_idx += 1;
            if font.is_err() {
                return Err(font.err().unwrap().into());
            }
            let font = font.unwrap();

            // collect all the required font data
            let font_revision = font.head().unwrap().font_revision();

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
                .enumerate()
                .map(|(idx, ni)| NamedInstanceInfo::from_font_ref(&path, &font, idx, &ni))
            {
                named_instances.push(ni?);
            }

            let mut features: SmallVec<[String; 32]> = font
                .gsub()
                .iter()
                .flat_map(|g| g.feature_list())
                .flat_map(|fl| fl.feature_records())
                .map(|f| f.feature_tag().to_string())
                .chain(
                    font.gpos()
                        .iter()
                        .flat_map(|g| g.feature_list())
                        .flat_map(|fl| fl.feature_records())
                        .map(|f| f.feature_tag().to_string()),
                )
                .collect::<SmallVec<[String; 32]>>();

            features.sort();
            features.dedup();

            let metrics = font.metrics(Size::unscaled(), LocationRef::default());

            font_datas.push(RawFontCacheData {
                font_ref_idx: font_ref_idx as u32,
                //raw_data_range: raw_data_range.clone(),
                family_name,
                subfamily_name,
                revision: font_revision,
                unscaled_default_metrics: metrics,
                variation_axes: axes,
                named_instances,
                features,
            })
        }

        Ok(RawCacheResult::New {
            path: path.as_ref().into(),
            raw_data: raw_bytes,
            raw_data_hash,
            font_file_type,
            font_datas,
        })
    }

    fn store_raw_data(&mut self, raw_cache_data: Result<RawCacheResult>) -> Result<CacheResult> {
        if raw_cache_data.is_err() {
            return Err(raw_cache_data.err().unwrap());
        }
        let raw_cache_data = raw_cache_data.unwrap();
        let (path, raw_data, raw_data_hash, font_file_type, font_datas) = match raw_cache_data {
            RawCacheResult::New {
                path,
                raw_data,
                raw_data_hash,
                font_file_type,
                font_datas,
            } => (path, raw_data, raw_data_hash, font_file_type, font_datas),
            RawCacheResult::AlreadyCached { path } => {
                let idxs = self.paths_to_font_idxs.get(&path).unwrap().clone();
                return Ok(CacheResult::AlreadyCached { path, idxs });
            }
        };

        // Construct the range (window) on all the cached raw data that will correspond to the data in this file
        let raw_data_len = raw_data.len();
        let start_index = self.all_raw_data.len();
        let end_index = start_index + raw_data_len;
        assert_eq!(
            end_index - start_index,
            raw_data_len,
            "font cache: the calculated range is not equal to the length of the inserted data"
        );
        let raw_data_range = start_index..end_index;

        // new_font_datas.len() + replace_font_datas.len() + skipped_font_datas should equal the number
        // of fonts in the file_ref
        let font_datas_length = font_datas.len();
        let mut new_font_datas: Vec<FontCacheData> = Vec::new();
        let mut replace_font_datas: Vec<(usize, FontCacheData)> = Vec::new();
        let mut skipped_font_datas: usize = 0;

        for raw_font_cache_data in font_datas {
            let fd = FontCacheData {
                font_ref_idx: raw_font_cache_data.font_ref_idx,
                raw_data_range: raw_data_range.clone(),
                family_name: raw_font_cache_data.family_name,
                subfamily_name: raw_font_cache_data.subfamily_name,
                revision: raw_font_cache_data.revision,
                unscaled_default_metrics: raw_font_cache_data.unscaled_default_metrics,
                variation_axes: raw_font_cache_data.variation_axes,
                named_instances: raw_font_cache_data.named_instances,
                features: raw_font_cache_data.features,
                shaper_data: UnsafeCell::new(None),
                shaper_instance: ShaperInstance::from_variations(
                    &FontRef::from_index(&raw_data, raw_font_cache_data.font_ref_idx)?,
                    &[] as &[Variation],
                ),
            };
            // Check if an this font is the same family + subfamily, but with "better"
            // properties
            if let Ok(existing) = self.find_font(&fd.family_name, fd.subfamily_name.as_ref()) {
                if fd.variation_axes.len() > existing.variation_axes().len()
                    || (fd.variation_axes.len() == existing.variation_axes().len()
                        && fd.features.len() > existing.features().len())
                    || (fd.variation_axes.len() == existing.variation_axes().len()
                        && fd.features.len() == existing.features().len()
                        && fd.named_instances.len() > existing.named_instances().len())
                    || (fd.variation_axes.len() == existing.variation_axes().len()
                        && fd.features.len() == existing.features().len()
                        && fd.named_instances.len() == existing.named_instances().len()
                        && fd.revision > *existing.revision())
                {
                    // if a duplicate font exists and the new one is better, replace the font
                    // at the existing index
                    replace_font_datas.push((existing.cache_index, fd))
                } else {
                    // if a duplicate font exists but the new one is not better,
                    // skip the new font
                    skipped_font_datas += 1;
                    continue;
                }
            } else {
                // if no duplicate is found, simply add the font
                new_font_datas.push(fd);
            };
        }

        // all fonts in the file should be processed
        debug_assert_eq!(
            new_font_datas.len() + replace_font_datas.len() + skipped_font_datas,
            // lengths start at 1, font_ref_index starts at 0
            font_datas_length,
            "{}",
            path.to_string_lossy()
        );

        // if all fonts were skipped, only save the path and hashed data values
        // (so the cache can verify this file/data was already processed)
        // and return an empty smallvec
        if new_font_datas.is_empty() && replace_font_datas.is_empty() {
            //let path: PathBuf = path.as_ref().into();
            self.paths.push(path.clone());
            self.raw_data_hashes_to_paths
                .insert(raw_data_hash, path.clone());
            self.font_file_types.push(font_file_type);

            self.paths_to_font_idxs
                .insert(path.clone(), SmallVec::default());
            self.paths_to_data_ranges
                .insert(path.clone(), std::ops::Range::default());

            return Ok(CacheResult::NoNewData(path));
        }

        // new font cache indexes
        let new_fonts_start_index: usize = self.font_datas.len();
        let new_fonts_end_index: usize = new_fonts_start_index + new_font_datas.len();

        // we cannot store as a Range, because replacing might mean having to remove an
        // index somewhere
        let new_font_idxs: SmallVec<[usize; 16]> =
            (new_fonts_start_index..new_fonts_end_index).collect();

        let font_datas_extended_len = self.font_datas.len() + new_font_datas.len();

        let replaced_font_idxs: SmallVec<[usize; 16]> =
            replace_font_datas.iter().map(|r| r.0).collect();

        // sanity checks *before* modifications

        // failing these three means something went wrong on a previous
        // load, but there is no more sensible check
        // luckily the operations involved with these vecs are pretty much infallible
        debug_assert_eq!(
            self.paths.len(),
            self.font_file_types.len(),
            "{}",
            path.to_string_lossy()
        );
        debug_assert_eq!(
            self.paths.len(),
            self.paths_to_font_idxs.len(),
            "{}",
            path.to_string_lossy()
        );
        debug_assert_eq!(
            self.paths.len(),
            self.raw_data_hashes_to_paths.len(),
            "{}",
            path.to_string_lossy()
        );

        // all fonts should be referred to by only a single path, therefore the
        // length of all the indexes mapped to by all the paths + the new indexes
        // should equal all the font datas + the new datas
        debug_assert_eq!(
            font_datas_extended_len,
            self.paths_to_font_idxs
                .values()
                .map(SmallVec::len)
                .sum::<usize>()
                + new_font_idxs.len(),
            "{}",
            path.to_string_lossy()
        );

        // the raw data ranges of all the paths + the new raw data range
        // should equal the length of the existing raw data + the length of the new raw data
        debug_assert_eq!(
            self.paths_to_data_ranges
                .values()
                .map(|r| r.len())
                .sum::<usize>()
                + raw_data_range.len(),
            self.all_raw_data.len() + &raw_data.len(),
            "{}",
            path.to_string_lossy()
        );

        // add path and hash related stuff
        // need to do this now because we need this data to properly process
        // replacements
        self.all_raw_data.extend(raw_data);
        self.paths.push(path.clone());
        self.raw_data_hashes_to_paths
            .insert(raw_data_hash, path.clone());
        self.font_file_types.push(font_file_type);

        let mut path_to_font_idxs = new_font_idxs.clone();
        path_to_font_idxs.extend(replace_font_datas.iter().map(|fd| fd.0));

        self.paths_to_font_idxs
            .insert(path.clone(), path_to_font_idxs.clone());

        self.paths_to_data_ranges
            .insert(path.clone(), raw_data_range);

        for font_data in replace_font_datas {
            let old_paths: Vec<&PathBuf> = self
                .paths_to_font_idxs
                .iter()
                // (we already added the new path to indexes, so ignore the current path in our search)
                .filter_map(|(p, is)| (*p != path && is.contains(&font_data.0)).then_some(p))
                .collect();

            // sanity check: ignoring the new path, the cache index should have beeen referenced by exactly
            // one other path
            debug_assert_eq!(
                1,
                old_paths.len(),
                "font at cache index {} is linked to multiple (or no) file paths: [{}]",
                &font_data.0,
                old_paths
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .reduce(|acc, el| format!("{}; \"{}\"", acc, el).into())
                    .unwrap_or("NO PATHS".into())
            );

            let old_path = old_paths[0].clone();

            let old_path_idxs = self.paths_to_font_idxs.get_mut(&old_path).unwrap();

            // remove cache index from old path
            old_path_idxs.remove(
                old_path_idxs
                    .iter()
                    .position(|i| i == &font_data.0)
                    .unwrap(),
            );

            self.font_datas[font_data.0] = font_data.1;
        }

        // double check that the sum of all cache indexes referenced by paths is exactly equal
        // to all the fonts (to be) in the cache
        debug_assert_eq!(
            font_datas_extended_len,
            self.paths_to_font_idxs
                .values()
                .map(SmallVec::len)
                .sum::<usize>(),
            "{}",
            path.to_string_lossy()
        );

        self.font_datas.extend(new_font_datas);

        Ok(CacheResult::New {
            path,
            newly_cached: new_font_idxs,
            replaced: replaced_font_idxs,
        })
    }

    fn shaper_data<'a>(&'a self, font_cache_ref: &FontCacheRef<'_>) -> &'a ShaperData {
        let font_data = &self.font_datas[font_cache_ref.cache_index];

        // Safety: FontCacheData initialization sets shaper_data to None.
        // The only place where shaper_data can be mutated is here, and only once.
        // The only place where references to shaper_data can be obtained from is here.
        // The return value's lifetime is tied to the font cache
        let value = unsafe {
            let maybe_shaper = &mut *font_data.shaper_data.get();
            if let None = maybe_shaper {
                *maybe_shaper = Some(Box::new(ShaperData::new(&FontRef::from_index(&self.all_raw_data[font_data.raw_data_range.clone()], font_data.font_ref_idx).expect(&format!("Failed to construct FontRef from FontCacheData (for {}; cache_idx: {}, font_ref_idx:{})", font_cache_ref.full_name(), font_cache_ref.cache_index, font_data.font_ref_idx)))));
            }
            &*maybe_shaper
        };

        // Value is guaranteed to be initialized with Some(Box(ShaperData)) at this point
        value.as_deref().unwrap()
    }

    pub fn font_shaper<'a>(
        &'a self,
        font_cache_ref: &'a FontCacheRef<'a>,
        settings: Option<ShaperSettings>,
    ) -> FontShaper<'a> {
        FontShaper::new(
            font_cache_ref,
            font_cache_ref.to_font_ref(),
            &self.shaper_data(font_cache_ref),
            settings.unwrap_or_else(|| ShaperSettings {
                instance_settings: None,
                shape_features: None,
            }),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShaperInstanceSettings {
    Variations(Vec<Variation>),
    NamedInstance(NamedInstanceInfo),
}

impl ShaperInstanceSettings {
    pub fn variations(variations: impl IntoIterator<Item: Into<Variation>>) -> Self {
        Self::Variations(variations.into_iter().map(|v| v.into()).collect())
    }

    pub fn named_instance(named_instance_info: &NamedInstanceInfo) -> Self {
        Self::NamedInstance(named_instance_info.clone())
    }
}

impl std::fmt::Display for ShaperInstanceSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Variations(variations) => write!(
                f,
                "Variations: {}",
                variations
                    .iter()
                    .map(|v| format!("[{}:{}]", v.tag.to_string(), v.value))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Self::NamedInstance(ni) => write!(f, "Named Instance: {}", ni.name),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShaperSettings {
    instance_settings: Option<ShaperInstanceSettings>,
    shape_features: Option<Vec<Feature>>,
}

impl ShaperSettings {
    pub fn new(
        instance_settings: Option<ShaperInstanceSettings>,
        shape_features: Option<impl IntoIterator<Item: Into<Feature>>>,
    ) -> Self {
        Self {
            instance_settings,
            shape_features: shape_features
                .map(|feats| feats.into_iter().map(|f| f.into()).collect()),
        }
    }
}

impl std::fmt::Display for ShaperSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut has_instance_settings: bool = false;
        let mut has_features: bool = false;
        let instance_settings_str: String = if let Some(ref is) = self.instance_settings {
            has_instance_settings = true;
            format!("{}", is)
        } else {
            "".to_string()
        };
        let features_str = if let Some(ref feats) = self.shape_features {
            has_features = true;
            format!(
                "Features: [{}]",
                feats
                    .iter()
                    .map(|f| format!("{}:{}", f.tag.to_string(), f.value))
                    .collect::<Vec<String>>()
                    .join(",")
            )
        } else {
            "".to_string()
        };

        if has_instance_settings && has_features {
            write!(f, "{}, {}", instance_settings_str, features_str)
        } else if has_instance_settings {
            write!(f, "{}", instance_settings_str)
        } else if has_features {
            write!(f, "{}", features_str)
        } else {
            write!(f, "[Default]")
        }
    }
}

pub struct FontShaper<'a> {
    font_cache_ref: &'a FontCacheRef<'a>,
    font_ref: FontRef<'a>,
    shaper_data: &'a ShaperData,
    shaper_settings: ShaperSettings,
    shaper_instance: ShaperInstance,
    features: Vec<Feature>,
}

impl<'a> FontShaper<'a> {
    fn new(
        font_cache_ref: &'a FontCacheRef<'a>,
        font: FontRef<'a>,
        shaper_data: &'a ShaperData,
        shaper_settings: ShaperSettings,
    ) -> FontShaper<'a> {
        //let shaper_data = ShaperData::new(&font);

        let shaper_instance = match shaper_settings.instance_settings {
            Some(ShaperInstanceSettings::Variations(ref variations)) => {
                ShaperInstance::from_variations(&font, variations)
            }
            Some(ShaperInstanceSettings::NamedInstance(ref ni)) => {
                ShaperInstance::from_named_instance(&font, ni.named_instance_index)
            }
            None => ShaperInstance::from_variations(&font, &[] as &[Variation]),
        };

        let features = shaper_settings
            .clone()
            .shape_features
            .map(|f| {
                f.into_iter()
                    .map(|feat| feat.into())
                    .collect::<Vec<Feature>>()
            })
            .unwrap_or_default();

        Self {
            font_cache_ref,
            font_ref: font,
            shaper_data,
            shaper_settings,
            shaper_instance: shaper_instance,
            features,
        }
    }

    pub fn with_settings(mut self, settings: ShaperSettings) -> Self {
        if settings == self.shaper_settings {
            return self;
        }
        if let Some(instance_settings) = settings.instance_settings {
            match instance_settings {
                ShaperInstanceSettings::Variations(variations) => self
                    .shaper_instance
                    .set_variations(&self.font_ref, variations),
                ShaperInstanceSettings::NamedInstance(ni) => self
                    .shaper_instance
                    .set_named_instance(&self.font_ref, ni.named_instance_index),
            }
        }

        if let Some(shape_features) = settings.shape_features {
            self.features = shape_features
        }

        self
    }

    pub fn shape(&'a self, line: &str, input_buffer: Option<UnicodeBuffer>) -> GlyphBuffer {
        let mut buffer = if let Some(mut input_buffer) = input_buffer {
            input_buffer.clear();
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
        let result = shaper.shape(buffer, &self.features);
        eprintln!(
            "shaping \"{}\" in font \"{}\" with settings: {{ {} }}\n  {}",
            line,
            self.font_cache_ref.full_name(),
            self.shaper_settings,
            result.serialize(&shaper, harfrust::SerializeFlags::empty())
        );
        result
    }
}

pub struct Glyph {}
