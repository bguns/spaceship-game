mod error;
mod input;

use device_query::{DeviceState, Keycode};

use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

use std::time::{Duration, Instant};

use crate::error::Result;
use crate::input::KeyboardState;

pub struct GameState {
    frame_number: u64,
    now: Instant,
    keyboard_state: KeyboardState,
    should_quit: bool,
}

impl GameState {
    pub fn update(&mut self) -> Result<()> {
        self.now = Instant::now();
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
    let device_state = DeviceState::new();
    let keyboard_state = KeyboardState::new(device_state);
    let mut game_state = GameState {
        frame_number: 0,
        now: Instant::now(),
        keyboard_state,
        should_quit: false,
    };

    let sixteen_millis = Duration::from_millis(16);

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            println!("The close button was pressed; stopping");
            *control_flow = ControlFlow::Exit
        }
        Event::MainEventsCleared => {
            let previous_frame_start = game_state.now.clone();
            game_state.update().unwrap();

            let elapsed = game_state.now.elapsed();
            let max_fps = 1_000_000.0 / elapsed.as_micros() as f64;
            let fps = 1_000_000.0 / (game_state.now - previous_frame_start).as_micros() as f64;
            print!(
                "\rFrame number: {}; FPS: {:.2}; Max FPS: {:.2}",
                game_state.frame_number, fps, max_fps
            );
            if elapsed < sixteen_millis {
                *control_flow = ControlFlow::WaitUntil(game_state.now + sixteen_millis - elapsed);
            } else {
                *control_flow = ControlFlow::Poll;
            }

            if game_state.should_quit {
                println!("Should quit");
                *control_flow = ControlFlow::Exit;
            }
        }
        _ => {}
    });

    Ok(())
}
