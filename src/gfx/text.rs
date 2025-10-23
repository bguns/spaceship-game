use std::collections::HashSet;
use std::ffi::OsStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::str::FromStr;
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

#[derive(Clone)]
pub struct FontCacheRef<'a> {
    font_cache: &'a FontCache,
    cache_index: usize,
}

impl<'a> FontCacheRef<'a> {
    pub fn family_name(&'a self) -> &'a str {
        &self.font_cache.font_datas[self.cache_index].family_name
    }

    pub fn subfamily_name(&'a self) -> Option<&'a str> {
        self.font_cache.font_datas[self.cache_index]
            .subfamily_name
            .as_deref()
    }

    pub fn variation_axes(&'a self) -> &'a [Axis] {
        &self.font_cache.font_datas[self.cache_index].variation_axes
    }

    pub fn named_instances(&'a self) -> &'a [NamedInstanceInfo] {
        &self.font_cache.font_datas[self.cache_index].named_instances
    }

    pub fn features(&'a self) -> &'a [String] {
        &self.font_cache.font_datas[self.cache_index].features
    }

    pub fn pretty_print(&'a self) -> String {
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

    fn revision(&'a self) -> &'a skrifa::raw::types::Fixed {
        &self.font_cache.font_datas[self.cache_index].revision
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
}

pub struct FontCache {
    all_raw_data: Vec<u8>,
    raw_data_hashes_to_paths: HashMap<u64, PathBuf>,
    paths: Vec<PathBuf>,
    paths_to_font_idxs: HashMap<PathBuf, std::ops::Range<usize>>,
    font_file_types: Vec<FontFileType>,

    font_datas: Vec<FontCacheData>,
}

impl FontCache {
    pub fn new() -> Self {
        Self {
            all_raw_data: Vec::new(),
            raw_data_hashes_to_paths: HashMap::new(),
            paths: Vec::new(),
            paths_to_font_idxs: HashMap::new(),
            font_file_types: Vec::new(),

            font_datas: Vec::new(),
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
                family_name: self.font_datas[idx].family_name.clone(),
                subfamily_name: self.font_datas[idx].subfamily_name.clone(),
            })
            .collect())
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

        let mut result_set: HashSet<FontCacheRef<'a>> = HashSet::new();

        let ss: String = search_string.into();
        let terms: Vec<&str> = ss.split(' ').collect();
        let tlen = terms.len();

        let t = std::time::Instant::now();

        /*result_set.par_extend((0..=tlen).into_par_iter().flat_map(|i| {
            let one = terms[0..i].join(" ");
            let two = terms[i..tlen].join(" ");
            self.family_names
                .par_iter()
                .zip(self.subfamily_names.par_iter())
                .enumerate()
                .filter(move |(_, (cached_name, cached_sub))| {
                    !one.is_empty()
                        && is_match(
                            cached_name,
                            cached_sub.as_deref(),
                            &one,
                            if two.is_empty() { None } else { Some(&two) },
                        )
                        || !two.is_empty()
                            && is_match(
                                cached_name,
                                cached_sub.as_deref(),
                                &two,
                                if one.is_empty() { None } else { Some(&one) },
                            )
                })
                .map(|(idx, _)| FontCacheRef {
                    font_cache: &self,
                    cache_index: idx,
                })
        }));*/

        for i in 0..=tlen {
            let one = terms[0..i].join(" ");
            let two = terms[i..tlen].join(" ");

            result_set.extend(
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
        eprintln!("elapsed: {}", t.elapsed().as_nanos());

        result_set.into_iter().collect()
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

        let mut hasher = DefaultHasher::new();
        raw_bytes.hash(&mut hasher);
        let raw_data_hash = hasher.finish();

        if let Some(p) = self.raw_data_hashes_to_paths.get(&raw_data_hash) {
            eprintln!("Data exists!");
            return Ok(self.paths_to_font_idxs.get(p).unwrap().clone());
        }

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

        let mut new_font_datas: Vec<FontCacheData> = Vec::new();

        let mut replace_font_datas: Vec<FontCacheData> = Vec::new();

        for font in file_ref.fonts() {
            if font.is_err() {
                return Err(font.err().unwrap().into());
            }
            let font = font.unwrap();

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
                .map(|ni| NamedInstanceInfo::from_font_ref(&path, &font, &ni))
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
                .collect::<HashSet<String>>()
                .into_iter()
                .collect::<SmallVec<[String; 32]>>();
            features.sort();

            let metrics = font.metrics(Size::unscaled(), LocationRef::default());

            /*if let Ok(existing) = self.find_font(&family_name, subfamily_name.as_ref()) {
                if axes.len() > existing.variation_axes().len()
                    || (axes.len() == existing.variation_axes().len()
                        && features.len() > existing.features().len())
                    || (axes.len() == existing.variation_axes().len()
                        && features.len() == existing.features().len()
                        && named_instances.len() > existing.named_instances().len())
                    || (axes.len() == existing.variation_axes().len()
                        && features.len() == existing.features().len()
                        && named_instances.len() == existing.named_instances().len()
                        && font_revision > *existing.revision())
                {
                    replace_idxs.push(existing.cache_index);
                    replace_font_ref_idxs.push(font_ref_idx);
                    replace_raw_data_ranges.push(raw_data_range.clone());
                    replace_family_names.push(family_name);
                    replace_subfamily_names.push(subfamily_name);
                    replace_revisions.push(font_revision);
                    replace_variation_axes.push(axes);
                    replace_named_instances.push(named_instances);
                    replace_features.push(features);
                    replace_unscaled_default_metrics.push(metrics);
                } else {
                    continue;
                }
            } else {*/

            new_font_datas.push(FontCacheData {
                font_ref_idx,
                raw_data_range: raw_data_range.clone(),
                family_name,
                subfamily_name,
                revision: font_revision,
                unscaled_default_metrics: metrics,
                variation_axes: axes,
                named_instances,
                features,
            });
            //};

            font_ref_idx += 1;
        }

        let fonts_start_index: usize = self.font_datas.len();
        let fonts_end_index: usize = fonts_start_index + new_font_datas.len();

        let new_font_idxs = fonts_start_index..fonts_end_index;

        let font_datas_extended_len = self.font_datas.len() + new_font_datas.len();

        debug_assert_eq!(
            self.paths.len(),
            self.font_file_types.len(),
            "{}",
            path.as_ref().to_string_lossy()
        );
        debug_assert_eq!(
            self.paths.len(),
            self.paths_to_font_idxs.len(),
            "{}",
            path.as_ref().to_string_lossy()
        );
        debug_assert_eq!(
            self.paths.len(),
            self.raw_data_hashes_to_paths.len(),
            "{}",
            path.as_ref().to_string_lossy()
        );
        debug_assert_eq!(
            font_datas_extended_len,
            self.paths_to_font_idxs
                .values()
                .map(std::ops::Range::len)
                .sum::<usize>()
                + new_font_idxs.len(),
            "{}",
            path.as_ref().to_string_lossy()
        );

        debug_assert_eq!(
            // one data_range per path
            self.paths_to_font_idxs
                .iter()
                .map(|(_, r)| &self.font_datas[r.start].raw_data_range)
                .map(|r| r.len())
                .sum::<usize>()
                + raw_data_range.len(),
            self.all_raw_data.len() + &raw_bytes.len(),
            "{}",
            path.as_ref().to_string_lossy()
        );

        let path: PathBuf = path.as_ref().into();

        self.all_raw_data.extend(raw_bytes);
        self.paths.push(path.clone());
        self.raw_data_hashes_to_paths
            .insert(raw_data_hash, path.clone());
        self.font_file_types.push(font_file_type);

        self.paths_to_font_idxs.insert(path, new_font_idxs);

        /*for idx in replace_idxs {
            let old_paths: Vec<&PathBuf> = self
                .paths_to_font_idxs
                .iter()
                .filter_map(|(p, is)| is.contains(&idx).then_some(p))
                .collect();
            assert_eq!(
                1,
                old_paths.len(),
                "font at cache index {} is linked to multiple (or no) file paths: [{}]",
                idx,
                old_paths
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .reduce(|acc, el| format!("{}; \"{}\"", acc, el).into())
                    .unwrap_or("NO PATHS".into())
            );
            let old_path = old_paths[0];

            self.paths_to_font_idxs.get_mut(old_path).unwrap().
        }*/

        self.font_datas.extend(new_font_datas);

        Ok(fonts_start_index..fonts_end_index)
    }
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
