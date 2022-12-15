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

use std::{
    f32::consts::PI,
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

#[allow(unreachable_code)]
fn main() -> Result<()> {
    env_logger::init();
    let device_state = DeviceState::new();
    let keyboard_state = KeyboardState::new(device_state);

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Game")
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        //.with_decorations(false)
        .build(&event_loop)
        .unwrap();

    let mut gfx_state = GfxState::new(&window);

    let now = Instant::now();
    let mut game_state = GameState {
        start_time: now,
        now,
        delta_time: Duration::from_millis(0),
        run_time: Duration::from_millis(0),
        frame_number: 0,
        keyboard_state,
        text: Some("Arrrrrrrrrrrrriverderci!".to_string()),
        test_multiline: Some(get_multiline(
            Duration::from_millis(0),
            window.inner_size().width as f32,
            window.inner_size().height as f32,
        )),
        should_quit: false,
    };

    let sixteen_millis = Duration::from_millis(16);

    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent {
            ref event,
            window_id,
        } if window_id == window.id() => match event {
            WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
            WindowEvent::Resized(physical_size) => {
                gfx_state.resize(Some(*physical_size));
            }
            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                gfx_state.resize(Some(**new_inner_size));
            }
            _ => {}
        },
        Event::MainEventsCleared => {
            let now = Instant::now();
            let previous_frame_start = game_state.now;
            if now - previous_frame_start < sixteen_millis {
                *control_flow = ControlFlow::WaitUntil(previous_frame_start + sixteen_millis)
            } else {
                game_state
                    .update(
                        now,
                        window.inner_size().width as f32,
                        window.inner_size().height as f32,
                    )
                    .unwrap();
                match gfx_state.render(&game_state) {
                    Ok(_) => {}
                    // Reconfigure the surface if lost
                    Err(GameError::WgpuError(wgpu::SurfaceError::Lost)) => gfx_state.resize(None),
                    // Out of graphics memory probably means we should quit.
                    Err(GameError::WgpuError(wgpu::SurfaceError::OutOfMemory)) => {
                        *control_flow = ControlFlow::Exit
                    }
                    Err(e) => eprintln!("{:?}", e),
                }

                let elapsed = now.elapsed();

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
