use super::vertex::Vertex;
use ab_glyph::{Font, ScaleFont};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPxBounds {
    pub min: cgmath::Point2<f32>,
    pub max: cgmath::Point2<f32>,
}

impl From<ab_glyph::Rect> for GlyphPxBounds {
    fn from(rect: ab_glyph::Rect) -> Self {
        // ab_glyph assumes opengl coordinates (0, 0 top left),
        // but wgpu uses DX11/Metal coordinates (0, 0 center),
        // so y axis needs to invert bounds
        Self {
            min: cgmath::Point2 {
                x: rect.min.x,
                y: -rect.min.y,
            },
            max: cgmath::Point2 {
                x: rect.max.x,
                y: -rect.max.y,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPxScale {
    pub x: f32,
    pub y: f32,
    screen_scale_factor: f32,
}

impl GlyphPxScale {
    fn to_ab_glyph_px_scale(&self) -> ab_glyph::PxScale {
        ab_glyph::PxScale {
            x: self.x * self.screen_scale_factor,
            y: self.y * self.screen_scale_factor,
        }
    }
}

/*impl From<ab_glyph::PxScale> for GlyphPxScale {
    fn from(px_scale: ab_glyph::PxScale) -> Self {
        Self {
            x: px_scale.x,
            y: px_scale.y,
        }
    }
}*/

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphUvBounds {
    uv_bounds: cgmath::Matrix2<f32>,
}

impl GlyphUvBounds {
    pub fn new(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            uv_bounds: cgmath::Matrix2::<f32>::new(left, top, right, bottom),
        }
    }

    pub fn top(&self) -> f32 {
        self.uv_bounds.x.y
    }

    pub fn bottom(&self) -> f32 {
        self.uv_bounds.y.y
    }

    pub fn left(&self) -> f32 {
        self.uv_bounds.x.x
    }

    pub fn right(&self) -> f32 {
        self.uv_bounds.y.x
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphData {
    character: char,
    font_idx: usize,
    px_scale: GlyphPxScale,
    px_bounds: GlyphPxBounds,
    uv_bounds: GlyphUvBounds,
}

impl GlyphData {
    pub fn px_scale(&self) -> &GlyphPxScale {
        &self.px_scale
    }
}

#[derive(Debug)]
pub struct FontData {
    path: std::path::PathBuf,
    name: String,
    font: ab_glyph::FontVec,
}

pub struct GlyphCache {
    surface_width: u32,
    surface_height: u32,
    screen_scale_factor: f32,
    pub cached_fonts: Vec<FontData>,
    pub cached_glyphs: Vec<GlyphData>,
    texture_row_size: usize,
    texture_rows: usize,
    current_px_offset: cgmath::Point2<usize>,
    max_x_assigned: usize,
    max_y_assigned: usize,
    texture_data: Vec<u8>,
    texture_data_dirty: bool,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group: wgpu::BindGroup,
}

impl GlyphCache {
    pub fn new(
        device: &wgpu::Device,
        surface_width: u32,
        surface_height: u32,
        screen_scale_factor: f32,
    ) -> Self {
        let label = Some("glyph_cache_texture");
        let px_scale = ab_glyph::PxScale {
            x: 64.0 * screen_scale_factor,
            y: 64.0 * screen_scale_factor,
        };

        let cached_fonts: Vec<FontData> = Vec::new();
        let cached_glyphs: Vec<GlyphData> = Vec::new();

        let base_row_size = px_scale.x.ceil() as usize * 32;
        let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        // Texture is R8Unorm i.e. one byte per pixel.
        let texture_row_size = std::cmp::min(
            base_row_size + ((alignment - (base_row_size % alignment)) % alignment),
            device.limits().max_texture_dimension_2d as usize,
        );
        eprintln!(
            "base_row_size: {}, alignment: {}, texture_row_size: {}",
            base_row_size, alignment, texture_row_size
        );

        let texture_rows = std::cmp::min(
            px_scale.y.ceil() as usize * 32,
            device.limits().max_texture_dimension_2d as usize,
        );

        let texture_data: Vec<u8> = vec![0; (texture_row_size * texture_rows) as usize];

        let size = wgpu::Extent3d {
            width: texture_row_size as u32,
            height: texture_rows as u32,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("glyph_cache_texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph_cache_texture_bind_group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Self {
            surface_width,
            surface_height,
            screen_scale_factor,
            cached_fonts,
            cached_glyphs,
            current_px_offset: cgmath::Point2 { x: 0, y: 0 },
            max_x_assigned: 0,
            max_y_assigned: 0,
            texture_row_size,
            texture_rows,
            texture_data,
            texture_data_dirty: false,
            texture,
            view,
            sampler,
            texture_bind_group_layout,
            texture_bind_group,
        }
    }

    pub fn surface_resized(&mut self, surface_width: u32, surface_height: u32) {
        self.surface_width = surface_width;
        self.surface_height = surface_height;
    }

    pub fn glyph_px_scale(&self, uniform_scale: f32) -> GlyphPxScale {
        GlyphPxScale {
            x: uniform_scale,
            y: uniform_scale,
            screen_scale_factor: self.screen_scale_factor,
        }
    }

    pub fn cache_font(&mut self, font_path: std::path::PathBuf) -> usize {
        if let Some(idx) = self.get_font_id_for_font_path(&font_path) {
            return idx;
        } else {
            let font_name = font_path
                .file_stem()
                .expect("Unable to extract file stem from font_path")
                .to_str()
                .unwrap()
                .to_string();
            let font_bytes = std::fs::read(&font_path).expect("Unable to read font file.");
            let font = ab_glyph::FontVec::try_from_vec_and_index(font_bytes, 0)
                .expect("Unable to load font.");

            let font_data = FontData {
                path: font_path,
                name: font_name,
                font,
            };
            self.cached_fonts.push(font_data);
            self.cached_fonts.len() - 1
        }
    }

    pub fn prepare_draw_for_glyph(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
        glyph: &GlyphData,
        caret_x: f32,
        caret_y: f32,
    ) {
        // Glyphs are already scaled to scale_factor in the texture cache, don'te rescale here.
        let surface_width_px = self.surface_width as f32;
        let surface_height_px = self.surface_height as f32;

        let uv_bounds = glyph.uv_bounds;
        let px_bounds = glyph.px_bounds;

        let left = caret_x + px_bounds.min.x / surface_width_px;
        let right = caret_x + px_bounds.max.x / surface_width_px;
        let top = caret_y + px_bounds.min.y / surface_height_px;
        let bottom = caret_y + px_bounds.max.y / surface_height_px;

        let previous_vertices_len = vertices.len() as u16;

        vertices.push(Vertex {
            position: [left, top, 0.0],
            tex_coords: [uv_bounds.left(), uv_bounds.top()],
        });
        vertices.push(Vertex {
            position: [left, bottom, 0.0],
            tex_coords: [uv_bounds.left(), uv_bounds.bottom()],
        });
        vertices.push(Vertex {
            position: [right, bottom, 0.0],
            tex_coords: [uv_bounds.right(), uv_bounds.bottom()],
        });
        vertices.push(Vertex {
            position: [right, top, 0.0],
            tex_coords: [uv_bounds.right(), uv_bounds.top()],
        });

        indices.push(0 + previous_vertices_len);
        indices.push(1 + previous_vertices_len);
        indices.push(2 + previous_vertices_len);
        indices.push(2 + previous_vertices_len);
        indices.push(3 + previous_vertices_len);
        indices.push(0 + previous_vertices_len);
    }

    pub fn prepare_draw_for_text(
        &mut self,
        text: &str,
        font_idx: usize,
        px_scale: GlyphPxScale,
        caret_x: &mut f32,
        caret_y: &mut f32,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u16>,
    ) {
        for c in text.chars() {
            self.ensure_glyph_cached(font_idx, c, px_scale);
        }
        let scaled_font = self
            .try_get_cached_font_with_scale(font_idx, px_scale)
            .expect(&format!("Unable to find cached font with idx {}", font_idx));
        let mut previous_char: Option<char> = None;
        for c in text.chars() {
            if let Some(glyph_data) = self.try_get_cached_glyph_data(font_idx, c, px_scale) {
                if let Some(prev) = previous_char {
                    *caret_x += scaled_font
                        .kern(scaled_font.glyph_id(prev), scaled_font.glyph_id(c))
                        / self.surface_width as f32;
                }
                self.prepare_draw_for_glyph(vertices, indices, glyph_data, *caret_x, *caret_y);

                *caret_x +=
                    scaled_font.h_advance(scaled_font.glyph_id(c)) / self.surface_width as f32;
                previous_char = Some(c);
            } else {
                *caret_x +=
                    scaled_font.h_advance(scaled_font.glyph_id(' ')) / self.surface_width as f32;
            }
        }
    }

    fn get_font_id_for_font_path(&self, font_path: &std::path::PathBuf) -> Option<usize> {
        for (idx, font) in self.cached_fonts.iter().enumerate() {
            if font.path == *font_path {
                return Some(idx);
            }
        }
        None
    }

    fn _get_font_id_for_font_name(&self, font_name: &str) -> Option<usize> {
        for (idx, font) in self.cached_fonts.iter().enumerate() {
            if &font.name == font_name {
                return Some(idx);
            }
        }
        None
    }

    pub fn try_get_cached_font_with_scale(
        &self,
        font_idx: usize,
        px_scale: GlyphPxScale,
    ) -> Option<ab_glyph::PxScaleFont<&ab_glyph::FontVec>> {
        if let Some(font_data) = self.cached_fonts.get(font_idx) {
            Some(font_data.font.as_scaled(px_scale.to_ab_glyph_px_scale()))
        } else {
            None
        }
    }

    pub fn try_get_cached_glyph_data(
        &self,
        font_idx: usize,
        character: char,
        px_scale: GlyphPxScale,
    ) -> Option<&GlyphData> {
        for glyph in &self.cached_glyphs {
            if glyph.font_idx == font_idx
                && glyph.character == character
                && glyph.px_scale == px_scale
            {
                return Some(&glyph);
            }
        }
        None
    }

    pub fn queue_write_texture_if_changed(&mut self, queue: &wgpu::Queue) {
        if self.texture_data_dirty {
            eprintln!("Writing texture data to texture.");
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.texture_data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: std::num::NonZeroU32::new(self.texture_row_size as u32),
                    rows_per_image: std::num::NonZeroU32::new(self.texture_rows as u32),
                },
                wgpu::Extent3d {
                    width: self.texture_row_size as u32,
                    height: self.texture_rows as u32,
                    depth_or_array_layers: 1,
                },
            );
            self.texture_data_dirty = false;
        }
    }

    pub fn ensure_glyph_cached(
        &mut self,
        font_idx: usize,
        character: char,
        px_scale: GlyphPxScale,
    ) {
        if let None = self.try_get_cached_glyph_data(font_idx, character, px_scale) {
            let font = &self.cached_fonts[font_idx].font;
            let glyph = font
                .glyph_id(character)
                .with_scale(px_scale.to_ab_glyph_px_scale());
            if let Some(g) = font.outline_glyph(glyph) {
                let px_bounds = g.px_bounds();
                let px_width = px_bounds.max.x - px_bounds.min.x;
                let px_height = px_bounds.max.y - px_bounds.min.y;
                eprintln!("{}.px_bounds: {:?}", character, px_bounds);

                if self.current_px_offset.x + px_width.ceil() as usize > self.texture_row_size {
                    self.current_px_offset.x = 0;
                    self.current_px_offset.y = self.max_y_assigned + 1;
                    self.max_x_assigned = 0;
                }
                eprintln!(
                    "current_px_offset.x: {}; current_px_offset.y: {}",
                    self.current_px_offset.x, self.current_px_offset.y
                );

                let texture_offset_u: f32 =
                    self.current_px_offset.x as f32 / self.texture_row_size as f32;
                let texture_offset_v: f32 =
                    self.current_px_offset.y as f32 / self.texture_rows as f32;
                eprintln!(
                    "texture_offset_u: {}; texture_offset_v: {}",
                    texture_offset_u, texture_offset_v
                );

                let glyph_data = GlyphData {
                    character,
                    font_idx,
                    px_scale,
                    px_bounds: px_bounds.into(),
                    uv_bounds: GlyphUvBounds::new(
                        texture_offset_u,
                        texture_offset_u + px_width / self.texture_row_size as f32,
                        texture_offset_v,
                        texture_offset_v + px_height / self.texture_rows as f32,
                    ),
                };

                self.cached_glyphs.push(glyph_data);

                g.draw(|x, y, c| {
                    let offset_x = x as usize + self.current_px_offset.x;
                    let offset_y = y as usize + self.current_px_offset.y;
                    self.max_x_assigned = std::cmp::max(self.max_x_assigned, offset_x as usize);
                    self.max_y_assigned = std::cmp::max(self.max_y_assigned, offset_y as usize);
                    let idx = offset_y * self.texture_row_size + offset_x;
                    self.texture_data[idx] = (c * 255.0) as u8;
                });

                self.current_px_offset.x = self.max_x_assigned + 1;

                self.texture_data_dirty = true;
            }
        }
    }
}
