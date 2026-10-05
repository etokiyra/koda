//! Print the README keymap table from the command registry.
//!
//! ```sh
//! cargo run --example keymap
//! ```
//!
//! The output belongs between the `<!-- keymap:start -->` and
//! `<!-- keymap:end -->` markers in `README.md`. `tests/keymap.rs` fails when
//! the two drift, so regenerate with this example after adding a command.

fn main() {
    print!("{}", koda::commands::keymap_markdown());
}
