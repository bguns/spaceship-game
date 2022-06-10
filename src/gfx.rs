use pollster::FutureExt as _;
use winit::window::Window;

use crate::error::Result;

use ab_glyph::Font;

pub struct GlyphCacheTexture {
    pub font_path: std::path::PathBuf,
    pub px_scale: ab_glyph::PxScale,
    cached_chars: Vec<char>,
    cached_px_bounds: Vec<ab_glyph::Rect>,
    cached_uv_bounds: Vec<cgmath::Matrix2<f32>>,
    max_x_assigned: usize,
    max_y_assigned: usize,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl GlyphCacheTexture {
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
        let font_data = include_bytes!("../fonts/wqy-microhei/WenQuanYiMicroHei.ttf");
        let font = ab_glyph::FontRef::try_from_slice(font_data).expect("Unable to load font.");

        let a_glyph: ab_glyph::Glyph = font.glyph_id('a').with_scale(px_scale);

        let cached_chars: Vec<char> = vec!['a', 'b'];
        let mut cached_uv_bounds: Vec<cgmath::Matrix2<f32>> = Vec::new();
        let mut cached_px_bounds: Vec<ab_glyph::Rect> = Vec::new();

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
            cached_px_bounds.push(px_bounds);
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

            cached_px_bounds.push(px_bounds);
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

        Self {
            font_path,
            px_scale,
            cached_chars,
            cached_px_bounds,
            cached_uv_bounds,
            max_x_assigned,
            max_y_assigned,
            texture,
            view,
            sampler,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

impl Vertex {
    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                },
            ],
        }
    }
}

pub struct GfxState {
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    scale_factor: f32,
    render_pipeline: wgpu::RenderPipeline,
    glyph_cache_texture: GlyphCacheTexture,
    glyph_cache_texture_bind_group: wgpu::BindGroup,
    glyph_vertex_buffer: wgpu::Buffer,
}

#[rustfmt::skip]
pub const _OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);

impl GfxState {
    pub fn new(window: &Window) -> Self {
        let size = window.inner_size();
        let scale_factor = window.scale_factor() as f32;

        // The instance's main purpose is to create Adapters and Surfaces
        // Backends::all => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(wgpu::Backends::all());

        // The surface is the part of the window that we draw to.
        let surface = unsafe { instance.create_surface(window) };

        let (adapter, device, queue) =
            async { Self::load_adapter_device_queue(&instance, &surface).await }.block_on();

        let config = wgpu::SurfaceConfiguration {
            // How SurfaceTextures will be used.
            // RENDER_ATTACHMENT specifies that the textures fill be used to write to the screen.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            // How SurfaceTextures fill be stored on the GPU.
            format: surface.get_preferred_format(&adapter).unwrap(),
            // width and height in pixels of a SurfaceTexture
            width: size.width,
            height: size.height,
            // How to sync the surface with the display.
            // Fifo = VSync
            present_mode: wgpu::PresentMode::Fifo,
        };

        surface.configure(&device, &config);

        let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let glyph_cache_texture =
            GlyphCacheTexture::new(&device, &queue, ab_glyph::PxScale::from(64.0), scale_factor);

        let glyph_cache_texture_bind_group_layout =
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

        let glyph_cache_texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph_cache_texture_bind_group"),
            layout: &glyph_cache_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glyph_cache_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&glyph_cache_texture.sampler),
                },
            ],
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&glyph_cache_texture_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                // What type of vertices we want to pass to the vertex shader.
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }],
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
        });

        let glyph_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_vertex_buffer"),
            size: (6000 as usize * std::mem::size_of::<Vertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        GfxState {
            surface,
            device,
            queue,
            config,
            size,
            scale_factor,
            render_pipeline,
            glyph_cache_texture,
            glyph_cache_texture_bind_group,
            glyph_vertex_buffer,
        }
    }

    async fn load_adapter_device_queue(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface,
    ) -> (wgpu::Adapter, wgpu::Device, wgpu::Queue) {
        // The adapter is the handle to the actual graphics card.
        // We use this to create the Device and Queue.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Unable to load adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    features: wgpu::Features::empty(),
                    limits: wgpu::Limits::default(),
                    label: None,
                },
                None, // Trace path
            )
            .await
            .expect("Unable to load device/queue");

        (adapter, device, queue)
    }

    pub fn resize(&mut self, new_size: Option<winit::dpi::PhysicalSize<u32>>) {
        let new_size_apply = new_size.unwrap_or(self.size);
        if new_size_apply.width > 0 && new_size_apply.height > 0 {
            self.size = new_size_apply;
            self.config.width = new_size_apply.width;
            self.config.height = new_size_apply.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn get_vertices_for_char(
        &self,
        character: char,
        x: f32,
        baseline_y: f32,
    ) -> Result<Vec<Vertex>> {
        let idx = self
            .glyph_cache_texture
            .cached_chars
            .iter()
            .position(|c| *c == character)
            .unwrap();

        // Glyphs are already scaled to scale_factor in the texture cache, don'te rescale here.
        let surface_width_px = self.size.width as f32;
        let surface_height_px = self.size.height as f32;

        let bounding_box = self.glyph_cache_texture.cached_uv_bounds[idx];

        let px_bounds = self.glyph_cache_texture.cached_px_bounds[idx];

        let left = x + px_bounds.min.x / surface_width_px;
        let right = x + px_bounds.max.x / surface_width_px;
        // ab_glyph assumes opengl coordinates (0, 0 top left),
        // but wgpu uses DX11/Metal coordinates (0, 0 center),
        // so y axis needs to subtract bounds, not add
        let top = baseline_y - px_bounds.min.y / surface_height_px;
        let bottom = baseline_y - px_bounds.max.y / surface_height_px;

        Ok(vec![
            Vertex {
                position: [left, top, 0.0],
                tex_coords: [bounding_box.x.x, bounding_box.x.y],
            },
            Vertex {
                position: [left, bottom, 0.0],
                tex_coords: [bounding_box.x.x, bounding_box.y.y],
            },
            Vertex {
                position: [right, bottom, 0.0],
                tex_coords: [bounding_box.y.x, bounding_box.y.y],
            },
            Vertex {
                position: [right, bottom, 0.0],
                tex_coords: [bounding_box.y.x, bounding_box.y.y],
            },
            Vertex {
                position: [right, top, 0.0],
                tex_coords: [bounding_box.y.x, bounding_box.x.y],
            },
            Vertex {
                position: [left, top, 0.0],
                tex_coords: [bounding_box.x.x, bounding_box.x.y],
            },
        ])
    }

    pub fn render(&mut self, text: Option<&str>) -> Result<()> {
        // Get SurfaceTexture
        let output = self.surface.get_current_texture()?;
        // Create TextureView with default settings
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // Create CommandEncoder to create the actual commands to send to the gpu.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        // begin_render_pass borrows encoder mutably, so we need to make sure that the borrow
        // is dropped before we can call encoder.finish()
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[
                    // This is what [[location(0)]] in the fragment shader targets
                    wgpu::RenderPassColorAttachment {
                        // The view to save the colors to. In this case, the screen.
                        view: &view,
                        // Target that will receive the resolved output. Is the same as `view` unless multisampling is enabled.
                        resolve_target: None,
                        // What to do with the colors on the view (i.e. the screen)
                        ops: wgpu::Operations {
                            // Load tells wgpu how to handle colors stored from the previous frame (we clear the screen)
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 25.0 / 255.0,
                                g: 25.0 / 255.0,
                                b: 25.0 / 255.0,
                                a: 1.0,
                            }),
                            // We want to store the rendered results to the (Surface)Texture behind the TextureView (the view)
                            store: true,
                        },
                    },
                ],
                depth_stencil_attachment: None,
            });

            let mut vertices = self
                .get_vertices_for_char(
                    'a',
                    (256.0 / (self.size.width as f32 / self.scale_factor as f32)) - 1.0,
                    1.0 - (256.0 / (self.size.height as f32 / self.scale_factor as f32)),
                )
                .unwrap();
            vertices.append(
                &mut self
                    .get_vertices_for_char(
                        'b',
                        (256.0 / (self.size.width as f32 / self.scale_factor as f32)) - 1.0
                            + 256.0 / (self.size.width as f32 / self.scale_factor as f32),
                        1.0 - (256.0 / (self.size.height as f32 / self.scale_factor as f32)),
                    )
                    .unwrap(),
            );

            self.queue.write_buffer(
                &self.glyph_vertex_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.glyph_cache_texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.glyph_vertex_buffer.slice(..));
            render_pass.draw(0..vertices.len() as u32, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
