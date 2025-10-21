use std::ffi::OsStr;
use std::path::PathBuf;
use std::str::FromStr;
use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use harfrust::{
    Feature, GlyphBuffer, Language, Shaper, ShaperData, ShaperInstance, UnicodeBuffer, Variation,
};
use smallvec::SmallVec;
use thiserror::Error;

use skrifa::raw::ReadError;
use skrifa::string::StringId;
use skrifa::{Axis, prelude::*};

#[derive(Debug, Error)]
enum FontError {
    #[error(
        "invalid font file extension ({0}) - accepted extensions are .ttf, .otf, .ttc, and .otc"
    )]
    FileExtension(String),
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
                Some(ext) => Err(FontError::FileExtension(ext.into()).into()),
                None => Err(FontError::FileExtension("file has no extension".to_string()).into()),
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

pub struct FontCache {
    paths: Vec<PathBuf>,
    paths_to_raw_data_idx: HashMap<PathBuf, usize>,
    /*names: Vec<String>,
    names_to_idx: HashMap<String, usize>,*/
    font_file_types: Vec<FontFileType>,
    all_raw_data: Vec<u8>,
    raw_data_ranges: Vec<std::ops::Range<usize>>,

    full_name_to_font_idx: HashMap<String, FontIndex>,
    font_families: Vec<String>,
    font_family_to_idx: HashMap<String, SmallVec<[usize; 8]>>,
}

impl FontCache {
    pub fn new() -> Self {
        Self {
            paths: Vec::new(),
            paths_to_raw_data_idx: HashMap::new(),
            font_file_types: Vec::new(),
            all_raw_data: Vec::new(),
            raw_data_ranges: Vec::new(),

            full_name_to_font_idx: HashMap::new(),
            font_families: Vec::new(),
            font_family_to_idx: HashMap::new(),
        }
    }

    pub fn load_font_file<P: Into<PathBuf>>(&mut self, path: P) -> Result<FontRef<'_>> {
        let path: PathBuf = path.into();
        let index = match self.paths_to_raw_data_idx.get(&path) {
            Some(idx) => *idx,
            None => self.cache_raw_data(&path)?,
        };

        self.font_ref_by_font_index(index, None)
        /*let data_range = self.raw_data_ranges[index].clone();
        match self.font_file_types[index] {
            FontFileType::Single => Ok(FontRef::new(&self.all_raw_data[data_range])?),
            FontFileType::Collection => Ok(FontRef::from_index(&self.all_raw_data[data_range], 0)?),
        }*/
    }

    fn font_ref_by_font_index(
        &self,
        index: usize,
        collection_index: Option<u32>,
    ) -> Result<FontRef<'_>> {
        let data_range = self.raw_data_ranges[index].clone();
        match self.font_file_types[index] {
            FontFileType::Single => Ok(FontRef::new(&self.all_raw_data[data_range])?),
            FontFileType::Collection => Ok(FontRef::from_index(
                &self.all_raw_data[data_range],
                collection_index.unwrap_or(0),
            )?),
        }
    }

    fn cache_raw_data<P: AsRef<Path>>(&mut self, path: P) -> Result<usize> {
        let font_file_type = FontFileType::from_path(&path)?;
        let raw_bytes = std::fs::read(&path).with_context(|| {
            format!(
                "unable to read font file at path: {}",
                path.as_ref().display()
            )
        })?;

        let raw_data_len = raw_bytes.len();
        let start_index = self.all_raw_data.len();
        self.all_raw_data.extend(&raw_bytes);
        let end_index = self.all_raw_data.len();
        assert_eq!(
            end_index - start_index,
            raw_data_len,
            "font cache: the calculated range is not equal to the length of the inserted data"
        );

        let file_ref: skrifa::raw::FileRef = skrifa::raw::FileRef::new(&raw_bytes)?;

        for font in file_ref.fonts() {
            if font.is_err() {
                return Err(font.err().unwrap().into());
            }
            let font = font.unwrap();

            let full_name = font
                .localized_strings(skrifa::string::StringId::FULL_NAME)
                .english_or_first()
                .map_or(String::new(), |l| l.to_string());
            eprintln!("full_name: {}", full_name);

            let subfamily_name = font
                .localized_strings(skrifa::string::StringId::SUBFAMILY_NAME)
                .english_or_first()
                .map_or(String::new(), |l| l.to_string());
            eprintln!("subfamily_name: {}", subfamily_name);

            let axes: SmallVec<[Axis; 4]> = font.axes().iter().collect();
            eprintln!(
                "axes: {}",
                axes.iter()
                    .map(|a| a.tag().to_string())
                    .reduce(|acc, e| format!("{}, {}", acc, e))
                    .unwrap_or(String::default())
            );
            let named_instances: Vec<String> = font
                .named_instances()
                .iter()
                .map(|ni| {
                    font.localized_strings(ni.subfamily_name_id())
                        .english_or_first()
                        .map_or(String::default(), |l| l.to_string())
                })
                .collect();
            eprintln!("named_instances: {}", named_instances.join(", "));
        }

        let index: usize = self.paths.len();

        let path: PathBuf = path.as_ref().into();
        self.paths.push(path.clone());
        self.paths_to_raw_data_idx.insert(path, index);
        self.font_file_types.push(font_file_type);
        self.raw_data_ranges.push(start_index..end_index);

        let paths_len = self.paths.len();

        assert_eq!(
            paths_len,
            self.paths_to_raw_data_idx.len(),
            "font cache: cache structure lengths not equal (path_to_idx)"
        );
        assert_eq!(
            paths_len,
            self.font_file_types.len(),
            "font cache: cache structure lengths not equal (font_file_types)"
        );
        assert_eq!(
            paths_len,
            self.raw_data_ranges.len(),
            "font cache: cache structure lengths not equal (raw_data_ranges)"
        );

        Ok(index)
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
