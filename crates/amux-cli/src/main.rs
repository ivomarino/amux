//! The `amux` CLI (Rust). Ships as `amux-rs` until Phase 11 cutover renames
//! it over the bash script — two commands must not fight for one name while
//! the Python server is still authoritative.

use clap::Parser;

#[derive(Parser)]
#[command(name = "amux-rs", version, about = "amux command-line interface (Rust rebuild)")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("amux-rs — Phase 0 scaffold. Subcommands land in Phase 8.");
}
