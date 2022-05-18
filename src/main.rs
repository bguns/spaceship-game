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

    loop {
        let now = Instant::now();

        let keys: Vec<Keycode> = device_state.get_keys();
        if (keys.contains(&Keycode::LControl) || keys.contains(&Keycode::RControl))
            && keys.contains(&Keycode::Q)
        {
            // We use device_query to get keyboard state, but this does not drain the terminal stdin input.
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
            "Frame processed in {} microseconds.",
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
