//! A tolerant reader for the Markdown notes of a file-based Limina knowledge base: YAML-ish
//! frontmatter, an `# ID — Title` heading, `> **Key**: value` metadata lines and `##` sections.

use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Document {
    /// Frontmatter scalars, with quotes removed. Keys are lowercase.
    pub frontmatter: BTreeMap<String, String>,
    /// The first `#` heading.
    pub title: Option<String>,
    /// `> **Key**: value` lines before the first section. Keys are lowercase.
    pub metadata: BTreeMap<String, String>,
    /// Text before the first `##` section, without the title and metadata lines.
    pub preamble: String,
    /// `##` sections in document order: (heading, body). Bodies keep their `###` subsections.
    pub sections: Vec<(String, String)>,
}

impl Document {
    pub fn parse(text: &str) -> Self {
        let text = text.replace("\r\n", "\n");
        let (frontmatter, body) = split_frontmatter(&text);
        let mut document = Self { frontmatter: parse_frontmatter(frontmatter), ..Self::default() };
        let mut current: Option<(String, Vec<&str>)> = None;
        let mut preamble = Vec::new();
        let mut in_fence = false;
        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
            }
            if !in_fence {
                if let Some(heading) = line.strip_prefix("## ") {
                    if let Some((title, lines)) = current.take() {
                        document.sections.push((title, join(&lines)));
                    }
                    current = Some((heading.trim().to_owned(), Vec::new()));
                    continue;
                }
                if current.is_none() {
                    if let Some(title) = line.strip_prefix("# ")
                        && document.title.is_none()
                    {
                        document.title = Some(title.trim().to_owned());
                        continue;
                    }
                    if let Some((key, value)) = metadata_line(line) {
                        document.metadata.insert(key, value);
                        continue;
                    }
                }
            }
            match current.as_mut() {
                Some((_, lines)) => lines.push(line),
                None => preamble.push(line),
            }
        }
        if let Some((title, lines)) = current {
            document.sections.push((title, join(&lines)));
        }
        document.preamble = join(&preamble);
        document
    }

    /// A frontmatter value, falling back to the metadata block.
    pub fn field(&self, key: &str) -> Option<&str> {
        self.frontmatter
            .get(key)
            .or_else(|| self.metadata.get(key))
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// The body of the first section whose heading matches (case-insensitively).
    pub fn section(&self, heading: &str) -> Option<&str> {
        self.sections
            .iter()
            .find(|(title, _)| title.eq_ignore_ascii_case(heading))
            .map(|(_, body)| body.as_str())
    }
}

fn join(lines: &[&str]) -> String {
    lines.join("\n").trim().to_owned()
}

fn split_frontmatter(text: &str) -> (&str, &str) {
    let Some(rest) = text.strip_prefix("---\n") else { return ("", text) };
    match rest.find("\n---") {
        Some(end) => {
            let after = &rest[end + 4..];
            (&rest[..end], after.strip_prefix('\n').unwrap_or(after))
        }
        None => ("", text),
    }
}

fn parse_frontmatter(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|line| !line.starts_with([' ', '\t', '-', '#']))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_lowercase(), unquote(value.trim()).to_owned()))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote).and_then(|v| v.strip_suffix(quote)) {
            return inner;
        }
    }
    value
}

/// `> **Status**: CONFIRMED` → ("status", "CONFIRMED").
fn metadata_line(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix('>')?.trim().strip_prefix("**")?;
    let (key, value) = rest.split_once("**")?;
    let key = key.trim().trim_end_matches(':').trim().to_lowercase();
    let value = value.trim().trim_start_matches(':').trim().to_owned();
    (!key.is_empty()).then_some((key, value))
}

/// The first artifact ID with one of the prefixes in a value such as `"[[H001]]"` or `H001, H002`.
pub fn artifact_reference(value: &str, prefix: char) -> Option<String> {
    let bytes: Vec<char> = value.chars().collect();
    for (index, character) in bytes.iter().enumerate() {
        if character.to_ascii_uppercase() != prefix {
            continue;
        }
        let previous_is_word = index > 0 && bytes[index - 1].is_alphanumeric();
        let digits: String = bytes[index + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if !previous_is_word && digits.len() >= 3 {
            return Some(format!("{prefix}{digits}"));
        }
    }
    None
}

/// The title from a heading such as `H001 — Title`, `H001 - Title` or `H001: Title`.
pub fn strip_artifact_prefix(title: &str, artifact_id: &str) -> String {
    let Some(rest) = title.strip_prefix(artifact_id) else { return title.trim().to_owned() };
    rest.trim_start().trim_start_matches(['—', '–', '-', ':']).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\nid: \"H001\"\naliases: [\"H001\"]\nstatus: CONFIRMED\ncreated: \"2026-04-15\"\n---\n\n# H001 — Retrieval is the bottleneck\n\n> **Status**: CONFIRMED\n> **Created**: 2026-04-15\n\nIntro line.\n\n## Statement\n\nIf X then Y.\n\n```text\n## not a heading\n```\n\n## Links\n\n- [[E001]]\n";

    #[test]
    fn notes_parse_into_frontmatter_title_metadata_and_sections() {
        let document = Document::parse(NOTE);
        assert_eq!(document.field("status"), Some("CONFIRMED"));
        assert_eq!(document.field("created"), Some("2026-04-15"));
        assert_eq!(document.title.as_deref(), Some("H001 — Retrieval is the bottleneck"));
        assert_eq!(document.metadata["status"], "CONFIRMED");
        assert_eq!(document.preamble, "Intro line.");
        assert_eq!(document.sections.len(), 2);
        assert_eq!(
            document.section("statement"),
            Some("If X then Y.\n\n```text\n## not a heading\n```")
        );
    }

    #[test]
    fn references_and_titles_are_normalized() {
        assert_eq!(artifact_reference("\"[[H014]]\"", 'H').as_deref(), Some("H014"));
        assert_eq!(artifact_reference("see E016, E017", 'E').as_deref(), Some("E016"));
        assert_eq!(artifact_reference("THE001", 'E'), None);
        assert_eq!(
            strip_artifact_prefix("F003 — Abstention separates", "F003"),
            "Abstention separates"
        );
        assert_eq!(strip_artifact_prefix("Plain title", "F003"), "Plain title");
    }
}
