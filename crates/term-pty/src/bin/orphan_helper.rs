//! Orphan-check helper process (UC-03 E3).
//!
//! Spawns a shell inside a kill-on-close job object via [`term_pty::ConPty`],
//! prints a READY line (so the parent test knows the child tree is up), then
//! idles forever. The parent test records this process's PID, hard-kills it with
//! `TerminateProcess`, waits, and asserts no descendant shell/conhost survives.
//!
//! Because the job object handle lives inside this process, killing this process
//! closes that handle, which (kill-on-close) must reap the entire child tree.

#[cfg(windows)]
fn main() {
    use std::io::Write;
    use std::time::Duration;
    use term_pty::{ConPty, Shell};

    let shell = match std::env::args().nth(1).as_deref() {
        Some("cmd") => Shell::Cmd,
        _ => Shell::Pwsh,
    };

    // Keep the ConPty alive for the process lifetime; leak it so no Drop runs on
    // a normal exit path (the test always hard-kills us anyway).
    let pty = ConPty::spawn(shell, 80, 24, |_chunk| {}).expect("spawn pseudoconsole");
    std::mem::forget(pty);

    // Give the child a moment to fully spawn its own descendants (conhost etc.).
    std::thread::sleep(Duration::from_millis(600));

    println!("READY");
    let _ = std::io::stdout().flush();

    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("orphan_helper is Windows-only.");
}
