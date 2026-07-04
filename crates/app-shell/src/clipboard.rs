//! Clipboard + input-geometry seams for M1 Task 12.
//!
//! Two concerns live here behind testable seams:
//!
//! 1. **Pure math / keybind detection** (`pixel_to_cell`, [`CopyPasteKey`]) —
//!    no Win32, unit-tested directly.
//! 2. **Win32 clipboard round-trip** (`set_text` / `get_text`, CF_UNICODETEXT) —
//!    thin `unsafe` wrappers with the `OpenClipboard`/`EmptyClipboard`/
//!    `GlobalAlloc`/`GlobalLock` discipline. Behind `#[cfg(windows)]`; the GUI
//!    path is exercised by an `#[ignore]` live test.

use term_render::CellMetrics;

/// Map a device-pixel coordinate (relative to the grid origin) to a viewport
/// cell `(col, row)`. Pure and clamp-free: callers clamp to the grid bounds
/// (the vt grid-ref resolution already rejects out-of-range cells, and
/// selection clamps internally). Negative-safe: values below the origin map to
/// column/row 0.
#[must_use]
pub fn pixel_to_cell(px: i32, py: i32, m: &CellMetrics) -> (u16, u16) {
    let cw = m.cell_w.max(1.0);
    let ch = m.cell_h.max(1.0);
    let col = ((px.max(0) as f32) / cw).floor();
    let row = ((py.max(0) as f32) / ch).floor();
    (col as u16, row as u16)
}

/// A recognized copy/paste key chord, derived from a keydown vk + modifier
/// state. Pure so the detection logic is unit-tested without Win32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyPasteKey {
    /// Ctrl+Shift+C — copy the selection.
    Copy,
    /// Ctrl+Shift+V — paste from the clipboard.
    Paste,
}

/// Windows virtual-key code for `C`.
const VK_C: u32 = 0x43;
/// Windows virtual-key code for `V`.
const VK_V: u32 = 0x56;

/// Detect a copy/paste chord from a keydown. `ctrl`/`shift` are the live
/// modifier states at keydown time. The terminal convention is Ctrl+Shift+C/V
/// (plain Ctrl+C is SIGINT, plain Ctrl+V is a literal to the shell), so BOTH
/// Ctrl and Shift are required and Alt must be absent.
#[must_use]
pub fn detect_copy_paste(vk: u32, ctrl: bool, shift: bool, alt: bool) -> Option<CopyPasteKey> {
    if !ctrl || !shift || alt {
        return None;
    }
    match vk {
        VK_C => Some(CopyPasteKey::Copy),
        VK_V => Some(CopyPasteKey::Paste),
        _ => None,
    }
}

#[cfg(windows)]
pub use win32::{get_text, set_text};

#[cfg(windows)]
mod win32 {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_UNICODETEXT;

    /// RAII guard: `OpenClipboard` on construction, `CloseClipboard` on drop, so
    /// every early return still closes the clipboard.
    struct ClipboardGuard;

    impl ClipboardGuard {
        /// Open the clipboard with no owner window (a NULL owner is acceptable
        /// and associates the clipboard with the current task).
        fn open() -> Option<ClipboardGuard> {
            // SAFETY: standard Win32 call; a `None` owner is a NULL HWND.
            let ok = unsafe { OpenClipboard(None) }.is_ok();
            ok.then_some(ClipboardGuard)
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: paired with a successful OpenClipboard.
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// Place `text` on the clipboard as CF_UNICODETEXT (UTF-16, NUL-terminated).
    /// Returns `false` if the clipboard could not be opened or memory could not
    /// be allocated.
    #[must_use]
    pub fn set_text(text: &str) -> bool {
        let Some(_guard) = ClipboardGuard::open() else {
            return false;
        };
        // SAFETY: we own the open clipboard; EmptyClipboard clears prior content
        // and assigns ownership to us (required before SetClipboardData).
        if unsafe { EmptyClipboard() }.is_err() {
            return false;
        }

        // UTF-16 + trailing NUL.
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        utf16.push(0);
        let bytes = utf16.len() * std::mem::size_of::<u16>();

        // GMEM_MOVEABLE is required for clipboard memory.
        // SAFETY: allocate a moveable global block sized for the UTF-16 payload.
        let hglobal: HGLOBAL = match unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) } {
            Ok(h) => h,
            Err(_) => return false,
        };
        // SAFETY: lock the block to get a writable pointer; must GlobalUnlock.
        let dst = unsafe { GlobalLock(hglobal) };
        if dst.is_null() {
            // The block is not yet owned by the clipboard; free it.
            // SAFETY: hglobal is a valid unlocked handle we just allocated.
            unsafe {
                let _ = GlobalFree(Some(hglobal));
            }
            return false;
        }
        // SAFETY: `dst` points to at least `bytes` writable bytes; source is a
        // matching-length UTF-16 buffer.
        unsafe {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst.cast::<u16>(), utf16.len());
            let _ = GlobalUnlock(hglobal);
        }

        // Hand ownership of the block to the clipboard. On success the system
        // owns the memory (we must NOT free it).
        // SAFETY: valid format + handle; clipboard is open and owned by us.
        let ok = unsafe {
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(hglobal.0))).is_ok()
        };
        if !ok {
            // SetClipboardData failed → we still own the block; free it.
            // SAFETY: hglobal is valid and not yet owned by the clipboard.
            unsafe {
                let _ = GlobalFree(Some(hglobal));
            }
        }
        ok
    }

    /// Read CF_UNICODETEXT from the clipboard as a `String`. Returns `None` when
    /// the clipboard has no unicode text or cannot be opened.
    #[must_use]
    pub fn get_text() -> Option<String> {
        let _guard = ClipboardGuard::open()?;
        // SAFETY: clipboard is open; GetClipboardData returns a borrowed handle
        // owned by the clipboard (we must not free it).
        let handle: HANDLE = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
        if handle.is_invalid() {
            return None;
        }
        let hglobal = HGLOBAL(handle.0);
        // SAFETY: lock the clipboard's global block for reading.
        let src = unsafe { GlobalLock(hglobal) };
        if src.is_null() {
            return None;
        }
        // Read the NUL-terminated UTF-16 string.
        let mut units: Vec<u16> = Vec::new();
        // SAFETY: `src` points to a NUL-terminated UTF-16 string owned by the
        // clipboard; we read until the terminator.
        unsafe {
            let mut p = src.cast::<u16>();
            loop {
                let u = *p;
                if u == 0 {
                    break;
                }
                units.push(u);
                p = p.add(1);
            }
            let _ = GlobalUnlock(hglobal);
        }
        Some(String::from_utf16_lossy(&units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_render::CellMetrics;

    fn metrics(cw: f32, ch: f32) -> CellMetrics {
        CellMetrics {
            cell_w: cw,
            cell_h: ch,
            baseline: 0.0,
            px_size: ch,
        }
    }

    #[test]
    fn pixel_to_cell_basic() {
        let m = metrics(8.0, 16.0);
        assert_eq!(pixel_to_cell(0, 0, &m), (0, 0));
        assert_eq!(pixel_to_cell(7, 15, &m), (0, 0));
        assert_eq!(pixel_to_cell(8, 16, &m), (1, 1));
        assert_eq!(pixel_to_cell(20, 40, &m), (2, 2));
    }

    #[test]
    fn pixel_to_cell_negative_clamps_to_origin() {
        let m = metrics(8.0, 16.0);
        assert_eq!(pixel_to_cell(-5, -100, &m), (0, 0));
    }

    #[test]
    fn pixel_to_cell_survives_zero_metrics() {
        // Degenerate metrics must not divide-by-zero / panic.
        let m = metrics(0.0, 0.0);
        let _ = pixel_to_cell(10, 10, &m);
    }

    #[test]
    fn copy_chord_requires_ctrl_shift() {
        assert_eq!(detect_copy_paste(0x43, true, true, false), Some(CopyPasteKey::Copy));
        assert_eq!(detect_copy_paste(0x56, true, true, false), Some(CopyPasteKey::Paste));
        // Missing shift → not our chord (plain Ctrl+C is SIGINT).
        assert_eq!(detect_copy_paste(0x43, true, false, false), None);
        // Alt held → not our chord.
        assert_eq!(detect_copy_paste(0x43, true, true, true), None);
        // Other keys → none.
        assert_eq!(detect_copy_paste(0x41, true, true, false), None);
    }

    /// Live Win32 clipboard round-trip. `#[ignore]` because it mutates the real
    /// OS clipboard and needs a windowstation (not headless CI). Run with
    /// `cargo test -p app-shell -- --ignored clipboard_roundtrip`.
    #[cfg(windows)]
    #[test]
    #[ignore = "mutates the real OS clipboard; run manually on an interactive session"]
    fn clipboard_roundtrip_live() {
        let sample = "banshee-selection ✓ 日本語";
        assert!(super::set_text(sample), "set_text should succeed");
        let got = super::get_text().expect("get_text should return the text just set");
        assert_eq!(got, sample, "clipboard round-trip must be lossless");
    }
}
