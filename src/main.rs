mod error;
mod gfx;
mod input;

use device_query::{DeviceState, Keycode};

use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

use std::time::{Duration, Instant};

use crate::error::{GameError, Result};
use crate::gfx::GfxState;
use crate::input::KeyboardState;

pub struct GameState {
    start_time: Instant,
    now: Instant,
    delta_time: Duration,
    run_time: Duration,
    frame_number: u64,
    keyboard_state: KeyboardState,
    should_quit: bool,
}

impl GameState {
    pub fn update(&mut self, now: Instant) -> Result<()> {
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
        Ok(())
    }
}

#[allow(unreachable_code)]
fn main() -> Result<()> {
    env_logger::init();
    let device_state = DeviceState::new();
    let keyboard_state = KeyboardState::new(device_state);

    let now = Instant::now();
    let mut game_state = GameState {
        start_time: now,
        now,
        delta_time: Duration::from_millis(0),
        run_time: Duration::from_millis(0),
        frame_number: 0,
        keyboard_state,
        should_quit: false,
    };

    let sixteen_millis = Duration::from_millis(16);

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Game")
        .with_inner_size(LogicalSize::new(1920.0, 1080.0))
        //.with_decorations(false)
        .build(&event_loop)
        .unwrap();

    let mut gfx_state = GfxState::new(&window);

    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent {
            ref event,
            window_id,
        } if window_id == window.id() => {
            match event {
                WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit
                }
                WindowEvent::Resized(physical_size) => {
                    gfx_state.resize(Some(*physical_size));
                }
                WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                    gfx_state.resize(Some(**new_inner_size));
                }
                _ => {}
            }
        }
        Event::MainEventsCleared => {
            let now = Instant::now();
            let previous_frame_start = game_state.now;
            if now - previous_frame_start < sixteen_millis {
                *control_flow = ControlFlow::WaitUntil(previous_frame_start + sixteen_millis)
            } else {
                game_state.update(now).unwrap();
                match gfx_state.render(Some("Welcome!")) {
                    Ok(_) => {}
                    // Reconfigure the surface if lost
                    Err(GameError::WgpuError(wgpu::SurfaceError::Lost)) => gfx_state.resize(None),
                    // Out of graphics memory probably means we should quit.
                    Err(GameError::WgpuError(wgpu::SurfaceError::OutOfMemory)) => *control_flow = ControlFlow::Exit,
                    Err(e) => eprintln!("{:?}", e),
                }

                let elapsed = now.elapsed();
                let max_fps = 1_000_000.0 / elapsed.as_micros() as f64;
                let fps = 1_000_000.0 / game_state.delta_time.as_micros() as f64;
                print!(
                    "\rElapsed time: {}; Runtime: {}; dt: {:.2}, Frame number: {}; FPS: {:.2}; Max FPS: {:.2}",
                    game_state.start_time.elapsed().as_millis(),
                    game_state.run_time.as_millis(),
                    game_state.delta_time.as_micros() as f64 / 1_000.0,
                    game_state.frame_number,
                    fps,
                    max_fps
                );

                if elapsed < sixteen_millis {
                    *control_flow = ControlFlow::WaitUntil(now + sixteen_millis - elapsed);
                } else {
                    *control_flow = ControlFlow::Poll;
                }

                if game_state.should_quit {
                    *control_flow = ControlFlow::Exit;
                }
            }
        }
        _ => {}
    });

    Ok(())
}
