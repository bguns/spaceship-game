mod glyph_cache;
mod vertex;

use ab_glyph::ScaleFont;
use pollster::FutureExt as _;
use winit::window::Window;

use crate::error::Result;
use glyph_cache::{GlyphCache, GlyphPxScale};
use vertex::Vertex;

pub struct GfxState {
    surface: wgpu::Surface,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    screen_scale_factor: f32,
    render_pipeline: wgpu::RenderPipeline,
    glyph_cache: GlyphCache,
    glyph_vertex_buffer: wgpu::Buffer,
    glyph_index_buffer: wgpu::Buffer,
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
        let screen_scale_factor = window.scale_factor() as f32;

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
            GlyphCache::new(&device, size.width, size.height, screen_scale_factor);

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
            size: (4000 as usize * std::mem::size_of::<Vertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let glyph_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph_index_buffer"),
            size: (6000 as usize * std::mem::size_of::<u16>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let font_path_1 = std::path::PathBuf::from("./fonts/fira-sans/FiraSans-Regular.otf");
        let _ = glyph_cache.cache_font(font_path_1);

        let font_path_2 = std::path::PathBuf::from("./fonts/westwood-studio/Westwood Studio.ttf");
        let _ = glyph_cache.cache_font(font_path_2);

        GfxState {
            surface,
            device,
            queue,
            config,
            size,
            screen_scale_factor,
            render_pipeline,
            glyph_cache,
            glyph_vertex_buffer,
            glyph_index_buffer,
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
            self.glyph_cache
                .surface_resized(new_size_apply.width, new_size_apply.height);
        }
    }

    pub fn render(&mut self, game_state: &super::GameState) -> Result<()> {
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
                                r: 10.0 / 255.0,
                                g: 10.0 / 255.0,
                                b: 10.0 / 255.0,
                                a: 1.0,
                            }),
                            // We want to store the rendered results to the (Surface)Texture behind the TextureView (the view)
                            store: true,
                        },
                    },
                ],
                depth_stencil_attachment: None,
            });

            let px_scale = self.glyph_cache.glyph_px_scale(128.0);

            self.glyph_cache.queue_write_texture_if_changed(&self.queue);

            self.glyph_cache.ensure_glyph_cached(0, 'a', px_scale);
            self.glyph_cache.ensure_glyph_cached(0, 'b', px_scale);

            let a_glyph = self
                .glyph_cache
                .try_get_cached_glyph_data(0, 'a', px_scale)
                .unwrap();

            let b_glyph = self
                .glyph_cache
                .try_get_cached_glyph_data(0, 'b', px_scale)
                .unwrap();

            let mut vertices: Vec<Vertex> = Vec::with_capacity(4000);
            let mut indices: Vec<u16> = Vec::with_capacity(6000);

            self.glyph_cache.prepare_draw_for_glyph(
                &mut vertices,
                &mut indices,
                a_glyph,
                -0.8,
                0.8,
            );
            self.glyph_cache.prepare_draw_for_glyph(
                &mut vertices,
                &mut indices,
                b_glyph,
                -0.8 + 2.0
                    * self.logical_px_to_horizontal_screen_space_offset(
                        a_glyph.px_scale().x.ceil() as u32,
                    ),
                0.8,
            );

            if let Some(txt) = &game_state.text {
                for c in txt.chars() {
                    self.glyph_cache.ensure_glyph_cached(1, c, px_scale);
                }
                let scaled_font = self
                    .glyph_cache
                    .try_get_cached_font_with_scale(1, px_scale)
                    .unwrap();
                let mut previous_char: Option<char> = None;
                let mut caret_x = -0.8;
                for c in txt.chars() {
                    if let Some(glyph_data) =
                        self.glyph_cache.try_get_cached_glyph_data(1, c, px_scale)
                    {
                        if let Some(prev) = previous_char {
                            caret_x += scaled_font
                                .kern(scaled_font.glyph_id(prev), scaled_font.glyph_id(c))
                                / self.size.width as f32;
                        }
                        self.glyph_cache.prepare_draw_for_glyph(
                            &mut vertices,
                            &mut indices,
                            glyph_data,
                            caret_x,
                            0.6,
                        );

                        caret_x +=
                            scaled_font.h_advance(scaled_font.glyph_id(c)) / self.size.width as f32;
                        previous_char = Some(c);
                    } else {
                        caret_x += scaled_font.h_advance(scaled_font.glyph_id(' '))
                            / self.size.width as f32;
                    }
                }
            }

            let fps = 1_000_000.0 / game_state.delta_time.as_micros() as f64;
            let fps_text = &format!(
                "Elapsed time: {}; Runtime: {}; dt: {:.2}, Frame number: {}; FPS: {:.2}",
                game_state.start_time.elapsed().as_millis(),
                game_state.run_time.as_millis(),
                game_state.delta_time.as_micros() as f64 / 1_000.0,
                game_state.frame_number,
                fps
            );

            let px_scale = self.glyph_cache.glyph_px_scale(48.0);
            for c in fps_text.chars() {
                self.glyph_cache.ensure_glyph_cached(0, c, px_scale);
            }
            let scaled_font = self
                .glyph_cache
                .try_get_cached_font_with_scale(0, px_scale)
                .unwrap();
            let mut previous_char: Option<char> = None;
            let mut caret_x = -0.9;
            for c in fps_text.chars() {
                if let Some(glyph_data) = self.glyph_cache.try_get_cached_glyph_data(0, c, px_scale)
                {
                    if let Some(prev) = previous_char {
                        caret_x += scaled_font
                            .kern(scaled_font.glyph_id(prev), scaled_font.glyph_id(c))
                            / self.size.width as f32;
                    }
                    self.glyph_cache.prepare_draw_for_glyph(
                        &mut vertices,
                        &mut indices,
                        glyph_data,
                        caret_x,
                        0.9,
                    );
                    caret_x +=
                        scaled_font.h_advance(scaled_font.glyph_id(c)) / self.size.width as f32;
                    previous_char = Some(c);
                } else {
                    caret_x +=
                        scaled_font.h_advance(scaled_font.glyph_id(' ')) / self.size.width as f32;
                }
            }

            let old_vertices_len = vertices.len() as u16;

            vertices.append(&mut vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    tex_coords: [0.0, 0.0],
                },
                Vertex {
                    position: [0.0, -1.0, 0.0],
                    tex_coords: [0.0, 1.0],
                },
                Vertex {
                    position: [1.0, -1.0, 0.0],
                    tex_coords: [1.0, 1.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    tex_coords: [1.0, 0.0],
                },
            ]);

            indices.append(&mut vec![
                0 + old_vertices_len,
                1 + old_vertices_len,
                2 + old_vertices_len,
                2 + old_vertices_len,
                3 + old_vertices_len,
                0 + old_vertices_len,
            ]);

            self.queue.write_buffer(
                &self.glyph_vertex_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );

            self.queue
                .write_buffer(&self.glyph_index_buffer, 0, bytemuck::cast_slice(&indices));

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.glyph_cache.texture_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.glyph_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(self.glyph_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    fn logical_px_to_horizontal_screen_space_offset(&self, logical_px_offset: u32) -> f32 {
        logical_px_offset as f32 * self.screen_scale_factor as f32 / self.size.width as f32
    }

    fn logical_px_to_vertical_screen_space_offset(&self, logical_px_offset: u32) -> f32 {
        logical_px_offset as f32 * self.screen_scale_factor as f32 / self.size.height as f32
    }
}
