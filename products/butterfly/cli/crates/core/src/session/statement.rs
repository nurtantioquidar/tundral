//! Statement signals: a deterministic read of how much a task statement gives
//! a model to bind to. Motivation, private suite 2026-09-12..16: the same
//! harness and model scored 0/10 on raw commit titles and 10/10 on rewritten
//! statements, and every miss left on both suites traced to a statement that
//! did not name what the hidden tests bound to (a helper's name, an exact
//! message, a path). Calibrated on 41 tasks: rewritten statements had at
//! least 4 distinct identifiers and 67 words; raw titles a median of 1 and 7.
//! Nothing here blocks a run; it labels the risk before money is spent.

use serde::Serialize;

/// Distinct identifier-like tokens below which a statement is called thin.
const MIN_IDENTIFIERS: usize = 3;
/// Words below which a statement is called thin.
const MIN_WORDS: usize = 40;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StatementSignals {
    pub words: usize,
    /// Distinct code-shaped tokens: snake_case, camelCase, dotted names,
    /// ALL_CAPS constants, backticked spans.
    pub identifiers: usize,
    /// Distinct tokens ending in a source or config extension.
    pub paths: usize,
    /// Double- or single-quoted spans of three or more characters: messages,
    /// formats, literal values a test could compare against.
    pub quoted: usize,
    /// Too little to bind to; expect interface mismatches.
    pub thin: bool,
}

impl StatementSignals {
    pub fn of(text: &str) -> Self {
        let words = text.split_whitespace().count();
        let mut identifiers = std::collections::BTreeSet::new();
        let mut paths = std::collections::BTreeSet::new();
        for raw in text.split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | ':' | '"' | '\''
                )
        }) {
            let token = raw
                .trim_matches(|c: char| matches!(c, '.' | '`' | '*' | '<' | '>' | '#' | '?' | '!'));
            if token.len() < 2 {
                continue;
            }
            if has_source_extension(token) {
                paths.insert(token.to_string());
                identifiers.insert(token.to_string());
            } else if looks_like_identifier(token) {
                identifiers.insert(token.to_string());
            }
        }
        for span in backticked(text) {
            if span.trim().len() >= 2 {
                identifiers.insert(span.trim().to_string());
            }
        }
        let quoted = quoted_spans(text);
        let thin = identifiers.len() < MIN_IDENTIFIERS || words < MIN_WORDS;
        Self {
            words,
            identifiers: identifiers.len(),
            paths: paths.len(),
            quoted,
            thin,
        }
    }

    /// One line for the run summary; None when the statement is not thin.
    pub fn summary_line(&self) -> Option<String> {
        self.thin.then(|| {
            format!(
                "statement: thin ({} words, {} identifier(s), {} path(s), {} quoted span(s)) — the \
                 tests will bind to names this statement does not give; expect interface \
                 mismatches. Name the functions, messages, paths and cases the result must match.",
                self.words, self.identifiers, self.paths, self.quoted
            )
        })
    }
}

fn has_source_extension(token: &str) -> bool {
    const EXT: &[&str] = &[
        ".py", ".java", ".ts", ".tsx", ".js", ".rs", ".go", ".sql", ".yaml", ".yml", ".toml",
        ".json", ".xml", ".md", ".kt", ".scala", ".sh", ".cfg", ".ini", ".proto",
    ];
    let lower = token.to_ascii_lowercase();
    EXT.iter().any(|ext| lower.ends_with(ext)) && token.len() > 4
}

/// snake_case, camelCase/PascalCase with an internal case change, dotted
/// names, or ALL_CAPS of three or more letters. Plain words never qualify.
fn looks_like_identifier(token: &str) -> bool {
    let bytes = token.as_bytes();
    if !bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'/'))
    {
        return false;
    }
    let has_alpha = bytes.iter().any(|b| b.is_ascii_alphabetic());
    if !has_alpha {
        return false;
    }
    let inner_underscore = token.contains('_') && !token.starts_with('_') && !token.ends_with('_');
    let dotted = token.contains('.')
        && token.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    let slashed = token.contains('/') && token.len() > 3;
    let case_change = bytes
        .windows(2)
        .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
    let all_caps = token.len() >= 3
        && bytes
            .iter()
            .all(|b| b.is_ascii_uppercase() || *b == b'_' || b.is_ascii_digit())
        && bytes.iter().filter(|b| b.is_ascii_uppercase()).count() >= 3;
    inner_underscore || dotted || slashed || case_change || all_caps
}

fn backticked(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else { break };
        out.push(&after[..end]);
        rest = &after[end + 1..];
    }
    out
}

fn quoted_spans(text: &str) -> usize {
    let mut count = 0;
    for quote in ['"', '\''] {
        let mut rest = text;
        while let Some(start) = rest.find(quote) {
            let after = &rest[start + 1..];
            let Some(end) = after.find(quote) else { break };
            let span = &after[..end];
            // An apostrophe inside a word is not a quote.
            if span.len() >= 3
                && !span.contains('\n')
                && (quote == '"' || !span.contains(' ') || span.len() < 60)
            {
                count += 1;
            }
            rest = &after[end + 1..];
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_commit_title_is_thin() {
        let s = StatementSignals::of(
            "fix(flink): preserve TYK checkpoints and fail on serialization errors",
        );
        assert!(s.thin, "{s:?}");
        assert!(s.words < MIN_WORDS);
        assert!(s.summary_line().is_some());
    }

    #[test]
    fn a_rewritten_statement_is_not_thin() {
        let s = StatementSignals::of(
            "ZenRows sometimes answers IDX PDF fetches with HTTP 422 transiently, but the scraperly \
             fetcher treats 422 as deterministic and never retries it. Make `fetch_pdf` in \
             scraperly/fetch.py retry a 422 up to `MAX_RETRIES` times with the existing backoff, \
             raise FetchExhausted with the message \"zenrows 422 after 3 attempts\" when it still \
             fails, and leave 4xx codes other than 422 non-retried. The helper is_retryable_status \
             must return True for 422 and False for 404.",
        );
        assert!(!s.thin, "{s:?}");
        assert!(s.identifiers >= 5, "{s:?}");
        assert_eq!(s.paths, 1);
        assert!(s.quoted >= 1);
        assert!(s.summary_line().is_none());
    }

    #[test]
    fn plain_prose_of_any_length_stays_thin() {
        let prose =
            "please make the service faster and more reliable when the upstream is slow ".repeat(8);
        let s = StatementSignals::of(&prose);
        assert!(s.words >= MIN_WORDS);
        assert_eq!(s.identifiers, 0);
        assert!(s.thin);
    }
}
