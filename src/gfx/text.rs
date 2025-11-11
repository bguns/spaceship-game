use std::ffi::OsStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, LazyLock, OnceLock};
use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use harfrust::{Feature, GlyphBuffer, ShaperData, ShaperInstance, UnicodeBuffer, Variation};
use parking_lot::Mutex;
use rayon::prelude::*;
use skrifa::raw::TableProvider;
use skrifa::{
    Axis, GlyphId, MetadataProvider, OutlineGlyphCollection, font::FontRef as ExtFontRef,
};
use smallvec::SmallVec;
use thiserror::Error;
use typed_arena::Arena;
use zeno::PathBuilder;

use crate::os::font_util;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphVertex {
    pub caret_position: [f32; 3],
    pub px_bounds_offset: [f32; 2],
    pub tex_coords: [f32; 2],
}

impl GlyphVertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<GlyphVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: (size_of::<[f32; 3]>() + size_of::<[f32; 2]>()) as wgpu::BufferAddress,
                    shader_location: 2,
                },
            ],
        }
    }
}

pub struct TextRenderer {
    pub glyph_cache: GlyphCache,
    surface_width: u32,
    surface_height: u32,
    surface_scale_factor: f32,
    texture_row_size_bytes: usize,
    texture_rows: usize,
    pub texture: wgpu::Texture,
    pub texture_bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
    glyph_vertex_buffer: wgpu::Buffer,
    glyph_index_buffer: wgpu::Buffer,
}

impl TextRenderer {
    pub fn new(
        device: &wgpu::Device,
        surface_configuration: &wgpu::SurfaceConfiguration,
        surface_dimensions_bind_group_layout: &wgpu::BindGroupLayout,
        surface_width: u32,
        surface_height: u32,
        surface_scale_factor: f32,
    ) -> Self {
        // keep this simple for now, just a 2K texture
        // Note that this (probably?) needs to be aligned to wgpu::COPY_BYTES_PER_ROW_ALIGNMENT (256)
        // Using Rgba8UnormSrgb
        let texture_row_size_bytes =
            std::cmp::min(2048, device.limits().max_texture_dimension_2d as usize);
        let texture_rows = std::cmp::min(2048, device.limits().max_texture_dimension_2d as usize);

        let size = wgpu::Extent3d {
            width: (texture_row_size_bytes / 4) as u32,
            height: texture_rows as u32,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph_cache_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Glyph Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text_shader.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Glyph Render Pipeline Layout"),
                bind_group_layouts: &[
                    &surface_dimensions_bind_group_layout,
                    &texture_bind_group_layout,
                ],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                // What type of vertices we want to pass to the vertex shader.
                buffers: &[GlyphVertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_configuration.format,
                    //blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    blend: Some(wgpu::BlendState {
                        // Dual source blending
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Src1,
                            dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrc1Alpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                // Setting this to anything other than Fill requires Features::NON_FILL_POLYGON_MODE
                polygon_mode: wgpu::PolygonMode::Fill,
                // Requires Features::DEPTH_CLIP_CONTROL
                unclipped_depth: false,
                // Requires Features::CONSERVATIVE_RASTERIZATION
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let glyph_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_vertex_buffer"),
            size: (4000 as usize * std::mem::size_of::<GlyphVertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let glyph_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_index_buffer"),
            size: (6000 as usize * std::mem::size_of::<u16>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            glyph_cache: GlyphCache::new(texture_row_size_bytes, texture_rows),
            surface_width,
            surface_height,
            surface_scale_factor,
            texture_row_size_bytes,
            texture_rows,
            texture,
            texture_bind_group,
            render_pipeline,
            glyph_vertex_buffer,
            glyph_index_buffer,
        }
    }

    pub fn render(
        &mut self,
        game_state: &crate::GameState,
        mut render_pass: wgpu::RenderPass,
        surface_dimensions_bind_group: &wgpu::BindGroup,
        queue: &wgpu::Queue,
    ) {
        let ppem = 19f32 * self.surface_scale_factor;

        let font_size = skrifa::instance::Size::new(ppem);

        self.queue_write_texture_if_changed(queue);

        let font = &game_state.font_cache.search_fonts("cascadia code")[0];

        let shaper = font.shaper(ShaperSettings::new());

        let glyphs = shaper.shape("abpAj", None, Some(font_size.clone()));

        let upem = font
            .ext_font_ref()
            .metrics(font_size.clone(), skrifa::instance::LocationRef::default())
            .units_per_em;

        let a_advance = glyphs.glyph_positions()[0].x_advance as f32 * ppem / upem as f32;
        let b_advance = glyphs.glyph_positions()[1].x_advance as f32 * ppem / upem as f32;
        let p_advance = glyphs.glyph_positions()[2].x_advance as f32 * ppem / upem as f32;
        let cap_a_advance = glyphs.glyph_positions()[3].x_advance as f32 * ppem / upem as f32;

        let a_glyph_id = glyphs.glyph_infos()[0].glyph_id;

        let (a_placement, a_uv_bounds) = self.glyph_cache.get_glyph_texture_bounds(
            &font,
            a_glyph_id.into(),
            font_size,
            Default::default(),
        );

        let b_glyph_id = glyphs.glyph_infos()[1].glyph_id;

        let (b_placement, b_uv_bounds) = self.glyph_cache.get_glyph_texture_bounds(
            &font,
            b_glyph_id.into(),
            font_size,
            Default::default(),
        );

        let p_glyph_id = glyphs.glyph_infos()[2].glyph_id;

        let (p_placement, p_uv_bounds) = self.glyph_cache.get_glyph_texture_bounds(
            &font,
            p_glyph_id.into(),
            font_size,
            Default::default(),
        );

        let cap_a_glyph_id = glyphs.glyph_infos()[3].glyph_id;

        let (cap_a_placement, cap_a_uv_bounds) = self.glyph_cache.get_glyph_texture_bounds(
            &font,
            cap_a_glyph_id.into(),
            font_size,
            Default::default(),
        );

        let j_glyph_id = glyphs.glyph_infos()[4].glyph_id;

        let (j_placement, j_uv_bounds) = self.glyph_cache.get_glyph_texture_bounds(
            &font,
            j_glyph_id.into(),
            font_size,
            Default::default(),
        );

        let mut glyph_vertices: Vec<GlyphVertex> = Vec::with_capacity(4000);
        let mut glyph_indices: Vec<u16> = Vec::with_capacity(6000);

        self.glyph_cache.prepare_draw_for_glyph(
            &mut glyph_vertices,
            &mut glyph_indices,
            (&a_uv_bounds).into(),
            -1.0 + super::logical_px_to_screen_surface_offset(
                256,
                self.surface_width,
                self.surface_scale_factor,
            ),
            1.0 - super::logical_px_to_screen_surface_offset(
                256 + (a_placement.height as i16 - a_placement.top as i16),
                self.surface_height,
                self.surface_scale_factor,
            ),
        );
        self.glyph_cache.prepare_draw_for_glyph(
            &mut glyph_vertices,
            &mut glyph_indices,
            (&b_uv_bounds).into(),
            -1.0 + super::logical_px_to_screen_surface_offset(
                256,
                self.surface_width,
                self.surface_scale_factor,
            ) + super::logical_px_to_screen_surface_offset(
                a_advance.floor() as i16,
                self.surface_width,
                self.surface_scale_factor,
            ),
            1.0 - super::logical_px_to_screen_surface_offset(
                256 + (b_placement.height as i16 - b_placement.top as i16),
                self.surface_height,
                self.surface_scale_factor,
            ),
        );
        self.glyph_cache.prepare_draw_for_glyph(
            &mut glyph_vertices,
            &mut glyph_indices,
            (&p_uv_bounds).into(),
            -1.0 + super::logical_px_to_screen_surface_offset(
                256,
                self.surface_width,
                self.surface_scale_factor,
            ) + super::logical_px_to_screen_surface_offset(
                (a_advance + b_advance).floor() as i16,
                self.surface_width,
                self.surface_scale_factor,
            ),
            1.0 - super::logical_px_to_screen_surface_offset(
                256 + (p_placement.height as i16 - p_placement.top as i16),
                self.surface_height,
                self.surface_scale_factor,
            ),
        );
        self.glyph_cache.prepare_draw_for_glyph(
            &mut glyph_vertices,
            &mut glyph_indices,
            (&cap_a_uv_bounds).into(),
            -1.0 + super::logical_px_to_screen_surface_offset(
                256,
                self.surface_width,
                self.surface_scale_factor,
            ) + super::logical_px_to_screen_surface_offset(
                (a_advance + b_advance + p_advance).floor() as i16,
                self.surface_width,
                self.surface_scale_factor,
            ),
            1.0 - super::logical_px_to_screen_surface_offset(
                256 + (cap_a_placement.height as i16 - cap_a_placement.top as i16),
                self.surface_height,
                self.surface_scale_factor,
            ),
        );
        self.glyph_cache.prepare_draw_for_glyph(
            &mut glyph_vertices,
            &mut glyph_indices,
            (&j_uv_bounds).into(),
            -1.0 + super::logical_px_to_screen_surface_offset(
                256,
                self.surface_width,
                self.surface_scale_factor,
            ) + super::logical_px_to_screen_surface_offset(
                (a_advance + b_advance + p_advance + cap_a_advance).floor() as i16,
                self.surface_width,
                self.surface_scale_factor,
            ),
            1.0 - super::logical_px_to_screen_surface_offset(
                256 + (j_placement.height as i16 - j_placement.top as i16),
                self.surface_height,
                self.surface_scale_factor,
            ),
        );

        /*let mut caret_x = -1.0 + self.logical_px_to_horizontal_screen_space_offset(256);
        let mut caret_y = 1.0 - self.logical_px_to_vertical_screen_space_offset(512);

        if let Some(txt) = &game_state.text {
            self.glyph_cache.prepare_draw_for_text(
                txt,
                1,
                px_scale,
                &mut caret_x,
                &mut caret_y,
                &mut glyph_vertices,
                &mut glyph_indices,
            );
        }

        let fps = 1_000_000.0 / game_state.delta_time.as_micros() as f64;
        let fps_text = &format!(
            "Elapsed time: {}; Runtime: {}; dt: {:.2}, State number: {}, Frame number: {}; FPS: {:.2}",
            game_state.start_time.elapsed().as_millis(),
            game_state.run_time.as_millis(),
            game_state.delta_time.as_micros() as f64 / 1_000.0,
            game_state.state_number,
            game_state.frame_number,
            fps
        );

        let px_scale = self.glyph_cache.create_glyph_px_scale(32.0);

        caret_x = -1.0 + self.logical_px_to_horizontal_screen_space_offset(64);
        caret_y = 1.0 - self.logical_px_to_vertical_screen_space_offset(64);

        self.glyph_cache.prepare_draw_for_text(
            fps_text,
            0,
            px_scale,
            &mut caret_x,
            &mut caret_y,
            &mut glyph_vertices,
            &mut glyph_indices,
        );*/

        let old_vertices_len = glyph_vertices.len() as u16;

        let scale = self.surface_height as f32 / self.texture.size().height as f32;

        glyph_vertices.append(&mut vec![
            GlyphVertex {
                caret_position: [0.0, 0.0, 0.0],
                px_bounds_offset: [0.0, 0.0],
                tex_coords: [0.0, 0.0],
            },
            GlyphVertex {
                caret_position: [0.0, -1.0, 0.0],
                px_bounds_offset: [0.0, 0.0],
                tex_coords: [0.0, 2048.0],
            },
            GlyphVertex {
                caret_position: [
                    0.0 + super::logical_px_to_screen_surface_offset(
                        (512.0 * scale).floor() as i16,
                        self.surface_width,
                        self.surface_scale_factor,
                    ),
                    -1.0,
                    0.0,
                ],
                px_bounds_offset: [0.0, 0.0],
                tex_coords: [512.0, 2048.0],
            },
            GlyphVertex {
                caret_position: [
                    0.0 + super::logical_px_to_screen_surface_offset(
                        (512.0 * scale).floor() as i16,
                        self.surface_width,
                        self.surface_scale_factor,
                    ),
                    0.0,
                    0.0,
                ],
                px_bounds_offset: [0.0, 0.0],
                tex_coords: [512.0, 0.0],
            },
        ]);

        glyph_indices.append(&mut vec![
            0 + old_vertices_len,
            1 + old_vertices_len,
            2 + old_vertices_len,
            2 + old_vertices_len,
            3 + old_vertices_len,
            0 + old_vertices_len,
        ]);

        queue.write_buffer(
            &self.glyph_vertex_buffer,
            0,
            bytemuck::cast_slice(&glyph_vertices),
        );

        queue.write_buffer(
            &self.glyph_index_buffer,
            0,
            bytemuck::cast_slice(&glyph_indices),
        );

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, surface_dimensions_bind_group, &[]);
        render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.glyph_vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.glyph_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..glyph_indices.len() as u32, 0, 0..1);
    }

    pub fn surface_resized(&mut self, surface_width: u32, surface_height: u32, _scale_factor: f32) {
        self.surface_width = surface_width;
        self.surface_height = surface_height;
    }

    pub fn queue_write_texture_if_changed(&mut self, queue: &wgpu::Queue) {
        if self.glyph_cache.texture_data_dirty {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.glyph_cache.texture,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.texture_row_size_bytes as u32),
                    rows_per_image: Some(self.texture_rows as u32),
                },
                wgpu::Extent3d {
                    width: (self.texture_row_size_bytes / 4) as u32,
                    height: self.texture_rows as u32,
                    depth_or_array_layers: 1,
                },
            );
            self.glyph_cache.texture_data_dirty = false;
        }
    }
}

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
        font_ref: &ExtFontRef<'_>,
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
pub struct FontRef<'a> {
    font_cache: &'a FontCache,
    cache_index: usize,
    font_data: &'a FontCacheData,
    lazy_font_data: &'a LazyFontCacheData,
}

impl<'a> FontRef<'a> {
    pub fn _full_name(&self) -> String {
        format!(
            "{}{}",
            &self.font_data.family_name,
            if let Some(sf) = self.font_data.subfamily_name.as_deref() {
                &format!(" - {}", sf)
            } else {
                ""
            }
        )
    }

    pub fn family_name(&self) -> &str {
        &self.font_data.family_name
    }

    pub fn subfamily_name(&self) -> Option<&str> {
        self.font_data.subfamily_name.as_deref()
    }

    pub fn variation_axes(&self) -> &[Axis] {
        &self.font_data.variation_axes
    }

    pub fn named_instances(&self) -> &[NamedInstanceInfo] {
        &self.font_data.named_instances
    }

    pub fn features(&self) -> &[String] {
        &self.font_data.features
    }

    fn revision(&self) -> &skrifa::raw::types::Fixed {
        &self.font_cache.font_datas[self.cache_index].revision
    }

    pub fn ext_font_ref(&self) -> &ExtFontRef<'static> {
        &self
            .lazy_font_data
            .ext_font_ref(self.font_cache, self.cache_index)
    }

    fn shaper_data(&self) -> &ShaperData {
        &self
            .lazy_font_data
            .shaper_data(self.font_cache, self.cache_index)
    }

    pub fn outline_glyph_collection(&self) -> &OutlineGlyphCollection<'static> {
        &self
            .lazy_font_data
            .outline_glyph_collection(self.font_cache, self.cache_index)
    }

    pub fn shaper(&'a self, settings: ShaperSettings) -> FontShaper<'a> {
        FontShaper::new(self, self.shaper_data(), settings)
    }

    pub fn _pretty_print(&self) -> String {
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
}

impl<'a> std::fmt::Display for FontRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(sub) = self.subfamily_name() {
            write!(f, "{} - {}", self.family_name(), sub)
        } else {
            write!(f, "{}", self.family_name())
        }
    }
}

impl<'a> std::fmt::Debug for FontRef<'a> {
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

impl<'a> std::cmp::PartialEq for FontRef<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.cache_index == other.cache_index
    }
}

impl<'a> std::cmp::Eq for FontRef<'a> {}

impl<'a> std::cmp::PartialOrd for FontRef<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.cache_index.partial_cmp(&other.cache_index)
    }
}

impl<'a> std::cmp::Ord for FontRef<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cache_index.cmp(&other.cache_index)
    }
}

impl<'a> std::hash::Hash for FontRef<'a> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.cache_index.hash(state);
    }
}

struct FontCacheData {
    raw_data_ref: &'static [u8],
    font_ref_idx: u32,
    family_name: String,
    subfamily_name: Option<String>,
    revision: skrifa::raw::types::Fixed,
    variation_axes: SmallVec<[Axis; 4]>,
    named_instances: SmallVec<[NamedInstanceInfo; 8]>,
    features: SmallVec<[String; 32]>,
}

impl std::fmt::Debug for FontCacheData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontCacheData")
            .field("font_ref_idx", &self.font_ref_idx)
            .field("family_name", &self.family_name)
            .field("subfamily_name", &self.subfamily_name)
            .field("revision", &self.revision)
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

struct LazyFontCacheData {
    ext_font_ref: OnceLock<Box<ExtFontRef<'static>>>,
    shaper_data: OnceLock<Box<ShaperData>>,
    outline_glyphs_ref: OnceLock<Box<OutlineGlyphCollection<'static>>>,
}

impl LazyFontCacheData {
    fn new() -> Self {
        Self {
            ext_font_ref: OnceLock::new(),
            shaper_data: OnceLock::new(),
            outline_glyphs_ref: OnceLock::new(),
        }
    }

    fn ext_font_ref(
        &self,
        font_cache: &FontCache,
        font_cache_index: usize,
    ) -> &ExtFontRef<'static> {
        let font_data = &font_cache.font_datas[font_cache_index];
        self.ext_font_ref.get_or_init(|| {
            Box::new(
                ExtFontRef::from_index(font_data.raw_data_ref, font_data.font_ref_idx)
                    .expect("Unable to create FontRef<'static> for cached font"),
            )
        })
    }

    fn shaper_data(&self, font_cache: &FontCache, font_cache_index: usize) -> &ShaperData {
        self.shaper_data.get_or_init(|| {
            Box::new(ShaperData::new(
                self.ext_font_ref(font_cache, font_cache_index),
            ))
        })
    }

    fn outline_glyph_collection(
        &self,
        font_cache: &FontCache,
        font_cache_index: usize,
    ) -> &OutlineGlyphCollection<'static> {
        self.outline_glyphs_ref.get_or_init(|| {
            Box::new(
                self.ext_font_ref(font_cache, font_cache_index)
                    .outline_glyphs(),
            )
        })
    }
}

struct RawFontCacheData {
    font_ref_idx: u32,
    family_name: String,
    subfamily_name: Option<String>,
    revision: skrifa::raw::types::Fixed,
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
        raw_data_ref: &'static [u8],
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
        skipped: SmallVec<[usize; 16]>,
    },
    AlreadyCached {
        path: PathBuf,
        idxs: SmallVec<[usize; 16]>,
    },
    NoNewData {
        path: PathBuf,
        existing_idxs: SmallVec<[usize; 16]>,
    },
}

pub struct FontCache {
    paths: Vec<PathBuf>,
    font_file_types: Vec<FontFileType>,
    paths_to_font_idxs: HashMap<PathBuf, SmallVec<[usize; 16]>>,
    paths_to_data_refs: HashMap<PathBuf, &'static [u8]>,
    raw_data_hashes_to_paths: HashMap<u64, PathBuf>,

    font_datas: Vec<FontCacheData>,
    lazy_font_datas: Vec<LazyFontCacheData>,
}

#[allow(unused)]
impl FontCache {
    pub fn new() -> Self {
        Self {
            raw_data_hashes_to_paths: HashMap::new(),
            paths: Vec::new(),
            paths_to_font_idxs: HashMap::new(),
            paths_to_data_refs: HashMap::new(),
            font_file_types: Vec::new(),

            font_datas: Vec::new(),
            lazy_font_datas: Vec::new(),
        }
    }

    fn ext_font_ref(&self, font_index: usize) -> &ExtFontRef<'static> {
        self.lazy_font_datas[font_index].ext_font_ref(self, font_index)
    }

    fn shaper_data(&self, font_index: usize) -> &ShaperData {
        self.lazy_font_datas[font_index].shaper_data(self, font_index)
    }

    fn outline_glyph_collection(&self, font_index: usize) -> &OutlineGlyphCollection<'static> {
        self.lazy_font_datas[font_index].outline_glyph_collection(self, font_index)
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

    pub fn get_font<'a>(&'a self, idx: usize) -> Option<FontRef<'a>> {
        self.font_datas.get(idx).map(|fd| FontRef {
            font_cache: self,
            cache_index: idx,
            font_data: fd,
            lazy_font_data: &self.lazy_font_datas[idx],
        })
    }

    pub fn find_font<'a>(
        &'a self,
        family_name: impl Into<String>,
        subfamily_name: Option<impl Into<String>>,
    ) -> Result<FontRef<'a>> {
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
            Ok(FontRef {
                font_cache: &self,
                cache_index: family_idxs[0],
                font_data: &self.font_datas[family_idxs[0]],
                lazy_font_data: &self.lazy_font_datas[family_idxs[0]],
            })
        } else {
            Err(FontError::NotCached {
                family_name: fam_name,
                subfamily_name: subfam_name,
            }
            .into())
        }
    }

    pub fn search_fonts<'a>(&'a self, search_string: impl Into<String>) -> Vec<FontRef<'a>> {
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

        let mut results: Vec<FontRef<'a>> = Vec::with_capacity(8);

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
                    .map(|(idx, _)| FontRef {
                        font_cache: &self,
                        cache_index: idx,
                        font_data: &self.font_datas[idx],
                        lazy_font_data: &self.lazy_font_datas[idx],
                    }),
            );
        }

        results.sort();
        results.dedup();

        results
    }

    pub fn load_system_fonts(&mut self) -> Result<usize> {
        let system_font_paths = font_util::load_system_font_paths()?;
        self.load_multiple_font_files(system_font_paths)
    }

    pub fn load_multiple_font_files(&mut self, paths: Vec<impl Into<PathBuf>>) -> Result<usize> {
        let result_count_heuristic = 2 * paths.len();

        let raw_data_hashes_to_paths = Arc::new(&self.raw_data_hashes_to_paths);

        let raw_datas: Vec<Result<RawCacheResult>> = paths
            .into_iter()
            .map(|path| path.into())
            .collect::<Vec<PathBuf>>()
            .into_par_iter()
            .map(|path| self.load_raw_data(path, raw_data_hashes_to_paths.clone()))
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
                CacheResult::NoNewData { existing_idxs, .. } => {
                    existing_idxs.into_iter().collect::<Vec<usize>>()
                }
            };

            result_idxs.extend(idxs);
        }

        result_idxs.sort();
        result_idxs.dedup();

        Ok(result_idxs.len())
    }

    pub fn load_font_file(&mut self, path: impl Into<PathBuf>) -> Result<SmallVec<[usize; 16]>> {
        let path: PathBuf = path.into();
        let raw_data_hashes_to_paths = Arc::new(&self.raw_data_hashes_to_paths);
        let cache_result: CacheResult =
            self.store_raw_data(self.load_raw_data(&path, raw_data_hashes_to_paths.clone()))?;

        let results: SmallVec<[usize; 16]> = match cache_result {
            CacheResult::New {
                newly_cached,
                replaced,
                skipped,
                ..
            } => newly_cached
                .into_iter()
                .chain(replaced.into_iter())
                .chain(skipped.into_iter())
                .collect(),
            CacheResult::AlreadyCached { idxs, .. } => idxs,
            CacheResult::NoNewData { existing_idxs, .. } => existing_idxs,
        };

        Ok(results)
    }

    fn raw_data(&self) -> &Mutex<Arena<&'static [u8]>> {
        static DATA: LazyLock<Mutex<Arena<&'static [u8]>>> =
            LazyLock::new(|| Mutex::new(Arena::new()));
        &DATA
    }

    pub fn raw_data_size(&self) -> usize {
        self.raw_data().lock().iter_mut().map(|d| d.len()).sum()
    }

    fn load_raw_data(
        &self,
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
            return Ok(RawCacheResult::AlreadyCached { path: p.clone() });
        }

        let raw_data_ref: &'static [u8] = {
            let data = self.raw_data();
            let _lock = data.lock();
            // SAFETY: We hold the lock, and this is the only place
            // that modifies the static DATA
            let raw = unsafe { &*data.data_ptr() };
            raw.alloc(raw_bytes.leak())
        };

        // Load the data with skrifa
        let file_ref: skrifa::raw::FileRef = skrifa::raw::FileRef::new(raw_data_ref)?;

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

            font_datas.push(RawFontCacheData {
                font_ref_idx: font_ref_idx as u32,
                family_name,
                subfamily_name,
                revision: font_revision,
                variation_axes: axes,
                named_instances,
                features,
            })
        }

        Ok(RawCacheResult::New {
            path: path.as_ref().into(),
            raw_data_ref,
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
        let (path, raw_data_ref, raw_data_hash, font_file_type, font_datas) = match raw_cache_data {
            RawCacheResult::New {
                path,
                raw_data_ref,
                raw_data_hash,
                font_file_type,
                font_datas,
            } => (
                path,
                raw_data_ref,
                raw_data_hash,
                font_file_type,
                font_datas,
            ),
            RawCacheResult::AlreadyCached { path } => {
                let idxs = self.paths_to_font_idxs.get(&path).unwrap().clone();
                return Ok(CacheResult::AlreadyCached { path, idxs });
            }
        };

        // new_font_datas.len() + replace_font_datas.len() + skipped_font_datas should equal the number
        // of fonts in the file_ref
        let font_datas_length = font_datas.len();
        let mut new_font_datas: Vec<FontCacheData> = Vec::new();
        let mut replace_font_datas: Vec<(usize, FontCacheData)> = Vec::new();
        let mut skipped_font_idxs: SmallVec<[usize; 16]> = SmallVec::new();

        for raw_font_cache_data in font_datas {
            let fd = FontCacheData {
                raw_data_ref,
                font_ref_idx: raw_font_cache_data.font_ref_idx,
                family_name: raw_font_cache_data.family_name,
                subfamily_name: raw_font_cache_data.subfamily_name,
                revision: raw_font_cache_data.revision,
                variation_axes: raw_font_cache_data.variation_axes,
                named_instances: raw_font_cache_data.named_instances,
                features: raw_font_cache_data.features,
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
                    skipped_font_idxs.push(existing.cache_index);
                    continue;
                }
            } else {
                // if no duplicate is found, simply add the font
                new_font_datas.push(fd);
            };
        }

        // all fonts in the file should be processed
        debug_assert_eq!(
            new_font_datas.len() + replace_font_datas.len() + skipped_font_idxs.len(),
            // lengths start at 1, font_ref_index starts at 0
            font_datas_length,
            "{}",
            path.to_string_lossy()
        );

        // if all fonts were skipped, only save the path and hashed data values
        // (so the cache can verify this file/data was already processed)
        // and return an empty smallvec
        if new_font_datas.is_empty() && replace_font_datas.is_empty() {
            self.paths.push(path.clone());
            self.raw_data_hashes_to_paths
                .insert(raw_data_hash, path.clone());
            self.font_file_types.push(font_file_type);

            self.paths_to_font_idxs
                .insert(path.clone(), SmallVec::default());
            self.paths_to_data_refs.insert(path.clone(), raw_data_ref);

            return Ok(CacheResult::NoNewData {
                path,
                existing_idxs: skipped_font_idxs,
            });
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

        // add path and hash related stuff
        // need to do this now because we need this data to properly process
        // replacements
        self.paths.push(path.clone());
        self.raw_data_hashes_to_paths
            .insert(raw_data_hash, path.clone());
        self.font_file_types.push(font_file_type);

        let mut path_to_font_idxs = new_font_idxs.clone();
        path_to_font_idxs.extend(replace_font_datas.iter().map(|fd| fd.0));

        self.paths_to_font_idxs
            .insert(path.clone(), path_to_font_idxs.clone());

        self.paths_to_data_refs.insert(path.clone(), raw_data_ref);

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
            self.lazy_font_datas[font_data.0] = LazyFontCacheData::new();
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

        self.lazy_font_datas.extend(Vec::from_iter(
            std::iter::repeat_with(|| LazyFontCacheData::new()).take(new_font_datas.len()),
        ));
        self.font_datas.extend(new_font_datas);

        // font_datas and lazy_font_datas must be equal in length
        debug_assert_eq!(
            self.font_datas.len(),
            self.lazy_font_datas.len(),
            "{}",
            path.to_string_lossy()
        );

        Ok(CacheResult::New {
            path,
            newly_cached: new_font_idxs,
            replaced: replaced_font_idxs,
            skipped: skipped_font_idxs,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
enum ShaperInstanceSettings {
    Variations(Vec<Variation>),
    NamedInstance(NamedInstanceInfo),
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
    pub fn new() -> Self {
        Self {
            instance_settings: None,
            shape_features: None,
        }
    }

    pub fn _with_variations(
        mut self,
        variations: impl IntoIterator<Item: Into<Variation>>,
    ) -> Self {
        self.instance_settings = Some(ShaperInstanceSettings::Variations(
            variations.into_iter().map(|v| v.into()).collect(),
        ));
        self
    }

    pub fn _with_named_instance(mut self, named_instance: NamedInstanceInfo) -> Self {
        self.instance_settings = Some(ShaperInstanceSettings::NamedInstance(named_instance));
        self
    }

    pub fn _with_features(mut self, features: impl IntoIterator<Item: Into<Feature>>) -> Self {
        self.shape_features = Some(features.into_iter().map(|f| f.into()).collect());
        self
    }

    pub fn _coords<'a>(&self, font: &'a FontRef<'a>) -> skrifa::instance::Location {
        match &self.instance_settings {
            Some(si) => match si {
                ShaperInstanceSettings::Variations(variations) => {
                    font.ext_font_ref().axes().location(
                        variations
                            .iter()
                            .map(|v| skrifa::setting::VariationSetting::new(v.tag, v.value)),
                    )
                }
                ShaperInstanceSettings::NamedInstance(named_instance_info) => {
                    named_instance_info.coords.clone()
                }
            },
            None => skrifa::instance::Location::default(),
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
    font_cache_ref: &'a FontRef<'a>,
    shaper_data: &'a ShaperData,
    _shaper_settings: ShaperSettings,
    shaper_instance: ShaperInstance,
    features: Vec<Feature>,
}

impl<'a> FontShaper<'a> {
    fn new(
        font_cache_ref: &'a FontRef<'a>,
        shaper_data: &'a ShaperData,
        shaper_settings: ShaperSettings,
    ) -> FontShaper<'a> {
        let ext_font_ref = font_cache_ref.ext_font_ref();
        let shaper_instance = match shaper_settings.instance_settings {
            Some(ShaperInstanceSettings::Variations(ref variations)) => {
                ShaperInstance::from_variations(ext_font_ref, variations)
            }
            Some(ShaperInstanceSettings::NamedInstance(ref ni)) => {
                ShaperInstance::from_named_instance(ext_font_ref, ni.named_instance_index)
            }
            None => ShaperInstance::from_variations(ext_font_ref, &[] as &[Variation]),
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
            shaper_data,
            _shaper_settings: shaper_settings,
            shaper_instance: shaper_instance,
            features,
        }
    }

    pub fn _with_settings(mut self, settings: ShaperSettings) -> Self {
        if settings == self._shaper_settings {
            return self;
        }
        let ext_font_ref = self.font_cache_ref.ext_font_ref();
        if let Some(instance_settings) = settings.instance_settings {
            match instance_settings {
                ShaperInstanceSettings::Variations(variations) => self
                    .shaper_instance
                    .set_variations(ext_font_ref, variations),
                ShaperInstanceSettings::NamedInstance(ni) => self
                    .shaper_instance
                    .set_named_instance(ext_font_ref, ni.named_instance_index),
            }
        }

        if let Some(shape_features) = settings.shape_features {
            self.features = shape_features
        }

        self
    }

    pub fn shape(
        &'a self,
        line: &str,
        input_buffer: Option<UnicodeBuffer>,
        size: Option<skrifa::instance::Size>,
    ) -> GlyphBuffer {
        let mut buffer = if let Some(mut input_buffer) = input_buffer {
            input_buffer.clear();
            input_buffer
        } else {
            UnicodeBuffer::new()
        };

        buffer.push_str(line);

        buffer.set_direction(harfrust::Direction::LeftToRight);
        buffer.set_script(harfrust::Script::from_str("Latn").unwrap());
        buffer.set_language(harfrust::Language::from_str("en").unwrap());

        let point_size: Option<f32> = size.map(|s| s.ppem().map(|ppem| ppem * 0.75)).flatten();

        let shaper = self
            .shaper_data
            .shaper(self.font_cache_ref.ext_font_ref())
            .instance(Some(&self.shaper_instance))
            .point_size(point_size)
            .build();
        let result = shaper.shape(buffer, &self.features);

        result
    }
}

pub struct Rasterizer {
    path: Vec<zeno::Command>,
    draw_buffer: Vec<u8>,
    scratch: zeno::Scratch,
}

impl Rasterizer {
    pub fn new() -> Self {
        Self {
            path: Vec::new(),
            draw_buffer: Vec::new(),
            scratch: zeno::Scratch::new(),
        }
    }

    pub fn render_mask(
        &mut self,
        font: &FontRef<'_>,
        glyph_id: GlyphId,
        size: skrifa::instance::Size,
        coords: &skrifa::instance::Location,
        buffer: &mut [u8],
        start: usize,
        _row_size: usize,
    ) -> zeno::Placement {
        self.path.clear();
        self.draw_buffer.clear();

        let hinting_instance = skrifa::outline::HintingInstance::new(
            font.outline_glyph_collection(),
            size,
            coords,
            skrifa::outline::HintingOptions {
                engine: skrifa::outline::Engine::AutoFallback,
                target: skrifa::outline::Target::Smooth {
                    mode: skrifa::outline::SmoothMode::Lcd,
                    symmetric_rendering: false,
                    preserve_linear_metrics: true,
                },
            },
        )
        .expect("Could not create HintingInstance");
        let draw_settings = skrifa::outline::DrawSettings::hinted(&hinting_instance, true);

        let glyph_outline = font.outline_glyph_collection().get(glyph_id).unwrap();
        glyph_outline.draw(draw_settings, self).unwrap();

        let placement = zeno::Mask::with_scratch(&self.path, &mut self.scratch)
            .origin(zeno::Origin::BottomLeft)
            .format(zeno::Format::Subpixel)
            .inspect(|format, width, height| {
                self.draw_buffer
                    .resize(format.buffer_size(width, height), 0);
            })
            .render_into(&mut buffer[start..], None);
        placement
    }
}

impl skrifa::outline::OutlinePen for Rasterizer {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to([x, y]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to([x, y]);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to([cx0, cy0], [x, y]);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to([cx0, cy0], [cx1, cy1], [x, y]);
    }

    fn close(&mut self) {
        self.path.close();
    }
}

#[derive(Eq, Hash, PartialEq)]
struct GlyphCacheKey {
    font_cache_index: usize,
    glyph_id: GlyphId,
    ppem: u32,
    coords: skrifa::instance::Location,
}

pub struct GlyphCache {
    texture_row_size: usize,
    _texture_rows: usize,
    atlas: etagere::AtlasAllocator,
    draw_texture: Vec<u8>,
    pub texture: Vec<u8>,
    texture_data_dirty: bool,
    rasterizer: Rasterizer,
    glyph_map: HashMap<GlyphCacheKey, (etagere::AllocId, zeno::Placement)>,
}

impl GlyphCache {
    pub fn new(texture_row_size: usize, texture_rows: usize) -> Self {
        Self {
            texture_row_size,
            _texture_rows: texture_rows,
            atlas: etagere::AtlasAllocator::new(etagere::size2(
                texture_row_size as i32,
                texture_rows as i32,
            )),
            draw_texture: vec![0u8; texture_row_size * texture_rows],
            texture: vec![0u8; texture_row_size * texture_rows],
            texture_data_dirty: false,
            rasterizer: Rasterizer::new(),
            glyph_map: HashMap::new(),
        }
    }

    pub fn get_glyph_texture_bounds(
        &mut self,
        font: &FontRef<'_>,
        glyph_id: GlyphId,
        size: skrifa::instance::Size,
        coords: skrifa::instance::Location,
    ) -> (
        zeno::Placement,
        etagere::euclid::Box2D<f32, etagere::euclid::UnknownUnit>,
    ) {
        fn result_uv_bounds(
            alloc_box: etagere::euclid::Box2D<i32, etagere::euclid::UnknownUnit>,
            raster_placement: &zeno::Placement,
        ) -> etagere::euclid::Box2D<f32, etagere::euclid::UnknownUnit> {
            etagere::euclid::Box2D::from_origin_and_size(
                alloc_box.to_f32().scale(0.25, 1.0).min,
                etagere::euclid::Size2D::new(
                    raster_placement.width as f32,
                    raster_placement.height as f32,
                ),
            )
        }

        let rounded_size = size.ppem().unwrap().floor() as u32;

        let key = GlyphCacheKey {
            font_cache_index: font.cache_index,
            glyph_id,
            ppem: rounded_size,
            coords: coords.clone(),
        };

        if let Some((alloc_id, placement)) = self.glyph_map.get(&key) {
            return (
                *placement,
                result_uv_bounds(self.atlas.get(*alloc_id), placement),
            );
        }

        for v in &mut self.draw_texture {
            *v = 0
        }

        let placement = self.rasterizer.render_mask(
            font,
            glyph_id,
            size,
            &key.coords,
            &mut self.draw_texture,
            0,
            self.texture_row_size,
        );

        let allocation = self
            .atlas
            .allocate(etagere::size2(
                (placement.width * 4) as i32,
                placement.height as i32,
            ))
            .unwrap();

        let start = (allocation.rectangle.min.y as usize) * self.texture_row_size
            + (allocation.rectangle.min.x) as usize;

        let width = placement.width as usize;
        let height = placement.height as usize;

        for row in 0..height {
            for value in 0..width {
                let r = self.draw_texture[(row * width * 4) + value * 4];
                let g = self.draw_texture[(row * width * 4) + value * 4 + 1];
                let b = self.draw_texture[(row * width * 4) + value * 4 + 2];
                let alpha = r.saturating_add(g).saturating_add(b);
                self.texture[start + (row * self.texture_row_size) + value * 4] = r;
                self.texture[start + (row * self.texture_row_size) + value * 4 + 1] = g;
                self.texture[start + (row * self.texture_row_size) + value * 4 + 2] = b;
                self.texture[start + (row * self.texture_row_size) + value * 4 + 3] = alpha;
            }
        }

        let uv_bounds = result_uv_bounds(allocation.rectangle, &placement);

        // debug draw border
        /*for value in uv_bounds.min.x as usize * 4..=uv_bounds.max.x as usize * 4 {
            self.texture[uv_bounds.min.y as usize * self.texture_row_size + value] = 255;
            self.texture[(uv_bounds.max.y as usize) * self.texture_row_size + value] = 255;
        }
        for row in uv_bounds.min.y as usize..=uv_bounds.max.y as usize {
            self.texture[row * self.texture_row_size + uv_bounds.min.x as usize * 4] = 255;
            self.texture[row * self.texture_row_size + uv_bounds.min.x as usize * 4 + 1] = 255;
            self.texture[row * self.texture_row_size + uv_bounds.min.x as usize * 4 + 2] = 255;
            self.texture[row * self.texture_row_size + uv_bounds.min.x as usize * 4 + 3] = 255;

            self.texture[row * self.texture_row_size + uv_bounds.max.x as usize * 4] = 255;
            self.texture[row * self.texture_row_size + uv_bounds.max.x as usize * 4 + 1] = 255;
            self.texture[row * self.texture_row_size + uv_bounds.max.x as usize * 4 + 2] = 255;
            self.texture[row * self.texture_row_size + uv_bounds.max.x as usize * 4 + 3] = 255;
        }*/

        /*image::save_buffer_with_format(
            "tex.png",
            &self.texture,
            (self.texture_row_size / 4) as u32,
            self.texture_rows as u32,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();*/

        self.glyph_map.insert(key, (allocation.id, placement));

        self.texture_data_dirty = true;

        (placement, uv_bounds)
    }

    pub fn prepare_draw_for_glyph(
        &self,
        vertices: &mut Vec<GlyphVertex>,
        indices: &mut Vec<u16>,
        glyph: RenderGlyphData,
        caret_x: f32,
        caret_y: f32,
    ) {
        let (glyph_vertices, glyph_indices) = glyph.to_indexed_vertices(caret_x, caret_y);
        let previous_vertices_len = vertices.len() as u16;
        for v in glyph_vertices {
            vertices.push(v);
        }
        for i in glyph_indices {
            indices.push(i + previous_vertices_len);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RenderGlyphData {
    px_bounds: etagere::euclid::Box2D<i32, etagere::euclid::UnknownUnit>,
    uv_bounds: etagere::euclid::Box2D<f32, etagere::euclid::UnknownUnit>,
}

impl RenderGlyphData {
    pub fn to_indexed_vertices(&self, caret_x: f32, caret_y: f32) -> ([GlyphVertex; 4], [u16; 6]) {
        let left = self.px_bounds.min.x as f32;
        let right = self.px_bounds.max.x as f32;
        let top = self.px_bounds.max.y as f32;
        let bottom = self.px_bounds.min.y as f32;
        let vertices: [GlyphVertex; 4] = [
            GlyphVertex {
                caret_position: [caret_x, caret_y, 0.0],
                px_bounds_offset: [left, top],
                tex_coords: [self.uv_bounds.min.x as f32, self.uv_bounds.min.y as f32],
            },
            GlyphVertex {
                caret_position: [caret_x, caret_y, 0.0],
                px_bounds_offset: [left, bottom],
                tex_coords: [self.uv_bounds.min.x as f32, self.uv_bounds.max.y as f32],
            },
            GlyphVertex {
                caret_position: [caret_x, caret_y, 0.0],
                px_bounds_offset: [right, bottom],
                tex_coords: [self.uv_bounds.max.x as f32, self.uv_bounds.max.y as f32],
            },
            GlyphVertex {
                caret_position: [caret_x, caret_y, 0.0],
                px_bounds_offset: [right, top],
                tex_coords: [self.uv_bounds.max.x as f32, self.uv_bounds.min.y as f32],
            },
        ];
        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];

        (vertices, indices)
    }
}

impl From<&etagere::euclid::Box2D<f32, etagere::euclid::UnknownUnit>> for RenderGlyphData {
    fn from(value: &etagere::euclid::Box2D<f32, etagere::euclid::UnknownUnit>) -> Self {
        RenderGlyphData {
            px_bounds: etagere::euclid::Box2D::from_size(value.to_i32().size()),
            uv_bounds: *value,
        }
    }
}
