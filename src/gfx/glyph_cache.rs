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

pub struct GlyphCache {
    pub font_path: std::path::PathBuf,
    pub cached_chars: Vec<(char, ab_glyph::PxScale)>,
    pub cached_px_bounds: Vec<GlyphPxBounds>,
    pub cached_uv_bounds: Vec<cgmath::Matrix2<f32>>,
    max_x_assigned: usize,
    max_y_assigned: usize,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group: wgpu::BindGroup,
}

impl GlyphCache {
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

        let a_glyph: ab_glyph::Glyph = font.glyph_id('a').with_scale(px_scale);

        let mut cached_chars: Vec<(char, ab_glyph::PxScale)> = Vec::new();
        let mut cached_uv_bounds: Vec<cgmath::Matrix2<f32>> = Vec::new();
        let mut cached_px_bounds: Vec<GlyphPxBounds> = Vec::new();

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

        let mut texture_data: Vec<u8> = vec![0; (texture_row_size * texture_rows) as usize];

        let mut current_pixel_offset_x: usize = 0;
        let mut current_pixel_offset_y: usize = 0;

        let mut max_y_assigned: usize = 0;
        let mut max_x_assigned: usize = 0;

        if let Some(a) = font.outline_glyph(a_glyph) {
            let px_bounds = a.px_bounds();
            eprintln!("a.px_bounds: {:?}", px_bounds);
            let texture_offset_u: f32 = current_pixel_offset_x as f32 / texture_row_size as f32;
            let texture_offset_v: f32 = current_pixel_offset_y as f32 / texture_rows as f32;
            let px_width = px_bounds.max.x - px_bounds.min.x;
            let px_height = px_bounds.max.y - px_bounds.min.y;
            cached_chars.push(('a', px_scale));
            cached_px_bounds.push(px_bounds.into());
            cached_uv_bounds.push(cgmath::Matrix2::<f32>::new(
                texture_offset_u,
                texture_offset_v,
                texture_offset_u + px_width / texture_row_size as f32,
                texture_offset_v + px_height / texture_rows as f32,
            ));

            a.draw(|x, y, c| {
                max_x_assigned = std::cmp::max(max_x_assigned, x as usize);
                max_y_assigned = std::cmp::max(max_y_assigned, y as usize);
                let idx = y as usize * texture_row_size + x as usize;
                texture_data[idx] = (c * 255.0) as u8;
            });

            current_pixel_offset_x = max_x_assigned + 1;
        }

        let b_glyph: ab_glyph::Glyph = font.glyph_id('b').with_scale(px_scale);

        if let Some(b) = font.outline_glyph(b_glyph) {
            let px_bounds = b.px_bounds();
            eprintln!("b.px_bounds: {:?}", px_bounds);
            let px_width = px_bounds.max.x - px_bounds.min.x;
            let px_height = px_bounds.max.y - px_bounds.min.y;

            if current_pixel_offset_x + px_width.ceil() as usize > texture_row_size {
                current_pixel_offset_x = 0;
                max_x_assigned = 0;
                current_pixel_offset_y = max_y_assigned + 1;
            }

            eprintln!(
                "a - max_x_assigned: {}, max_y_assigned: {}, current_pixel_offset_x: {}, current_pixel_offset_y: {}", 
                max_x_assigned, max_y_assigned, current_pixel_offset_x, current_pixel_offset_y
            );

            let texture_offset_u: f32 = current_pixel_offset_x as f32 / texture_row_size as f32;
            let texture_offset_v: f32 = current_pixel_offset_y as f32 / texture_rows as f32;

            cached_chars.push(('b', px_scale));
            cached_px_bounds.push(px_bounds.into());
            cached_uv_bounds.push(cgmath::Matrix2::<f32>::new(
                texture_offset_u,
                texture_offset_v,
                texture_offset_u + px_width / texture_row_size as f32,
                texture_offset_v + px_height / texture_rows as f32,
            ));

            b.draw(|x, y, c| {
                let offset_x = x as usize + current_pixel_offset_x;
                let offset_y = y as usize + current_pixel_offset_y;
                max_x_assigned = std::cmp::max(max_x_assigned, offset_x);
                max_y_assigned = std::cmp::max(max_y_assigned, offset_y);
                let idx = offset_y * texture_row_size + offset_x;
                texture_data[idx] = (c * 255.0) as u8;
            });

            current_pixel_offset_x = max_x_assigned + 1;

            eprintln!(
                "b - max_x_assigned: {}, max_y_assigned: {}, current_pixel_offset_x: {}, current_pixel_offset_y: {}", 
                max_x_assigned, max_y_assigned, current_pixel_offset_x, current_pixel_offset_y
            );
        }

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
            cached_chars,
            cached_px_bounds,
            cached_uv_bounds,
            max_x_assigned,
            max_y_assigned,
            texture,
            view,
            sampler,
            texture_bind_group_layout,
            texture_bind_group,
        }
    }
}
