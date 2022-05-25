mod error;
mod input;
mod term_render;

use device_query::{DeviceState, Keycode};

use std::thread;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::input::KeyboardState;
use crate::term_render::Renderer;

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

fn main() -> Result<()> {
    let mut renderer = Renderer::init()?;
    let device_state = DeviceState::new();
    let keyboard_state = KeyboardState::new(device_state);
    let mut game_state = GameState {
        frame_number: 0,
        now: Instant::now(),
        keyboard_state,
        should_quit: false,
    };

    let fifteen_millis = Duration::from_millis(15);

    loop {
        game_state.update()?;

        if game_state.should_quit {
            break;
        }

        renderer.render_frame(&game_state)?;

        let elapsed = game_state.now.elapsed();
        if elapsed < fifteen_millis {
            thread::sleep(fifteen_millis - elapsed);
        }
    }

    Ok(())
}
