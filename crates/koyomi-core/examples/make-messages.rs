//! Write `docs/troubleshooting.md` (Japanese) from the message catalogue.
//!
//!     cargo run -p koyomi-core --example make-messages [-- --check]
//!
//! `--check` writes nothing and exits 1 when the file is out of date.

use std::path::Path;

fn main() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/troubleshooting.md");
    let want = koyomi_core::msg::troubleshooting_markdown("ja");
    if std::env::args().any(|a| a == "--check") {
        let have = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        if have != want {
            eprintln!("{} is out of date: cargo run -p koyomi-core --example make-messages", path.display());
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = std::fs::write(&path, want) {
        eprintln!("{}: {e}", path.display());
        std::process::exit(1);
    }
}
