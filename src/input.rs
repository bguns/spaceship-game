use device_query::{DeviceQuery, DeviceState, Keycode};

pub struct KeyboardState {
    device_state: DeviceState,
    character_keys: [KeyState; 38],
    shift: KeyState,
    control: KeyState,
    alt: KeyState,
}

impl KeyboardState {
    pub fn new(device_state: DeviceState) -> Self {
        Self {
            device_state,
            character_keys: [
                KeyState::new(Keycode::Key0),
                KeyState::new(Keycode::Key1),
                KeyState::new(Keycode::Key2),
                KeyState::new(Keycode::Key3),
                KeyState::new(Keycode::Key4),
                KeyState::new(Keycode::Key5),
                KeyState::new(Keycode::Key6),
                KeyState::new(Keycode::Key7),
                KeyState::new(Keycode::Key8),
                KeyState::new(Keycode::Key9),
                KeyState::new(Keycode::A),
                KeyState::new(Keycode::B),
                KeyState::new(Keycode::C),
                KeyState::new(Keycode::D),
                KeyState::new(Keycode::E),
                KeyState::new(Keycode::F),
                KeyState::new(Keycode::G),
                KeyState::new(Keycode::H),
                KeyState::new(Keycode::I),
                KeyState::new(Keycode::J),
                KeyState::new(Keycode::K),
                KeyState::new(Keycode::L),
                KeyState::new(Keycode::M),
                KeyState::new(Keycode::N),
                KeyState::new(Keycode::O),
                KeyState::new(Keycode::P),
                KeyState::new(Keycode::Q),
                KeyState::new(Keycode::R),
                KeyState::new(Keycode::S),
                KeyState::new(Keycode::T),
                KeyState::new(Keycode::U),
                KeyState::new(Keycode::V),
                KeyState::new(Keycode::W),
                KeyState::new(Keycode::X),
                KeyState::new(Keycode::Y),
                KeyState::new(Keycode::Z),
                KeyState::new(Keycode::LeftBracket),
                KeyState::new(Keycode::RightBracket),
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
                for key_state in &self.character_keys {
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
        return vec![];
        /*let intermediate_iter = self
            .character_keys
            .iter()
            .filter(|(_, key_state)| key_state.is_pressed());
        if self.shift.is_down() {
            intermediate_iter
                .map(|(c, _)| c.clone().to_ascii_uppercase())
                .collect()
        } else {
            intermediate_iter.map(|(c, _)| c.clone()).collect()
        }*/
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
            let key_state = &mut self.character_keys[i];
            key_state.update(&keys, frame_number);
        }

        self.shift.update(&keys, frame_number);
        self.control.update(&keys, frame_number);
        self.alt.update(&keys, frame_number);
    }
}

pub struct KeyState {
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
