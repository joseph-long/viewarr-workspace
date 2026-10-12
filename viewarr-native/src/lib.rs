//! Native desktop front-end for the viewarr array/image viewer.
//!
//! The `viewarr` binary is a thin wrapper over this library: [`fits`] reads
//! array data off disk and [`app`] hosts viewarr's `ArrayViewerWidget` in an
//! eframe window.

pub mod app;
pub mod fits;
