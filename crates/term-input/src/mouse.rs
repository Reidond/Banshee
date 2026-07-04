//! Mouse encodings (SGR / urxvt / X10) per the vt-reported mouse mode.
//!
//! Filled in by M1 Task 6. Wheel-to-scrollback routing is decided by the
//! shell via `term_core::Terminal::mouse_reporting_active()`; this module
//! only encodes events destined for the application.
