//! Kitty keyboard protocol encoder (progressive enhancement flags).
//!
//! Filled in by M1 Task 5. The shell queries the vt's reported flags via
//! `term_core::Terminal::kitty_flags()` and passes them into [`crate::Mode`];
//! this module encodes key events per exactly those flags.
