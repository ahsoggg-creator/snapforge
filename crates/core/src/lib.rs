//! SnapForge core: capture, editor model, history, post-capture pipeline and plugins. No UI.
#![allow(clippy::chunks_exact_to_as_chunks)]

pub mod capture;
pub mod diag;
pub mod editor;
pub mod face;
pub mod geometry;
pub mod history;
pub mod hotkey;
pub mod imageops;
pub mod naming;
pub mod net;
#[cfg(windows)]
pub mod ocr;
pub mod output;
pub mod paths;
pub mod plugin;
pub mod post;
pub mod secrets;
pub mod sensitive;
pub mod settings;
pub mod upload;

pub use image;
pub use image::RgbaImage;
