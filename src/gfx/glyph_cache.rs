use ab_glyph::Font;

pub struct GlyphPxBounds {
    pub min: cgmath::Point2<f32>,
    pub max: cgmath::Point2<f32>,
}

impl From<ab_glyph::Rect> for GlyphPxBounds {
    fn from(rect: ab_glyph::Rect) -> Self {
        Self {
            min: cgmath::Point2 {
                x: rect.min.x,
                y: rect.min.y,
            },
            max: cgmath::Point2 {
                x: rect.max.x,
                y: rect.max.y,
            },
        }
    }
}

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

pub struct GlyphBounds {
    pub px_bounds: GlyphPxBounds,
    pub uv_bounds: GlyphUvBounds,
}

pub struct GlyphCache<'a> {
    pub font_path: std::path::PathBuf,
    pub font: ab_glyph::FontRef<'a>,
    pub cached_chars: Vec<(char, ab_glyph::PxScale)>,
    pub cached_px_bounds: Vec<GlyphPxBounds>,
    pub cached_uv_bounds: Vec<GlyphUvBounds>,
    texture_row_size: usize,
    texture_rows: usize,
    current_px_offset: cgmath::Point2<usize>,
    max_x_assigned: usize,
    max_y_assigned: usize,
    texture_data: Vec<u8>,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group: wgpu::BindGroup,
}

impl<'a> GlyphCache<'a> {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        initial_px_scale: ab_glyph::PxScale,
        window_scale_factor: f32,
    ) -> Self {
        let label = Some("glyph_cache_texture");
        let px_scale = ab_glyph::PxScale {
            x: initial_px_scale.x * window_scale_factor,
            y: initial_px_scale.y * window_scale_factor,
        };

        let font_path = std::path::PathBuf::from("../fonts/wqy-microhei/WenQuanYiMicroHei.ttf");
        let font_data = include_bytes!("../../fonts/wqy-microhei/WenQuanYiMicroHei.ttf");
        let font = ab_glyph::FontRef::try_from_slice(font_data).expect("Unable to load font.");

        let cached_chars: Vec<(char, ab_glyph::PxScale)> = Vec::new();
        let cached_uv_bounds: Vec<GlyphUvBounds> = Vec::new();
        let cached_px_bounds: Vec<GlyphPxBounds> = Vec::new();

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

        let current_pixel_offset_x: usize = 0;
        let current_pixel_offset_y: usize = 0;

        let max_y_assigned: usize = 0;
        let max_x_assigned: usize = 0;

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

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texture_data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(texture_row_size as u32),
                rows_per_image: std::num::NonZeroU32::new(texture_rows as u32),
            },
            size,
        );

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
            font_path,
            font,
            cached_chars,
            cached_px_bounds,
            cached_uv_bounds,
            current_px_offset: cgmath::Point2 {
                x: current_pixel_offset_x,
                y: current_pixel_offset_y,
            },
            max_x_assigned,
            max_y_assigned,
            texture_row_size,
            texture_rows,
            texture_data,
            texture,
            view,
            sampler,
            texture_bind_group_layout,
            texture_bind_group,
        }
    }

    pub fn queue_write_texture_if_changed(&mut self, queue: &wgpu::Queue) {
        // if (changed) {
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
    }

    pub fn prepare_cache_glyph(&mut self, character: char, px_scale: ab_glyph::PxScale) {
        let glyph = self.font.glyph_id(character).with_scale(px_scale);
        if let Some(g) = self.font.outline_glyph(glyph) {
            let px_bounds = g.px_bounds();
            let px_width = px_bounds.max.x - px_bounds.min.x;
            let px_height = px_bounds.max.y - px_bounds.min.y;
            eprintln!("{}.px_bounds: {:?}", character, px_bounds);

            if self.current_px_offset.x + px_width.ceil() as usize > self.texture_row_size {
                self.current_px_offset.x = 0;
                self.current_px_offset.y = self.max_y_assigned + 1;
                self.max_x_assigned = 0;
            }

            let texture_offset_u: f32 =
                self.current_px_offset.x as f32 / self.texture_row_size as f32;
            let texture_offset_v: f32 = self.current_px_offset.y as f32 / self.texture_rows as f32;

            self.cached_chars.push((character, px_scale));
            self.cached_px_bounds.push(px_bounds.into());
            self.cached_uv_bounds.push(GlyphUvBounds::new(
                texture_offset_u,
                texture_offset_u + px_width / self.texture_row_size as f32,
                texture_offset_v,
                texture_offset_v + px_height / self.texture_rows as f32,
            ));

            g.draw(|x, y, c| {
                let offset_x = x as usize + self.current_px_offset.x;
                let offset_y = y as usize + self.current_px_offset.y;
                self.max_x_assigned = std::cmp::max(self.max_x_assigned, x as usize);
                self.max_y_assigned = std::cmp::max(self.max_y_assigned, y as usize);
                let idx = offset_y * self.texture_row_size + offset_x;
                self.texture_data[idx] = (c * 255.0) as u8;
            });

            self.current_px_offset.x = self.max_x_assigned + 1;
        }
    }
}
