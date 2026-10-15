//! The image editor model: vector objects over a raster base, undo/redo and rendering.

pub mod chart;
pub mod model;
pub mod render;

pub use chart::{ChartKind, ChartSpec, Series};
pub use model::{Color, DEFAULT_COLOR, DEFAULT_WIDTH, Document, Editor, Object, Pt, Shape};
pub use render::Beautify;
