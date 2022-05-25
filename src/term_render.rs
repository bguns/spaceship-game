use std::io::{stdout, Stdout, Write};
use std::time::Duration;

use crossterm::{
    cursor, event,
    style::{self, Stylize},
    terminal::{self, size, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand, QueueableCommand,
};

use crate::error::Result;
use crate::GameState;

pub struct Renderer {
    stdout: Stdout,
}

impl Renderer {
    pub fn init() -> Result<Self> {
        let mut stdout = stdout();
        stdout.execute(EnterAlternateScreen)?;
        terminal::enable_raw_mode()?;
        stdout.execute(Clear(ClearType::All))?;
        stdout.execute(cursor::MoveToRow(3))?;

        Ok(Self { stdout })
    }

    pub fn render_frame(&mut self, game_state: &GameState) -> Result<()> {
        self.stdout.queue(cursor::MoveToColumn(1))?;
        self.stdout.queue(cursor::MoveToRow(1))?;
        self.stdout.queue(Clear(ClearType::All))?;
        self.stdout.queue(style::Print(&format!(
            "Frame {} processed in {} microseconds{}",
            game_state.frame_number,
            game_state.now.elapsed().as_micros(),
            ".".repeat((game_state.frame_number % 60) as usize)
        )))?;

        let (columns, rows) = size()?;
        if columns < 200 || rows < 50 {
            self.show_size_error(columns, rows)?;
        } else {
            self.stdout.queue(cursor::SavePosition)?;
            self.stdout.queue(cursor::MoveToRow(2))?;
            self.stdout.queue(cursor::MoveToColumn(1))?;
            self.stdout.queue(Clear(ClearType::CurrentLine))?;
            self.stdout.queue(style::Print(&format!(
                "Terminal width x height: {} x {}.",
                columns, rows
            )))?;
            self.stdout.queue(cursor::RestorePosition)?;
        }

        for c in game_state.keyboard_state.get_pressed_characters() {
            self.stdout.queue(style::Print(c))?;
        }

        self.stdout.flush()?;
        Ok(())
    }

    fn show_size_error(&mut self, current_columns: u16, current_rows: u16) -> Result<()> {
        let middle_column = current_columns / 2;
        let middle_row = current_rows / 2;
        let error_message = "Please resize your terminal window until it is at least 200 columns wide and 50 rows high.";
        let error_message_length = error_message.chars().count() as u16;
        let half_error_message_length = error_message_length / 2;
        let mut x = middle_column.saturating_sub(half_error_message_length);
        let mut y = middle_row.saturating_sub(1);

        let stdout = &mut self.stdout;
        stdout.queue(cursor::SavePosition)?;
        stdout.queue(cursor::MoveToColumn(x))?;
        stdout.queue(cursor::MoveToRow(y))?;
        stdout.queue(style::Print(error_message))?;

        y += 1;
        let current_size_message = &format!(
            "Current colums x rows: {} x {}",
            current_columns, current_rows
        );
        let half_current_size_message_length = current_size_message.chars().count() as u16 / 2;
        x = middle_column.saturating_sub(half_current_size_message_length);
        stdout.queue(cursor::MoveToColumn(x))?;
        stdout.queue(cursor::MoveToRow(y))?;
        stdout.queue(style::Print(current_size_message))?;

        stdout.queue(cursor::RestorePosition)?;

        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // We use device_query to get keyboard state, but this does not actually read the terminal stdin input.
        // If we don't "drain" the input, all the keys the user presses while running this, will appear
        // on the command line after exiting the application.
        while event::poll(Duration::from_millis(1)).unwrap_or(false) {
            let _ = event::read().expect("Unexpected crossterm error: event::read() returned Err after succesful event::poll.");
        }
        terminal::disable_raw_mode()
            .expect("Unexpected crossterm error: failed to disable raw mode on exit.");
        self.stdout
            .execute(LeaveAlternateScreen)
            .expect("Unexpected crossterm error: failed to leave alternate scree on exit.");
    }
}
