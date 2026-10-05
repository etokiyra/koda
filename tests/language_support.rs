//! The authoritative language-support matrix must track the provider registry.
//!
//! Two invariants: every known language has a provider with a file-level
//! detection signal, and every known language has a row in
//! `docs/LANGUAGE-SUPPORT.md`. Regenerate the row when adding a language.

use koda::language::id::LanguageId;
use koda::language::provider::ProviderRegistry;

const START: &str = "<!-- languages:start -->";
const END: &str = "<!-- languages:end -->";

#[test]
fn every_language_has_a_provider_with_detection() {
    let registry = ProviderRegistry::builtin();
    for &id in &LanguageId::ALL {
        let provider = registry
            .get_opt(id)
            .unwrap_or_else(|| panic!("no provider registered for {}", id.name()));
        let descriptor = provider.descriptor();
        assert_eq!(
            descriptor.id,
            id,
            "the descriptor for {} claims a different language",
            id.name()
        );
        assert!(
            !descriptor.extensions.is_empty()
                || !descriptor.file_names.is_empty()
                || !descriptor.shebangs.is_empty(),
            "{} has no file-level detection signal",
            id.name()
        );
    }
}

#[test]
fn every_language_appears_in_the_support_matrix() {
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/LANGUAGE-SUPPORT.md"
    ))
    .expect("read docs/LANGUAGE-SUPPORT.md");
    let start = doc.find(START).expect("matrix start marker") + START.len();
    let end = doc[start..].find(END).expect("matrix end marker") + start;
    let block = &doc[start..end];

    for &id in &LanguageId::ALL {
        let needle = format!("| **{}** |", id.name());
        assert!(
            block.contains(&needle),
            "{} ({}) has no row in docs/LANGUAGE-SUPPORT.md",
            id.name(),
            id.slug()
        );
    }
}
