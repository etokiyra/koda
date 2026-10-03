//! Every built-in provider must survive adversarial, non-ASCII input.
//!
//! The HTML tag scanner once mixed character indices with byte slicing and
//! panicked on ordinary accented text. This sweeps every registered provider
//! over documents chosen to catch that class of bug — multibyte characters,
//! combining marks, emoji, lone delimiters and pathological lengths — through
//! each provider entry point.

use koda::language::LanguageService;
use koda::language::id::LanguageId;
use koda::language::provider::HighlightState;

fn cases() -> Vec<String> {
    let fixed = [
        "",
        "é",
        "→",
        "<é></é>",
        "<h1>Café ☕</h1>",
        "<p>こんにちは</p>",
        r#"{"café": "naïve", "emoji": "🎉"}"#,
        r#"let s = "日本語"; // コメント"#,
        "# 見出し\n\nemoji 🎉 and ✨",
        "select * from café where id = 1;",
        "@media (min-width: 100px) { .café { color: #ff0; } }",
        r#"fn main() { println!("héllo {}", name); }"#,
        "a\u{0301}\u{0301}\u{0301}b",
        "👩‍💻👨‍👩‍👧‍👦",
        "<<<<<<<<<<>>>>>>>>>>",
        "(((([[[[{{{{}}}}]]]]))))",
        "`hello ${name}`",
        r#""\"\\'\n\t""#,
        "'''\"\"\"` ` `",
        "<div id=\"\u{1F600}\" => </div>",
        "<!-- café 🎉\nまだコメント\n-->",
        "text with \ttabs\tand\r\nCRLF\r\n",
    ];
    let mut cases: Vec<String> = fixed.iter().map(|case| (*case).to_string()).collect();
    cases.push("x".repeat(4096));
    cases.push("日本語".repeat(512));
    cases.push("[".repeat(1000));
    cases
}

#[test]
fn every_provider_survives_adversarial_text() {
    let service = LanguageService::builtin();
    for &language in &LanguageId::ALL {
        let provider = service.provider(language);
        for text in cases() {
            // Highlight line by line, carrying the block state across lines.
            let mut state = HighlightState::default();
            for line in text.lines() {
                let (spans, next) = provider.highlight(line, state);
                state = next;
                for span in &spans {
                    assert!(
                        span.range.start <= span.range.end,
                        "{language:?}: inverted span in {text:?}"
                    );
                }
            }

            let _ = provider.diagnostics(&text);
            let _ = provider.symbols(&text);
            let _ = provider.completions(&text, 0, 0);
            let _ = provider.hover(&text, 0, 0);

            // Positions beyond the document must be handled, not assumed.
            for (row, col) in [(0, 0), (0, 3), (1, 1), (5, 9), (usize::MAX, usize::MAX)] {
                let _ = provider.definition(&text, row, col);
                let _ = provider.references(&text, row, col);
            }
        }
    }
}
