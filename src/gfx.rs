pub mod text;
mod vertex;

use std::sync::Arc;

use pollster::FutureExt as _;
use text::TextRenderer;
use wgpu::util::DeviceExt;
use winit::window::Window;

use anyhow::Result;
use vertex::LineVertex;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SurfaceDimensionsUniform {
    width: u32,
    height: u32,
    scale_factor: f32,
}

pub struct GfxState {
    pub _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    screen_scale_factor: f32,
    debug_line_vertex_buffer: wgpu::Buffer,
    debug_line_render_pipeline: wgpu::RenderPipeline,
    text_renderer: TextRenderer,
    _line_vertex_buffer: wgpu::Buffer,
    _line_render_pipeline: wgpu::RenderPipeline,
    surface_dimensions_buffer: wgpu::Buffer,
    surface_dimensions_bind_group: wgpu::BindGroup,
}

#[rustfmt::skip]
pub const _OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.0,
    0.0, 0.0, 0.5, 1.0,
);

impl GfxState {
    pub fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let screen_scale_factor = window.scale_factor() as f32;

        // The instance's main purpose is to create Adapters and Surfaces
        // Backends::all => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // The surface is the part of the window that we draw to.
        let surface = instance.create_surface(window.clone()).unwrap();

        let (adapter, device, queue) =
            async { Self::load_adapter_device_queue(&instance, &surface).await }.block_on();

        let surface_caps = surface.get_capabilities(&adapter);

        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            // How SurfaceTextures will be used.
            // RENDER_ATTACHMENT specifies that the textures fill be used to write to the screen.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            // How SurfaceTextures fill be stored on the GPU.
            format: surface_format,
            // width and height in pixels of a SurfaceTexture
            width: size.width,
            height: size.height,
            // How to sync the surface with the display.
            // Fifo = VSync
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        let surface_dimensions_px_uniform = SurfaceDimensionsUniform {
            width: size.width,
            height: size.height,
            scale_factor: screen_scale_factor,
        };

        let surface_dimensions_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Surface Dimensions Buffer"),
                contents: bytemuck::cast_slice(&[surface_dimensions_px_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let surface_dimensions_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("surface_dimensions_bind_group_layout"),
            });

        let surface_dimensions_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &surface_dimensions_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: surface_dimensions_buffer.as_entire_binding(),
            }],
            label: Some("surface_dimensions_bind_group"),
        });

        let debug_line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Debug line shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("debug-line-shader.wgsl").into()),
        });

        let debug_line_render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Debug line renderer pipeline layout"),
                bind_group_layouts: &[&surface_dimensions_bind_group_layout],
                push_constant_ranges: &[],
            });

        let debug_line_render_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Debug line renderer pipeline"),
                layout: Some(&debug_line_render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &debug_line_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &debug_line_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
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

        let debug_line_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("debug_line_vertex_buffer"),
            size: (256 as usize * std::mem::size_of::<[f32; 2]>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let text_renderer = TextRenderer::new(
            &device,
            &config,
            &surface_dimensions_bind_group_layout,
            size.width,
            size.height,
            screen_scale_factor,
        );

        let line_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("line_vertex_buffer"),
            size: (4000 as usize * std::mem::size_of::<LineVertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let line_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Line Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("line-shader.wgsl").into()),
        });

        let line_render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Line Render Pipeline Layout"),
                bind_group_layouts: &[&surface_dimensions_bind_group_layout],
                push_constant_ranges: &[],
            });

        let line_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Line Render Pipeline"),
            layout: Some(&line_render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &line_shader,
                entry_point: Some("vs_main"),
                // What type of vertices we want to pass to the vertex shader.
                buffers: &[LineVertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &line_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
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

        GfxState {
            _window: window,
            surface,
            device,
            queue,
            config,
            size,
            screen_scale_factor,
            debug_line_vertex_buffer,
            debug_line_render_pipeline,
            text_renderer,
            _line_vertex_buffer: line_vertex_buffer,
            _line_render_pipeline: line_render_pipeline,
            surface_dimensions_buffer,
            surface_dimensions_bind_group,
        }
    }

    async fn load_adapter_device_queue(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
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
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty()
                    .union(wgpu::Features::DUAL_SOURCE_BLENDING),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                label: None,
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off,
            })
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

            let surface_dimensions_px_uniform = SurfaceDimensionsUniform {
                width: new_size_apply.width,
                height: new_size_apply.height,
                scale_factor: self.screen_scale_factor,
            };
            self.queue.write_buffer(
                &self.surface_dimensions_buffer,
                0,
                bytemuck::cast_slice(&[surface_dimensions_px_uniform]),
            );
            self.text_renderer.surface_resized(
                new_size_apply.width,
                new_size_apply.height,
                self.screen_scale_factor,
            );
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

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Debug line render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    // The view to save the colors to. In this case, the screen.
                    view: &view,
                    // Target that will receive the resolved output. Is the same as `view` unless multisampling is enabled.
                    resolve_target: None,
                    // What to do with the colors on the view (i.e. the screen)
                    ops: wgpu::Operations {
                        // Load tells wgpu how to handle colors stored from the previous frame (we clear the screen)
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0, //242.0 / 255.0,
                            g: 1.0, //240.0 / 255.0,
                            b: 1.0, //239.0 / 255.0,
                            a: 1.0,
                        }),
                        // We want to store the rendered results to the (Surface)Texture behind the TextureView (the view)
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            let line_vertices: [[f32; 2]; 2] = [
                [
                    -1.0,
                    1.0 - logical_px_to_screen_surface_offset(
                        257,
                        self.size.height,
                        self.screen_scale_factor,
                    ),
                ],
                [
                    1.0,
                    1.0 - logical_px_to_screen_surface_offset(
                        257,
                        self.size.height,
                        self.screen_scale_factor,
                    ),
                ],
            ];

            /*self.queue.write_buffer(
                &self.debug_line_vertex_buffer,
                0,
                bytemuck::cast_slice(&line_vertices),
            );*/

            render_pass.set_pipeline(&self.debug_line_render_pipeline);
            render_pass.set_bind_group(0, &self.surface_dimensions_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.debug_line_vertex_buffer.slice(..));
            render_pass.draw(0..line_vertices.len() as u32, 0..1);
        }

        // begin_render_pass borrows encoder mutably, so we need to make sure that the borrow
        // is dropped before we can call encoder.finish()
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Glyph Render Pass"),
                color_attachments: &[
                    // This is what [[location(0)]] in the fragment shader targets
                    Some(wgpu::RenderPassColorAttachment {
                        // The view to save the colors to. In this case, the screen.
                        view: &view,
                        // Target that will receive the resolved output. Is the same as `view` unless multisampling is enabled.
                        resolve_target: None,
                        // What to do with the colors on the view (i.e. the screen)
                        ops: wgpu::Operations {
                            // Load tells wgpu how to handle colors stored from the previous frame (load from previous render pass)
                            load: wgpu::LoadOp::Load,
                            // We want to store the rendered results to the (Surface)Texture behind the TextureView (the view)
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            self.text_renderer.render(
                &game_state,
                render_pass,
                &self.surface_dimensions_bind_group,
                &self.queue,
            );
        }
        /*
        // begin_render_pass borrows encoder mutably, so we need to make sure that the borrow
        // is dropped before we can call encoder.finish()
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Line Render Pass"),
                color_attachments: &[
                    // This is what [[location(0)]] in the fragment shader targets
                    Some(wgpu::RenderPassColorAttachment {
                        // The view to save the colors to. In this case, the screen.
                        view: &view,
                        // Target that will receive the resolved output. Is the same as `view` unless multisampling is enabled.
                        resolve_target: None,
                        // What to do with the colors on the view (i.e. the screen)
                        ops: wgpu::Operations {
                            // Load tells wgpu how to handle colors stored from the previous frame (keep what we have)
                            load: wgpu::LoadOp::Load,
                            // We want to store the rendered results to the (Surface)Texture behind the TextureView (the view)
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    }),
                ],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            /*let line_vertices = Self::generate_line_vertices(
                &vec![
                    [0.0, 0.0, 0.0],
                    [0.5, 0.5, 0.0],
                    [0.5, 0.0, 0.0],
                    [0.75, 0.5, 0.0],
                    [0.75, 0.0, 0.0],
                ],
                10.0,
            );*/

            let line_vertices: Vec<LineVertex> = if let Some(multiline) = game_state.test_multiline
            {
                self.generate_line_vertices(&Vec::from(multiline), 10.0)
            } else {
                Vec::new()
            };

            self.queue.write_buffer(
                &self.line_vertex_buffer,
                0,
                bytemuck::cast_slice(&line_vertices[..]),
            );

            render_pass.set_pipeline(&self.line_render_pipeline);
            render_pass.set_bind_group(0, &self.surface_dimensions_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.line_vertex_buffer.slice(..));
            render_pass.draw(0..line_vertices.len() as u32, 0..1);
        }
        */
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    fn _generate_line_vertices(
        &self,
        positions: &Vec<[f32; 3]>,
        thickness: f32,
    ) -> Vec<LineVertex> {
        assert!(positions.len() > 1);

        let mut vertices: Vec<LineVertex> = Vec::with_capacity((positions.len() - 1) * 6);
        let scaled_thickness: f32 = thickness * self.screen_scale_factor;

        for i in 0..positions.len() - 1 {
            let position = positions[i];

            let previous_point = if i > 0 {
                positions[i - 1]
            } else {
                [-2.0, -2.0, 0.0]
            };

            let next_point = positions[i + 1];

            let next_next_point = if i < positions.len() - 2 {
                positions[i + 2]
            } else {
                [2.0, 2.0, 0.0]
            };

            vertices.push(LineVertex {
                position,
                previous_point,
                next_point,
                thickness: scaled_thickness,
                miter_dir: -1.0,
            });
            vertices.push(LineVertex {
                position,
                previous_point,
                next_point,
                thickness: scaled_thickness,
                miter_dir: 1.0,
            });
            vertices.push(LineVertex {
                position: next_point,
                previous_point: position,
                next_point: next_next_point,
                thickness: scaled_thickness,
                miter_dir: 1.0,
            });
            vertices.push(LineVertex {
                position: next_point,
                previous_point: position,
                next_point: next_next_point,
                thickness: scaled_thickness,
                miter_dir: -1.0,
            });
            vertices.push(LineVertex {
                position: next_point,
                previous_point: position,
                next_point: next_next_point,
                thickness: scaled_thickness,
                miter_dir: 1.0,
            });
            vertices.push(LineVertex {
                position,
                previous_point,
                next_point,
                thickness: scaled_thickness,
                miter_dir: -1.0,
            });
        }

        vertices
    }
}

// OPENGL CORDINATES ARE CENTERED AND GO FROM -1.0 TO +1.0
// !!!!! THIS MEANS THAT THE LOGICAL WIDTH AND HEIGHT OF THE SCREEN IS *** 2 *** !!!!!
// DON'T FORGET THIS *AGAIN*

pub fn logical_px_to_screen_surface_offset(
    logical_px_offset: i16,
    surface_dimension: u32,
    surface_scale_factor: f32,
) -> f32 {
    2.0 * logical_px_offset as f32 * surface_scale_factor / surface_dimension as f32
}
