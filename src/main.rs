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
        if keyboard_state.control.down && keyboard_state.q.down {
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
            "Frame {} processed in {} microseconds.",
            frame_number,
            now.elapsed().as_micros()
        )))?;
        stdout.queue(cursor::RestorePosition)?;
        stdout.queue(style::Print('.'))?;

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
    pub down: bool,
    pub pressed: bool,
    last_pressed_frame: Option<u64>,
    pub released: bool,
    last_released_frame: Option<u64>,
    pub held: bool,
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
    pub top_0: KeyState,
    pub top_1: KeyState,
    pub top_2: KeyState,
    pub top_3: KeyState,
    pub top_4: KeyState,
    pub top_5: KeyState,
    pub top_6: KeyState,
    pub top_7: KeyState,
    pub top_8: KeyState,
    pub top_9: KeyState,
    pub a: KeyState,
    pub b: KeyState,
    pub c: KeyState,
    pub d: KeyState,
    pub e: KeyState,
    pub f: KeyState,
    pub g: KeyState,
    pub h: KeyState,
    pub i: KeyState,
    pub j: KeyState,
    pub k: KeyState,
    pub l: KeyState,
    pub m: KeyState,
    pub n: KeyState,
    pub o: KeyState,
    pub p: KeyState,
    pub q: KeyState,
    pub r: KeyState,
    pub s: KeyState,
    pub t: KeyState,
    pub u: KeyState,
    pub v: KeyState,
    pub w: KeyState,
    pub x: KeyState,
    pub y: KeyState,
    pub z: KeyState,
    pub shift: KeyState,
    pub control: KeyState,
    pub alt: KeyState,
}

impl KeyboardState {
    pub fn new(device_state: DeviceState) -> Self {
        Self {
            device_state,
            top_0: KeyState::new(Keycode::Key0),
            top_1: KeyState::new(Keycode::Key1),
            top_2: KeyState::new(Keycode::Key2),
            top_3: KeyState::new(Keycode::Key3),
            top_4: KeyState::new(Keycode::Key4),
            top_5: KeyState::new(Keycode::Key5),
            top_6: KeyState::new(Keycode::Key6),
            top_7: KeyState::new(Keycode::Key7),
            top_8: KeyState::new(Keycode::Key8),
            top_9: KeyState::new(Keycode::Key9),
            a: KeyState::new(Keycode::A),
            b: KeyState::new(Keycode::B),
            c: KeyState::new(Keycode::C),
            d: KeyState::new(Keycode::D),
            e: KeyState::new(Keycode::E),
            f: KeyState::new(Keycode::F),
            g: KeyState::new(Keycode::G),
            h: KeyState::new(Keycode::H),
            i: KeyState::new(Keycode::I),
            j: KeyState::new(Keycode::J),
            k: KeyState::new(Keycode::K),
            l: KeyState::new(Keycode::L),
            m: KeyState::new(Keycode::M),
            n: KeyState::new(Keycode::N),
            o: KeyState::new(Keycode::O),
            p: KeyState::new(Keycode::P),
            q: KeyState::new(Keycode::Q),
            r: KeyState::new(Keycode::R),
            s: KeyState::new(Keycode::S),
            t: KeyState::new(Keycode::T),
            u: KeyState::new(Keycode::U),
            v: KeyState::new(Keycode::V),
            w: KeyState::new(Keycode::W),
            x: KeyState::new(Keycode::X),
            y: KeyState::new(Keycode::Y),
            z: KeyState::new(Keycode::Z),
            shift: KeyState::new(Keycode::LShift),
            control: KeyState::new(Keycode::LControl),
            alt: KeyState::new(Keycode::LAlt),
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
        self.top_0.update(&keys, frame_number);
        self.top_1.update(&keys, frame_number);
        self.top_2.update(&keys, frame_number);
        self.top_3.update(&keys, frame_number);
        self.top_4.update(&keys, frame_number);
        self.top_5.update(&keys, frame_number);
        self.top_6.update(&keys, frame_number);
        self.top_7.update(&keys, frame_number);
        self.top_8.update(&keys, frame_number);
        self.top_9.update(&keys, frame_number);
        self.a.update(&keys, frame_number);
        self.b.update(&keys, frame_number);
        self.c.update(&keys, frame_number);
        self.d.update(&keys, frame_number);
        self.e.update(&keys, frame_number);
        self.f.update(&keys, frame_number);
        self.g.update(&keys, frame_number);
        self.h.update(&keys, frame_number);
        self.i.update(&keys, frame_number);
        self.j.update(&keys, frame_number);
        self.k.update(&keys, frame_number);
        self.l.update(&keys, frame_number);
        self.m.update(&keys, frame_number);
        self.n.update(&keys, frame_number);
        self.o.update(&keys, frame_number);
        self.p.update(&keys, frame_number);
        self.q.update(&keys, frame_number);
        self.r.update(&keys, frame_number);
        self.s.update(&keys, frame_number);
        self.t.update(&keys, frame_number);
        self.u.update(&keys, frame_number);
        self.v.update(&keys, frame_number);
        self.w.update(&keys, frame_number);
        self.x.update(&keys, frame_number);
        self.y.update(&keys, frame_number);
        self.z.update(&keys, frame_number);
        self.shift.update(&keys, frame_number);
        self.control.update(&keys, frame_number);
        self.alt.update(&keys, frame_number);
    }
}
