use crossterm::{
    cursor, event, style,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand, QueueableCommand, Result,
};

use device_query::{DeviceQuery, DeviceState, Keycode};

use std::io::{stdout, Write};
use std::thread;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let mut stdout = stdout();
    stdout.execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    stdout.execute(cursor::MoveToRow(2))?;

    let one_ms = Duration::from_millis(1);
    let fifteen_millis = Duration::from_millis(15);

    let device_state = DeviceState::new();
    let mut keyboard_state = KeyboardState::new(device_state);

    let mut frame_number: u64 = 0;

    loop {
        let now = Instant::now();
        frame_number += 1;

        keyboard_state.update(frame_number);
        if keyboard_state.get_key_state(Keycode::LControl).is_down()
            && keyboard_state.get_key_state(Keycode::Q).is_down()
        {
            // We use device_query to get keyboard state, but this does not actually read the terminal stdin input.
            // If we don't "drain" the input, all the keys the user presses while running this, will appear
            // on the command line after exiting the application.
            while event::poll(one_ms)? {
                let _ = event::read()?;
            }
            break;
        }

        stdout.queue(cursor::SavePosition)?;
        stdout.queue(cursor::MoveToColumn(1))?;
        stdout.queue(cursor::MoveToRow(1))?;
        stdout.queue(style::Print(&format!(
            "Frame {} processed in {} microseconds. {}",
            frame_number,
            now.elapsed().as_micros(),
            ".".repeat((frame_number % 15) as usize)
        )))?;
        stdout.queue(cursor::RestorePosition)?;

        for c in keyboard_state.get_pressed_characters() {
            stdout.queue(style::Print(c))?;
        }

        stdout.flush()?;

        let elapsed = now.elapsed();
        if elapsed < fifteen_millis {
            thread::sleep(fifteen_millis - elapsed);
        }
    }

    terminal::disable_raw_mode()?;
    stdout.execute(LeaveAlternateScreen)?;
    Ok(())
}

struct KeyState {
    key_code: Keycode,
    down: bool,
    pressed: bool,
    last_pressed_frame: Option<u64>,
    released: bool,
    last_released_frame: Option<u64>,
    held: bool,
    held_since_frame: Option<u64>,
}

impl KeyState {
    pub fn new(key_code: Keycode) -> Self {
        Self {
            key_code,
            down: false,
            pressed: false,
            last_pressed_frame: None,
            released: false,
            last_released_frame: None,
            held: false,
            held_since_frame: None,
        }
    }

    pub fn is_down(&self) -> bool {
        self.down
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    pub fn _is_released(&self) -> bool {
        self.released
    }

    pub fn _is_held(&self) -> bool {
        self.held
    }

    pub fn update(&mut self, keys_down: &Vec<Keycode>, frame_number: u64) {
        let is_down = keys_down.contains(&self.key_code);
        self.pressed = !self.down && is_down;
        self.released = self.down && !is_down;
        self.held = self.down && is_down;
        if self.pressed {
            self.last_pressed_frame = Some(frame_number)
        }
        if self.released {
            self.last_released_frame = Some(frame_number)
        }
        if self.held && self.held_since_frame.is_none() {
            self.held_since_frame = Some(frame_number)
        } else if !self.held {
            self.held_since_frame = None
        }
        self.down = is_down;
    }
}

struct KeyboardState {
    device_state: DeviceState,
    character_keys: [(char, KeyState); 36],
    shift: KeyState,
    control: KeyState,
    alt: KeyState,
}

impl KeyboardState {
    pub fn new(device_state: DeviceState) -> Self {
        Self {
            device_state,
            character_keys: [
                ('0', KeyState::new(Keycode::Key0)),
                ('1', KeyState::new(Keycode::Key1)),
                ('2', KeyState::new(Keycode::Key2)),
                ('3', KeyState::new(Keycode::Key3)),
                ('4', KeyState::new(Keycode::Key4)),
                ('5', KeyState::new(Keycode::Key5)),
                ('6', KeyState::new(Keycode::Key6)),
                ('7', KeyState::new(Keycode::Key7)),
                ('8', KeyState::new(Keycode::Key8)),
                ('9', KeyState::new(Keycode::Key9)),
                ('a', KeyState::new(Keycode::A)),
                ('b', KeyState::new(Keycode::B)),
                ('c', KeyState::new(Keycode::C)),
                ('d', KeyState::new(Keycode::D)),
                ('e', KeyState::new(Keycode::E)),
                ('f', KeyState::new(Keycode::F)),
                ('g', KeyState::new(Keycode::G)),
                ('h', KeyState::new(Keycode::H)),
                ('i', KeyState::new(Keycode::I)),
                ('j', KeyState::new(Keycode::J)),
                ('k', KeyState::new(Keycode::K)),
                ('l', KeyState::new(Keycode::L)),
                ('m', KeyState::new(Keycode::M)),
                ('n', KeyState::new(Keycode::N)),
                ('o', KeyState::new(Keycode::O)),
                ('p', KeyState::new(Keycode::P)),
                ('q', KeyState::new(Keycode::Q)),
                ('r', KeyState::new(Keycode::R)),
                ('s', KeyState::new(Keycode::S)),
                ('t', KeyState::new(Keycode::T)),
                ('u', KeyState::new(Keycode::U)),
                ('v', KeyState::new(Keycode::V)),
                ('w', KeyState::new(Keycode::W)),
                ('x', KeyState::new(Keycode::X)),
                ('y', KeyState::new(Keycode::Y)),
                ('z', KeyState::new(Keycode::Z)),
            ],
            shift: KeyState::new(Keycode::LShift),
            control: KeyState::new(Keycode::LControl),
            alt: KeyState::new(Keycode::LAlt),
        }
    }

    pub fn get_key_state(&self, key_code: Keycode) -> &KeyState {
        match key_code {
            Keycode::LShift | Keycode::RShift => &self.shift,
            Keycode::LControl | Keycode::RControl => &self.control,
            Keycode::LAlt | Keycode::RAlt => &self.alt,
            _ => {
                for (_, key_state) in &self.character_keys {
                    if key_state.key_code == key_code {
                        return &key_state;
                    }
                }
                panic!(
                    "KeyboardState does not have a key state for keycode: {:?}",
                    key_code
                );
            }
        }
    }

    pub fn get_pressed_characters(&self) -> Vec<char> {
        let intermediate_iter = self
            .character_keys
            .iter()
            .filter(|(_, key_state)| key_state.is_pressed());
        if self.shift.is_down() {
            intermediate_iter
                .map(|(c, _)| c.clone().to_ascii_uppercase())
                .collect()
        } else {
            intermediate_iter.map(|(c, _)| c.clone()).collect()
        }
    }

    pub fn update(&mut self, frame_number: u64) {
        let mut keys: Vec<Keycode> = self.device_state.get_keys();
        for i in 0..keys.len() {
            match &keys[i] {
                &Keycode::RShift => keys[i] = Keycode::LShift,
                &Keycode::RControl => keys[i] = Keycode::LControl,
                &Keycode::RAlt => keys[i] = Keycode::LAlt,
                _ => continue,
            }
        }

        for i in 0..self.character_keys.len() {
            let (_, key_state) = &mut self.character_keys[i];
            key_state.update(&keys, frame_number);
        }

        self.shift.update(&keys, frame_number);
        self.control.update(&keys, frame_number);
        self.alt.update(&keys, frame_number);
    }
}
