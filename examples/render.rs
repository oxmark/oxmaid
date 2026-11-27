//! Renders a Mermaid file (or stdin) to SVG on stdout.
//!
//! ```sh
//! cargo run --example render -- examples/diagrams/basic.mmd > basic.svg
//! ```
use std::io::Read;

fn main() {
    let src = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(2);
        }),
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .expect("cannot read stdin");
            s
        }
    };
    match oxmaid::render(&src) {
        Ok(svg) => println!("{svg}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
