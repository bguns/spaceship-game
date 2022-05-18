use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    style,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand, QueueableCommand, Result,
};
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

    'outer: loop {
        let now = Instant::now();
        while event::poll(one_ms)? {
            if let Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                ..
            }) = event::read()?
            {
                match c {
                    'q' => break 'outer,
                    _ => {
                        let _ = stdout.execute(style::Print(c))?;
                    }
                }
            }
        }

        stdout.queue(cursor::SavePosition)?;
        stdout.queue(cursor::MoveToColumn(1))?;
        stdout.queue(cursor::MoveToRow(1))?;
        stdout.queue(style::Print(&format!(
            "Frame processed in {} microseconds.",
            now.elapsed().as_micros()
        )))?;
        stdout.queue(cursor::RestorePosition)?;

        stdout.flush()?;

        stdout.execute(style::Print('.'))?;
        thread::sleep(fifteen_millis - now.elapsed());
    }

    terminal::disable_raw_mode()?;
    stdout.execute(LeaveAlternateScreen)?;
    Ok(())
}
