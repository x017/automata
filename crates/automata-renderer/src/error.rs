//! Renderer error types.

use thiserror::Error;

/// Errors produced by the renderer.
#[derive(Debug, Error)]
pub enum Error {
    /// The window surface could not be created.
    #[error("failed to create window surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),

    /// The GPU adapter request failed.
    #[error("failed to acquire a GPU adapter: {0}")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),

    /// The adapter rejected the device request.
    #[error("failed to create GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),

    /// The adapter cannot present to the window surface.
    #[error("window surface is not compatible with the selected GPU adapter")]
    IncompatibleSurface,

    /// A validation error occurred while acquiring the next frame.
    #[error("validation error while acquiring the next frame")]
    SurfaceValidation,

    /// An image could not be decoded.
    #[error("failed to decode image: {0}")]
    Image(#[from] image::ImageError),
}

/// Convenience alias for results returned by the renderer.
pub type Result<T> = std::result::Result<T, Error>;
