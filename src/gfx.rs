mod glyph_cache;

use pollster::FutureExt as _;
use winit::window::Window;

use crate::error::Result;
use glyph_cache::GlyphCache;

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

pub struct GfxState<'a> {
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    scale_factor: f32,
    render_pipeline: wgpu::RenderPipeline,
    glyph_cache: GlyphCache<'a>,
    glyph_vertex_buffer: wgpu::Buffer,
}

#[rustfmt::skip]
pub const _OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);

impl<'a> GfxState<'a> {
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

        let mut glyph_cache =
            GlyphCache::new(&device, &queue, ab_glyph::PxScale::from(64.0), scale_factor);

        /*let glyph_cache_texture_bind_group_layout =
            device.create_bind_group_layout(&GlyphCache::texture_bind_group_layout_desc());

        let glyph_cache_texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph_cache_texture_bind_group"),
            layout: &glyph_cache_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&glyph_cache.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&glyph_cache.sampler),
                },
            ],
        });*/

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&glyph_cache.texture_bind_group_layout],
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

        glyph_cache.prepare_cache_glyph('a', ab_glyph::PxScale::from(128.0));
        glyph_cache.prepare_cache_glyph('b', ab_glyph::PxScale::from(128.0));

        glyph_cache.queue_write_texture_if_changed(&queue);

        GfxState {
            surface,
            device,
            queue,
            config,
            size,
            scale_factor,
            render_pipeline,
            glyph_cache,
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
        px_scale: ab_glyph::PxScale,
        x: f32,
        baseline_y: f32,
    ) -> Result<Vec<Vertex>> {
        let idx = self
            .glyph_cache
            .cached_chars
            .iter()
            .position(|(c, s)| *c == character && *s == px_scale)
            .unwrap();

        // Glyphs are already scaled to scale_factor in the texture cache, don'te rescale here.
        let surface_width_px = self.size.width as f32;
        let surface_height_px = self.size.height as f32;

        let uv_bounds = &self.glyph_cache.cached_uv_bounds[idx];

        let px_bounds = &self.glyph_cache.cached_px_bounds[idx];

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
                tex_coords: [uv_bounds.left(), uv_bounds.top()],
            },
            Vertex {
                position: [left, bottom, 0.0],
                tex_coords: [uv_bounds.left(), uv_bounds.bottom()],
            },
            Vertex {
                position: [right, bottom, 0.0],
                tex_coords: [uv_bounds.right(), uv_bounds.bottom()],
            },
            Vertex {
                position: [right, bottom, 0.0],
                tex_coords: [uv_bounds.right(), uv_bounds.bottom()],
            },
            Vertex {
                position: [right, top, 0.0],
                tex_coords: [uv_bounds.right(), uv_bounds.top()],
            },
            Vertex {
                position: [left, top, 0.0],
                tex_coords: [uv_bounds.left(), uv_bounds.top()],
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
                    ab_glyph::PxScale::from(128.0),
                    (256.0 / (self.size.width as f32 / self.scale_factor as f32)) - 1.0,
                    1.0 - (256.0 / (self.size.height as f32 / self.scale_factor as f32)),
                )
                .unwrap();
            vertices.append(
                &mut self
                    .get_vertices_for_char(
                        'b',
                        ab_glyph::PxScale::from(128.0),
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
            render_pass.set_bind_group(0, &self.glyph_cache.texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.glyph_vertex_buffer.slice(..));
            render_pass.draw(0..vertices.len() as u32, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
