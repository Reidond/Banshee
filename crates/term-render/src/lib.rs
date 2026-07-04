//! `term-render` — D3D11 renderer for Banshee (SPEC §6.2).
//!
//! M0 scope (Task 6, UC-04 steps 1 & 3): a hand-rolled D3D11 device + flip-model
//! composition swapchain that draws an animated colored cell grid, plus
//! device-removed recovery. The DirectWrite/HarfBuzz text pipeline lands in M1;
//! this crate currently proves the swapchain/present/latency bet only.
//!
//! The renderer ([`GridRenderer`]) is swapchain-agnostic: it draws into any RTV,
//! so headless WARP tests use an [`OffscreenTarget`] and the Tier-A shell (Task 7)
//! drives the same renderer against a composition swapchain back buffer.

pub mod context;
pub mod device;
pub mod grid_spike;
pub mod offscreen;
pub mod swapchain;

pub use context::RenderContext;
pub use device::{Device, DriverPreference};
pub use grid_spike::GridRenderer;
pub use offscreen::OffscreenTarget;
pub use swapchain::CompositionSwapchain;
