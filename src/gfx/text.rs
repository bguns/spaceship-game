use std::path::PathBuf;
use std::str::FromStr;

use harfrust::{
    Feature, FontRef, GlyphBuffer, Language, Shaper, ShaperData, ShaperInstance, UnicodeBuffer,
    Variation,
};

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
