//! The README keymap table must match the command registry.
//!
//! Regenerate the block with `cargo run --example keymap`. This test fails when
//! the registry gains or changes a shortcut and the README is not updated.

const START: &str = "<!-- keymap:start -->";
const END: &str = "<!-- keymap:end -->";

#[test]
fn readme_keymap_matches_the_command_registry() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
        .expect("read README.md");
    let start = readme
        .find(START)
        .expect("README has a keymap start marker")
        + START.len();
    let end = readme[start..]
        .find(END)
        .expect("README has a keymap end marker")
        + start;
    let actual = readme[start..end].trim();
    let expected = koda::commands::keymap_markdown();
    assert_eq!(
        actual,
        expected.trim(),
        "README keymap is stale — regenerate it with `cargo run --example keymap`"
    );
}
