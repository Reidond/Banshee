//! Link + round-trip smoke test for the raw FFI (UC-01 step 6).
//!
//! Constructs a terminal through the raw C ABI, feeds `"hi"`, resolves the
//! first two cells, and asserts they hold `'h'` and `'i'`. This proves the
//! vendored static lib links and the core feed/snapshot path works across the
//! FFI boundary. Real coverage lives in `term-core`; this stays minimal.

use std::ptr;

use ghostty_vt_sys::*;

#[test]
fn construct_feed_snapshot() {
    unsafe {
        // Construct an 80x24 terminal with the default allocator (NULL).
        let opts = GhosttyTerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 1000,
        };
        let mut term: GhosttyTerminal = ptr::null_mut();
        let rc = ghostty_terminal_new(ptr::null(), &mut term, opts);
        assert_eq!(rc, GhosttyResult::GHOSTTY_SUCCESS, "terminal_new failed");
        assert!(!term.is_null(), "terminal handle is null");

        // Feed "hi".
        let bytes = b"hi";
        ghostty_terminal_vt_write(term, bytes.as_ptr(), bytes.len());

        // Read back cells (0,0) and (1,0) in the active coordinate space.
        for (x, expected) in [(0u16, 'h'), (1u16, 'i')] {
            let point = GhosttyPoint {
                tag: GhosttyPointTag::GHOSTTY_POINT_TAG_ACTIVE,
                value: GhosttyPointValue {
                    coordinate: GhosttyPointCoordinate { x, y: 0 },
                },
            };
            let mut gref = GhosttyGridRef {
                size: std::mem::size_of::<GhosttyGridRef>(),
                node: ptr::null_mut(),
                x: 0,
                y: 0,
            };
            let rc = ghostty_terminal_grid_ref(term, point, &mut gref);
            assert_eq!(rc, GhosttyResult::GHOSTTY_SUCCESS, "grid_ref @ {x} failed");

            let mut cell: GhosttyCell = 0;
            let rc = ghostty_grid_ref_cell(&gref, &mut cell);
            assert_eq!(rc, GhosttyResult::GHOSTTY_SUCCESS, "grid_ref_cell @ {x}");

            let mut cp: u32 = 0;
            let rc = ghostty_cell_get(
                cell,
                GhosttyCellData::GHOSTTY_CELL_DATA_CODEPOINT,
                (&mut cp as *mut u32).cast(),
            );
            assert_eq!(
                rc,
                GhosttyResult::GHOSTTY_SUCCESS,
                "cell_get codepoint @ {x}"
            );
            assert_eq!(
                char::from_u32(cp),
                Some(expected),
                "cell @ {x} expected {expected:?}, got U+{cp:04X}"
            );
        }

        ghostty_terminal_free(term);
    }
}
