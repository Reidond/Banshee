//! ConPTY echo spike (UC-03 manual driver).
//!
//! Usage:
//!   cargo run -p term-pty --example echo_spike [pwsh|cmd]
//!
//! Spawns the chosen shell in a pseudoconsole, writes `echo hello`, reads until
//! the echoed text appears, performs a few resizes, sends `exit`, and prints the
//! exit code plus timing evidence.

#[cfg(windows)]
fn main() {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use term_pty::{ConPty, Shell};

    let shell = match std::env::args().nth(1).as_deref() {
        Some("cmd") => Shell::Cmd,
        _ => Shell::Pwsh,
    };
    println!("== echo_spike: {shell:?} ==");

    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = Arc::clone(&output);

    let started = Instant::now();
    let pty = ConPty::spawn(shell, 80, 24, move |chunk| {
        sink.lock().unwrap().extend_from_slice(chunk);
    })
    .expect("spawn pseudoconsole");
    println!("spawned in {:?}", started.elapsed());

    // Give the shell a moment to print its prompt, then type the command.
    std::thread::sleep(Duration::from_millis(300));
    pty.write(b"echo hello\r").expect("write command");

    // Read until we see the echoed text (post-command, distinct from the typed
    // line). We look for a second occurrence to avoid matching our own echo.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_echo = false;
    while Instant::now() < deadline {
        {
            let buf = output.lock().unwrap();
            let text = String::from_utf8_lossy(&buf);
            if text.matches("hello").count() >= 2 {
                saw_echo = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    println!("echo observed: {saw_echo}");
    {
        // Diagnostic: dump the captured output with control bytes escaped so we
        // can see exactly how the shell rendered the echoed line.
        let buf = output.lock().unwrap();
        let mut esc = String::new();
        for &b in buf.iter() {
            match b {
                0x1b => esc.push_str("<ESC>"),
                b'\r' => esc.push_str("<CR>"),
                b'\n' => esc.push_str("<LF>\n"),
                0x20..=0x7e => esc.push(b as char),
                _ => esc.push_str(&format!("<{b:02x}>")),
            }
        }
        eprintln!(
            "---RAW OUTPUT ({} bytes)---\n{esc}\n---END RAW---",
            buf.len()
        );
    }

    // Resize a few times; these coalesce behind the ~50 ms debounce.
    for (c, r) in [(100, 30), (120, 40), (90, 25)] {
        pty.resize(c, r);
        std::thread::sleep(Duration::from_millis(80));
    }
    println!("applied resizes: {:?}", pty.applied_resizes());

    // Ask the shell to exit and wait for the process-handle waiter to surface it.
    pty.write(b"exit\r").expect("write exit");
    match pty.wait_exit(Duration::from_secs(10)) {
        Some(status) => {
            println!(
                "exit code: {} (detected {:?} after process-handle signal)",
                status.code, status.detect_latency
            );
        }
        None => println!("!! exit not surfaced within timeout"),
    }

    println!("total session: {:?}", started.elapsed());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("echo_spike is Windows-only (ConPTY).");
}
