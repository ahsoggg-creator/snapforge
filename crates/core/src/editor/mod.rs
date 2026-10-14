//! The image editor model: vector objects over a raster base, undo/redo and rendering.

pub mod model;
pub mod render;

pub use model::{Color, DEFAULT_COLOR, DEFAULT_WIDTH, Document, Editor, Object, Pt, Shape};
