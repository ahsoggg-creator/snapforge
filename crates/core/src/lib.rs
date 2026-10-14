//! SnapForge core: capture, editor model, history, post-capture pipeline and plugins. No UI.

pub mod capture;
pub mod diag;
pub mod geometry;
pub mod history;
pub mod hotkey;
pub mod naming;
pub mod paths;
pub mod settings;

pub use image::RgbaImage;
