mod error;
mod input;

use crossterm::{
    cursor, event,
    style::{self, Stylize},
    terminal::{self, size, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand, QueueableCommand,
};

use device_query::{DeviceState, Keycode};

use std::io::{stdout, Write};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Result;
use crate::input::KeyboardState;

fn main() -> Result<()> {
    let mut stdout = stdout();
    stdout.execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    stdout.execute(Clear(ClearType::All))?;
    stdout.execute(cursor::MoveToRow(3))?;

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
        stdout.queue(Clear(ClearType::CurrentLine))?;
        stdout.queue(style::Print(&format!(
            "Frame {} processed in {} microseconds{}",
            frame_number,
            now.elapsed().as_micros(),
            ".".repeat((frame_number % 60) as usize)
        )))?;
        stdout.queue(cursor::MoveToRow(2))?;
        stdout.queue(cursor::MoveToColumn(1))?;
        stdout.queue(Clear(ClearType::CurrentLine))?;
        let (columns, rows) = size()?;
        stdout.queue(style::Print(&format!(
            "Terminal width x height: {} x {}. Desired dimensions: ",
            columns, rows
        )))?;
        if columns < 200 || rows < 50 {
            stdout.queue(style::PrintStyledContent("200 x 50".red()))?;
        } else {
            stdout.queue(style::PrintStyledContent("200 x 50".green()))?;
        }
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
