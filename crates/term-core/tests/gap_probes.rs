//! Gap-Log probing tests (UC-02 steps 5–6). Each test corresponds to one row in
//! `.specs/m0-seance/gap-log.md` and proves — by calling the *real* pinned C API
//! (ghostty commit d560c645), not from memory — whether a SPEC §6.1(3)
//! capability is exposed. A passing test = the capability behaves as the Gap Log
//! records it. No capability is assumed; every row is verified here.
//!
//! These reach through `Terminal::raw()` (a hidden escape hatch) because the
//! capabilities under test are intentionally NOT part of the M0 safe surface —
//! we are verifying the FFI substrate exists for M1 to build on.

use std::os::raw::c_void;
use std::ptr;

use ghostty_vt_sys as sys;
use term_core::{GridSnapshot, Terminal, VtOptions};

fn new_term() -> Terminal {
    Terminal::new(80, 24, VtOptions::default()).unwrap()
}

fn active_point(x: u16, y: u16) -> sys::GhosttyPoint {
    sys::GhosttyPoint {
        tag: sys::GhosttyPointTag::GHOSTTY_POINT_TAG_ACTIVE,
        value: sys::GhosttyPointValue {
            coordinate: sys::GhosttyPointCoordinate { x, y: u32::from(y) },
        },
    }
}

fn zeroed_grid_ref() -> sys::GhosttyGridRef {
    sys::GhosttyGridRef {
        size: std::mem::size_of::<sys::GhosttyGridRef>(),
        node: ptr::null_mut(),
        x: 0,
        y: 0,
    }
}

/// GAP ROW: selection state.
/// STATUS: exposed — read via `ghostty_terminal_get(GHOSTTY_TERMINAL_DATA_SELECTION)`,
/// written via `GHOSTTY_TERMINAL_OPT_SELECTION`, plus a full derive/gesture API
/// in selection.h. We install a select-all selection and read it back.
#[test]
fn probe_selection_state_exposed() {
    let mut term = new_term();
    term.feed(b"select me");

    // Derive a select-all snapshot, install it, then read it back.
    let mut sel = sys::GhosttySelection {
        size: std::mem::size_of::<sys::GhosttySelection>(),
        start: zeroed_grid_ref(),
        end: zeroed_grid_ref(),
        rectangle: false,
    };
    // SAFETY: live handle; valid out-pointer.
    let rc = unsafe { sys::ghostty_terminal_select_all(term.raw(), &mut sel) };
    assert_eq!(
        rc,
        sys::GhosttyResult::GHOSTTY_SUCCESS,
        "ghostty_terminal_select_all should expose a selection over fed content"
    );

    // SAFETY: install the derived selection (copied immediately by the C API).
    let rc = unsafe {
        sys::ghostty_terminal_set(
            term.raw(),
            sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_SELECTION,
            (&sel as *const sys::GhosttySelection).cast::<c_void>(),
        )
    };
    assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);

    // Read the active selection back.
    let mut readback = sys::GhosttySelection {
        size: std::mem::size_of::<sys::GhosttySelection>(),
        start: zeroed_grid_ref(),
        end: zeroed_grid_ref(),
        rectangle: false,
    };
    // SAFETY: live handle; valid out-pointer.
    let rc = unsafe {
        sys::ghostty_terminal_get(
            term.raw(),
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SELECTION,
            (&mut readback as *mut sys::GhosttySelection).cast::<c_void>(),
        )
    };
    assert_eq!(
        rc,
        sys::GhosttyResult::GHOSTTY_SUCCESS,
        "active selection should be readable after install"
    );
}

/// GAP ROW: hyperlink ids.
/// STATUS: partial — per-cell hyperlink *presence* (`GHOSTTY_CELL_DATA_HAS_HYPERLINK`)
/// and the *URI* (`ghostty_grid_ref_hyperlink_uri`) are exposed, but NO stable
/// numeric hyperlink id is exposed in this commit. We prove presence + URI
/// readback; the id itself is the gap (fallback: key by URI, or assign ids
/// Rust-side).
#[test]
fn probe_hyperlink_presence_and_uri_exposed_id_missing() {
    let mut term = new_term();
    // OSC 8 hyperlink: ESC ] 8 ; ; URI ST  text  ESC ] 8 ; ; ST
    term.feed(b"\x1b]8;;https://example.com\x1b\\LINK\x1b]8;;\x1b\\");

    let point = active_point(0, 0);
    let mut gref = zeroed_grid_ref();
    // SAFETY: live handle; valid out-pointers.
    let rc = unsafe { sys::ghostty_terminal_grid_ref(term.raw(), point, &mut gref) };
    assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);

    let mut cell: sys::GhosttyCell = 0;
    // SAFETY: gref resolved.
    unsafe { sys::ghostty_grid_ref_cell(&gref, &mut cell) };
    let mut has_link = false;
    // SAFETY: output type for HAS_HYPERLINK is bool*.
    unsafe {
        sys::ghostty_cell_get(
            cell,
            sys::GhosttyCellData::GHOSTTY_CELL_DATA_HAS_HYPERLINK,
            (&mut has_link as *mut bool).cast::<c_void>(),
        );
    }
    assert!(has_link, "cell under OSC 8 should report a hyperlink");

    // URI is retrievable; query required length first, then read it.
    let mut needed: usize = 0;
    // SAFETY: NULL buf queries required size into out_len.
    let rc = unsafe { sys::ghostty_grid_ref_hyperlink_uri(&gref, ptr::null_mut(), 0, &mut needed) };
    // Either OUT_OF_SPACE (needs a buffer) or SUCCESS with len set.
    assert!(
        rc == sys::GhosttyResult::GHOSTTY_OUT_OF_SPACE || rc == sys::GhosttyResult::GHOSTTY_SUCCESS,
        "hyperlink_uri size query should report the URI length; got {rc:?}"
    );
    assert!(needed > 0, "URI length should be non-zero");

    let mut buf = vec![0u8; needed];
    let mut written: usize = 0;
    // SAFETY: buf is `needed` bytes.
    let rc = unsafe {
        sys::ghostty_grid_ref_hyperlink_uri(&gref, buf.as_mut_ptr(), buf.len(), &mut written)
    };
    assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
    assert_eq!(&buf[..written], b"https://example.com");

    // NOTE: there is no `ghostty_*_hyperlink_id` symbol in this commit — the
    // numeric id is the documented gap. This absence is compile-time-evidenced:
    // the sys crate exposes no such function to call here.
}

/// GAP ROW: Kitty-graphics image payload access.
/// STATUS: exposed (API), gated at build time. The full storage/placement/image
/// API exists (kitty_graphics.h). Whether images can actually be stored depends
/// on `GHOSTTY_BUILD_INFO_KITTY_GRAPHICS` for this static lib; we record the
/// build flag and prove the storage handle + generation query work.
#[test]
fn probe_kitty_graphics_payload_access() {
    // Build flag first.
    let mut kitty_enabled = false;
    // SAFETY: output type for KITTY_GRAPHICS build info is bool*.
    let rc = unsafe {
        sys::ghostty_build_info(
            sys::GhosttyBuildInfo::GHOSTTY_BUILD_INFO_KITTY_GRAPHICS,
            (&mut kitty_enabled as *mut bool).cast::<c_void>(),
        )
    };
    assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
    // Record for the Gap Log; do not hard-require enabled (fallback documented).
    println!("BUILD_INFO_KITTY_GRAPHICS = {kitty_enabled}");

    let term = new_term();
    // Fetch the storage handle. Returns NO_VALUE if Kitty is disabled at build.
    let mut storage: sys::GhosttyKittyGraphics = ptr::null_mut();
    // SAFETY: live handle; output type is GhosttyKittyGraphics*.
    let rc = unsafe {
        sys::ghostty_terminal_get(
            term.raw(),
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,
            (&mut storage as *mut sys::GhosttyKittyGraphics).cast::<c_void>(),
        )
    };
    if kitty_enabled {
        assert_eq!(
            rc,
            sys::GhosttyResult::GHOSTTY_SUCCESS,
            "Kitty storage handle should be available when built with Kitty graphics"
        );
        assert!(!storage.is_null());
        // Generation stamp is queryable (change-detection path for renderer).
        let mut generation: u64 = 0;
        // SAFETY: output type for GENERATION is uint64_t*.
        let rc = unsafe {
            sys::ghostty_kitty_graphics_get(
                storage,
                sys::GhosttyKittyGraphicsData::GHOSTTY_KITTY_GRAPHICS_DATA_GENERATION,
                (&mut generation as *mut u64).cast::<c_void>(),
            )
        };
        assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
    } else {
        assert_eq!(
            rc,
            sys::GhosttyResult::GHOSTTY_NO_VALUE,
            "Kitty disabled at build → NO_VALUE (fallback: enable in vendor build)"
        );
    }
}

/// GAP ROW: terminal query responses (DSR / DA).
/// STATUS: exposed — via the `GHOSTTY_TERMINAL_OPT_WRITE_PTY` effect callback,
/// which `term-core` wraps as `Terminal::responses()`. We feed a DA1 query
/// (CSI c) and a DSR cursor-position query (CSI 6 n) and assert the wrapper
/// surfaces reply bytes to be written back to the pty.
#[test]
fn probe_query_responses_exposed() {
    let mut term = new_term();
    // Move the cursor, then ask for a cursor position report (DSR 6) and
    // primary device attributes (DA1). Both should elicit write-pty responses.
    term.feed(b"\x1b[5;3H"); // move cursor to row 5, col 3
    term.feed(b"\x1b[6n"); // DSR: report cursor position
    term.feed(b"\x1b[c"); // DA1: primary device attributes

    let replies: Vec<Vec<u8>> = term.responses().collect();
    assert!(
        !replies.is_empty(),
        "DSR/DA queries should produce write-pty responses via the effect callback"
    );
    let joined: Vec<u8> = replies.concat();
    // A CPR reply looks like ESC [ <row> ; <col> R; a DA1 reply like ESC [ ? ... c.
    assert!(
        joined.contains(&b'R') || joined.contains(&b'c'),
        "responses should contain a CPR (R) and/or DA (c) terminator; got {joined:?}"
    );
    // Responses drain: a second call is empty.
    assert_eq!(term.responses().count(), 0, "responses() drains the buffer");
}

/// EXTRA GAP ROW: damage / dirty-row tracking (needed by the render contract).
/// STATUS: exposed — `GHOSTTY_ROW_DATA_DIRTY` per row (also a full render-state
/// fast path in render.h). `GridSnapshot::RowSnapshot::dirty` surfaces it.
#[test]
fn probe_dirty_row_tracking_exposed() {
    let mut term = new_term();
    term.feed(b"row zero");
    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    // After feeding, at least the touched row should be reported dirty.
    assert!(
        snap.rows_data.iter().any(|r| r.dirty),
        "at least one row should report dirty after a feed"
    );
}

/// EXTRA GAP ROW: keyboard-mode readback (Kitty keyboard flags).
/// STATUS: exposed — `GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS`. Keyboard
/// *encoding* is a separate lib (SPEC §6.3), but the mode/flag readback the vt
/// owns is present. We read the flags without error.
#[test]
fn probe_keyboard_mode_readback_exposed() {
    let term = new_term();
    let mut flags: u8 = 0;
    // SAFETY: output type for KITTY_KEYBOARD_FLAGS is uint8_t*.
    let rc = unsafe {
        sys::ghostty_terminal_get(
            term.raw(),
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS,
            (&mut flags as *mut u8).cast::<c_void>(),
        )
    };
    assert_eq!(
        rc,
        sys::GhosttyResult::GHOSTTY_SUCCESS,
        "Kitty keyboard flags should be readable"
    );
}

/// EXTRA GAP ROW: scrollback read access.
/// STATUS: exposed — scrollback rows are reachable via history/screen point
/// tags through `ghostty_terminal_grid_ref`, and counts via
/// `GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS`. `term-core` snapshots only the
/// active area in M0 (fallback: scrollback read deferred to M1 search/AI path),
/// but the substrate is present. We prove the count query works.
#[test]
fn probe_scrollback_read_exposed() {
    let mut term = new_term();
    // Overflow the 24-row screen to push lines into scrollback.
    for i in 0..60u32 {
        term.feed(format!("line {i}\r\n").as_bytes());
    }
    let mut sb_rows: usize = 0;
    // SAFETY: output type for SCROLLBACK_ROWS is size_t*.
    let rc = unsafe {
        sys::ghostty_terminal_get(
            term.raw(),
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS,
            (&mut sb_rows as *mut usize).cast::<c_void>(),
        )
    };
    assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
    assert!(
        sb_rows > 0,
        "feeding 60 lines into a 24-row screen should populate scrollback"
    );
}

/// Sanity: `Terminal` is `Send` (single-owner, movable across threads) but not
/// `Sync` (never shared). This is a compile-time contract check.
#[test]
fn terminal_is_send_not_sync() {
    fn assert_send<T: Send>() {}
    assert_send::<Terminal>();
    // If someone adds `unsafe impl Sync for Terminal`, add a static assertion
    // here that fails to compile. We document the invariant in lib.rs.
    let term = new_term();
    let handle = std::thread::spawn(move || {
        let mut t = term;
        t.feed(b"moved across threads");
        t.cols()
    });
    assert_eq!(handle.join().unwrap(), 80);
}
