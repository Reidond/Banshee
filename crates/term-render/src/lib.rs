//! `term-render` — D3D11 renderer for Banshee (SPEC §6.2).
//!
//! M0 (Task 6) proved the swapchain/present/latency bet with an animated color
//! grid ([`GridRenderer`], still used by the app-shell `--self-test` path and the
//! WARP smoke test). **M1 Task 2** adds the real text pipeline:
//!
//!   * [`text`] — DirectWrite font stack + fallback + `rustybuzz` shaping, and the
//!     single `GridSnapshot` → [`text::FrameLayout`] boundary.
//!   * [`atlas`] — R8 grayscale glyph atlas (`IDWriteGlyphRunAnalysis` raster).
//!   * [`overlay`] — cursor / selection / decoration geometry.
//!   * [`grid`] — the [`CellRenderer`]: bg-run, glyph, decoration, and overlay
//!     passes, damage-driven ([`grid::Frame::is_dirty`]).
//!
//! Renderers are swapchain-agnostic: they draw into any RTV, so headless WARP
//! tests use an [`OffscreenTarget`] and the shell drives the same renderer against
//! a composition swapchain back buffer.

pub mod atlas;
pub mod context;
pub mod device;
pub mod grid;
pub mod grid_spike;
pub mod offscreen;
pub mod overlay;
pub mod swapchain;
pub mod text;

pub use context::RenderContext;
pub use device::{Device, DriverPreference};
pub use grid::{CellRenderer, Frame};
pub use grid_spike::GridRenderer;
pub use offscreen::OffscreenTarget;
pub use overlay::{RowRange, SolidRect};
pub use swapchain::CompositionSwapchain;
pub use text::{CellMetrics, FontStack, TextEngine};
