use thiserror::Error;

#[derive(Error, Clone, Debug)]
pub enum SlickscreenError {
    #[error("Slickscreen is already running")]
    AlreadyRunning,

    #[error("Unable to initialize FFmpeg library")]
    FFmpegInitError,
    #[error("Audio encoder not found")]
    AudioEncoderNotFound,
    #[error("Unable to configure audio capture")]
    AudioCaptureError(String),
    #[error("Video encoder not found: {0}")]
    VideoEncoderNotFound(String),
    #[error("Unable to configure screen capture")]
    ScreenCaptureError(String),

    #[error("unexpected error")]
    Unexpected,
}
