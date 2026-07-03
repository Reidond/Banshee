//! Build glue: fetch/pin libghostty-vt artifact, bindgen.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("vendor-vt") => {
            println!("vendor-vt: not yet implemented");
            ExitCode::from(1)
        }
        _ => {
            eprintln!("usage: xtask <COMMAND>");
            eprintln!();
            eprintln!("Commands:");
            eprintln!("  vendor-vt    Fetch/pin the vendored libghostty-vt artifact");
            ExitCode::from(2)
        }
    }
}
