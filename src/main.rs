mod error;
mod gfx;
mod input;
#[cfg_attr(windows, path = "os/windows/mod.rs")]
mod os;

use anyhow::Result;

use device_query::{DeviceState, Keycode};

use cgmath::prelude::*;
use gfx::text::ShaperSettings;
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

use crate::error::GameError;
use crate::gfx::GfxState;
use crate::gfx::text::FontCache;
use crate::input::KeyboardState;

const SIXTEEN_MILLIS: Duration = Duration::from_millis(16);

#[allow(dead_code)]
pub struct GameState {
    start_time: Instant,
    now: Instant,
    delta_time: Duration,
    run_time: Duration,
    state_number: u64,
    frame_number: u64,
    keyboard_state: KeyboardState,
    font_cache: FontCache,
    text: Option<String>,
    test_multiline: Option<[[f32; 3]; 5]>,
    should_quit: bool,
}

impl GameState {
    pub fn update(&mut self, now: Instant) -> Result<()> {
        self.delta_time = now - self.now;
        self.run_time += self.delta_time;
        self.now = now;
        self.state_number += 1;
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
        //self.test_multiline = Some(get_multiline(self.run_time, surface_size_x, surface_size_y));
        Ok(())
    }

    #[inline]
    pub fn should_update(&self, now: &Instant) -> bool {
        self.frame_number == self.state_number && *now - self.now >= SIXTEEN_MILLIS
    }
}

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    gfx_state: Option<GfxState>,
    game_state: Option<GameState>,
}

impl App {
    fn new() -> Self {
        let device_state = DeviceState::new();
        let keyboard_state = KeyboardState::new(device_state);
        let now = Instant::now();
        let mut font_cache = FontCache::new();
        font_cache
            .load_system_fonts()
            .expect("Unable to load system fonts");
        font_cache
            .load_font_file("./fonts/SourceSerifVariable-Roman.ttf")
            .expect("Unable to load source serif variable font file");
        Self {
            window: None,
            gfx_state: None,
            game_state: Some(GameState {
                start_time: now,
                now,
                delta_time: Duration::from_millis(0),
                run_time: Duration::from_millis(0),
                frame_number: 0,
                state_number: 1,
                keyboard_state,
                font_cache,
                text: Some("Arrrrrrrrrrrrriverderci!".to_string()),
                test_multiline: None,
                should_quit: false,
            }),
        }
    }

    fn should_render(&self) -> bool {
        if let Some(game_state) = &self.game_state
            && self.window.is_some()
        {
            game_state.state_number > game_state.frame_number
        } else {
            false
        }
    }

    fn window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attributes = Window::default_attributes()
                .with_title("Game")
                .with_inner_size(LogicalSize::new(1440.0, 900.0));
            let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
            self.window = Some(window.clone());
            self.gfx_state = Some(GfxState::new(window.clone()));
            /*self.game_state.as_mut().unwrap().test_multiline = Some(get_multiline(
                Duration::from_millis(0),
                window.inner_size().width as f32,
                window.inner_size().height as f32,
            ));*/
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: winit::event::StartCause) {
        if cause == winit::event::StartCause::Init {
            return;
        }
        let game_state = match &mut self.game_state {
            Some(state) => state,
            None => return,
        };
        let now = Instant::now();
        if game_state.should_update(&now) {
            game_state.update(now).unwrap();
        }
        if self.should_render() {
            self.window().unwrap().request_redraw();
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
            WindowEvent::RedrawRequested => match gfx_state.render(&game_state) {
                Ok(_) => game_state.frame_number += 1,
                Err(e) => match e.downcast_ref::<GameError>() {
                    // Reconfigure the surface if lost
                    Some(GameError::WgpuError(wgpu::SurfaceError::Lost)) => gfx_state.resize(None),
                    // Out of graphics memory probably means we should quit.
                    Some(GameError::WgpuError(wgpu::SurfaceError::OutOfMemory)) => {
                        event_loop.exit()
                    }
                    _ => eprintln!("{:?}", e),
                },
            },
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

        if game_state.should_quit {
            event_loop.exit();
        }
    }
}

#[allow(unreachable_code)]
fn main() -> Result<()> {
    rayon::ThreadPoolBuilder::new().build_global()?;
    env_logger::init();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();

    event_loop.run_app(&mut app).unwrap();

    Ok(())
}

fn _get_multiline(run_time: Duration, surface_size_x: f32, surface_size_y: f32) -> [[f32; 3]; 5] {
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
