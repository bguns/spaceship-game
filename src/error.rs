use std::fmt;

pub type Result<T> = std::result::Result<T, GameError>;

/// The error type for errors that can originate while running the Stellaris Tool.
#[derive(Debug)]
pub enum GameError {
    Error(String),
    WgpuError(wgpu::SurfaceError), //CrosstermError(crossterm::ErrorKind),
}

impl From<wgpu::SurfaceError> for GameError {
    fn from(e: wgpu::SurfaceError) -> Self {
        GameError::WgpuError(e)
    }
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", "[ERROR]")?;
        match &self {
            &GameError::Error(message) => {
                write!(f, "{}", message)
            }
            &GameError::WgpuError(message) => {
                write!(f, "Wgpu Error: {}", message)
            }
        }
    }
}
