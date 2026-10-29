//! Native desktop front-end for the viewarr array/image viewer.
//!
//! The `viewarr` binary is a thin wrapper over this library: [`fits`] reads
//! array data off disk and [`app`] hosts viewarr's `ArrayViewerWidget` in an
//! eframe window.

pub mod app;
pub mod fits;
pub mod header;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod source;
pub mod tab;
