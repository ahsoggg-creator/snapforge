//! SnapForge core: capture, editor model, history, post-capture pipeline and plugins. No UI.
#![allow(clippy::chunks_exact_to_as_chunks)]

pub mod capture;
pub mod diag;
pub mod editor;
pub mod geometry;
pub mod history;
pub mod hotkey;
pub mod naming;
pub mod output;
pub mod paths;
pub mod settings;

pub use image;
pub use image::RgbaImage;
