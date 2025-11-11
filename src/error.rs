use thiserror::Error;

//pub type Result<T> = std::result::Result<T, GameError>;

/// The error type for errors that can originate while running the Stellaris Tool.
#[derive(Error, Debug)]
pub enum GameError {
    #[error("[ERROR] {0}")]
    _Error(String),
    #[error("[ERROR] Wgpu Error: {0}")]
    WgpuError(#[from] wgpu::SurfaceError), //CrosstermError(crossterm::ErrorKind),
}
