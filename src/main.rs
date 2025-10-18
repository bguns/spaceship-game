mod error;
mod gfx;
mod input;

use device_query::{DeviceState, Keycode};

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use std::{
    f32::consts::PI,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::error::{GameError, Result};
use crate::gfx::GfxState;
use crate::input::KeyboardState;

use cgmath::prelude::*;

pub struct GameState {
    start_time: Instant,
    now: Instant,
    delta_time: Duration,
    run_time: Duration,
    frame_number: u64,
    keyboard_state: KeyboardState,
    text: Option<String>,
    test_multiline: Option<[[f32; 3]; 5]>,
    should_quit: bool,
}

impl GameState {
    pub fn update(&mut self, now: Instant, surface_size_x: f32, surface_size_y: f32) -> Result<()> {
        self.delta_time = now - self.now;
        self.run_time += self.delta_time;
        self.now = now;
        self.frame_number += 1;
        self.keyboard_state.update(self.frame_number);
        self.should_quit = self
            .keyboard_state
            .get_key_state(Keycode::LControl)
            .is_down()
            && self.keyboard_state.get_key_state(Keycode::Q).is_down();

        let slice_end = std::cmp::min(
            "Arrrrrrrrrrrrriverderci!".len(),
            (self.frame_number / 2) as usize,
        );
        self.text = Some("Arrrrrrrrrrrrriverderci!"[0..slice_end].to_string());
        self.test_multiline = Some(get_multiline(self.run_time, surface_size_x, surface_size_y));
        Ok(())
    }
}

#[derive(Default)]
struct App {
    gfx_state: Option<GfxState>,
    game_state: Option<GameState>,
}

impl App {
    fn new() -> Self {
        let device_state = DeviceState::new();
        let keyboard_state = KeyboardState::new(device_state);
        let now = Instant::now();
        Self {
            gfx_state: None,
            game_state: Some(GameState {
                start_time: now,
                now,
                delta_time: Duration::from_millis(0),
                run_time: Duration::from_millis(0),
                frame_number: 0,
                keyboard_state,
                text: Some("Arrrrrrrrrrrrriverderci!".to_string()),
                test_multiline: None,
                should_quit: false,
            }),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx_state.is_none() {
            let window_attributes = Window::default_attributes()
                .with_title("Game")
                .with_inner_size(LogicalSize::new(1440.0, 900.0));
            let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
            self.gfx_state = Some(GfxState::new(window.clone()));
            self.game_state.as_mut().unwrap().test_multiline = Some(get_multiline(
                Duration::from_millis(0),
                window.inner_size().width as f32,
                window.inner_size().height as f32,
            ));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let gfx_state = match &mut self.gfx_state {
            Some(state) => state,
            None => return,
        };

        let game_state = match &mut self.game_state {
            Some(state) => state,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(physical_size) => {
                gfx_state.resize(Some(physical_size));
            }
            WindowEvent::RedrawRequested => gfx_state.render(game_state).unwrap(),
            WindowEvent::ScaleFactorChanged { .. } => {
                gfx_state.resize(None);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let game_state = match &mut self.game_state {
            Some(state) => state,
            None => return,
        };
        let gfx_state = match &mut self.gfx_state {
            Some(state) => state,
            None => return,
        };

        let now = Instant::now();
        let sixteen_millis = Duration::from_millis(16);
        let previous_frame_start = game_state.now;
        if now - previous_frame_start < sixteen_millis {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                previous_frame_start + sixteen_millis,
            ));
        } else {
            game_state
                .update(
                    now,
                    gfx_state.window.inner_size().width as f32,
                    gfx_state.window.inner_size().height as f32,
                )
                .unwrap();
            gfx_state.window.request_redraw();
            match gfx_state.render(&game_state) {
                Ok(_) => {}
                // Reconfigure the surface if lost
                Err(GameError::WgpuError(wgpu::SurfaceError::Lost)) => gfx_state.resize(None),
                // Out of graphics memory probably means we should quit.
                Err(GameError::WgpuError(wgpu::SurfaceError::OutOfMemory)) => {
                    event_loop.exit();
                }
                Err(e) => eprintln!("{:?}", e),
            }

            let elapsed = now.elapsed();

            if elapsed < sixteen_millis {
                event_loop.set_control_flow(ControlFlow::WaitUntil(now + sixteen_millis - elapsed));
            } else {
                event_loop.set_control_flow(ControlFlow::Poll);
            }

            if game_state.should_quit {
                event_loop.exit();
            }
        }
    }
}

#[allow(unreachable_code)]
fn main() -> Result<()> {
    env_logger::init();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();

    event_loop.run_app(&mut app).unwrap();

    Ok(())
}

fn get_multiline(run_time: Duration, surface_size_x: f32, surface_size_y: f32) -> [[f32; 3]; 5] {
    let runtime_seconds: f32 = run_time.as_millis() as f32 / 1000.0;

    let t = runtime_seconds * 2.0;

    let moving_x_one = 0.25 + 0.125 * t.sin();
    let moving_x_two = 0.5 + 0.125 * t.cos();

    let aspect_ratio = surface_size_x / surface_size_y;

    let rotation: cgmath::Basis2<f32> = Rotation2::from_angle(cgmath::Rad(2.0 * PI * (t / 4.0)));
    let mut rotated_vector = rotation.rotate_vector(cgmath::Vector2::unit_x());
    rotated_vector.x /= aspect_ratio;

    let first = [0.0, 0.0, 0.0];
    let second = [moving_x_one, 0.5, 0.0];
    //let second = [0.5, 0.5, 0.0];
    let third = [moving_x_one, 0.0, 0.0];
    //let third = [0.5, 0.0, 0.0];
    let fourth = [moving_x_two, 0.5, 0.0];
    //let fourth = [0.75, 0.5, 0.0];

    let fifth = [
        moving_x_two + (0.125 * rotated_vector.x),
        0.5 + (0.125 * rotated_vector.y),
        0.0,
    ];

    [first, second, third, fourth, fifth]
}
