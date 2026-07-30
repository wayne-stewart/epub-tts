//! Plain-text helpers for EPUB content and TTS-friendly chunking.

use regex::Regex;
use std::sync::LazyLock;

static MULTI_SPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[ \t]+").expect("valid regex"));
static MULTI_NEWLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n{3,}").expect("valid regex"));

/// Convert XHTML/HTML chapter markup into readable plain text.
pub fn html_to_text(html: &str) -> String {
    // Prefer body content when present; many EPUB chapters wrap everything.
    let fragment = extract_body(html).unwrap_or(html);
    let text = html2text::from_read(fragment.as_bytes(), 100).unwrap_or_default();
    normalize_whitespace(&text)
}

fn extract_body(html: &str) -> Option<&str> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<body")?;
    let after = start + "<body".len();
    let open_end = lower[after..].find('>')? + after + 1;
    let end = lower[open_end..].find("</body")? + open_end;
    Some(&html[open_end..end])
}

/// Collapse redundant whitespace while preserving paragraph breaks.
pub fn normalize_whitespace(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = MULTI_SPACE.replace_all(&text, " ");
    let text = MULTI_NEWLINE.replace_all(&text, "\n\n");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Split long text into chunks suitable for TTS models.
///
/// Prefers paragraph and sentence boundaries so speech cadence stays natural.
pub fn chunk_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(32);
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        if para.chars().count() > max_chars {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            for sentence_chunk in split_long_paragraph(para, max_chars) {
                chunks.push(sentence_chunk);
            }
            continue;
        }

        let extra = if current.is_empty() {
            para.chars().count()
        } else {
            para.chars().count() + 2
        };

        if current.chars().count() + extra > max_chars && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }

        if current.is_empty() {
            current.push_str(para);
        } else {
            current.push_str("\n\n");
            current.push_str(para);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

fn split_long_paragraph(para: &str, max_chars: usize) -> Vec<String> {
    let sentences = split_sentences(para);
    let mut out = Vec::new();
    let mut current = String::new();

    for sentence in sentences {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }

        if sentence.chars().count() > max_chars {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.extend(hard_wrap(sentence, max_chars));
            continue;
        }

        let extra = if current.is_empty() {
            sentence.chars().count()
        } else {
            sentence.chars().count() + 1
        };

        if current.chars().count() + extra > max_chars && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }

        if current.is_empty() {
            current.push_str(sentence);
        } else {
            current.push(' ');
            current.push_str(sentence);
        }
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        if matches!(c, '.' | '!' | '?') {
            let next_is_space_or_end = i + 1 >= chars.len() || chars[i + 1].is_whitespace();
            if next_is_space_or_end {
                let end = i + 1;
                let slice: String = chars[start..end].iter().collect();
                let trimmed = slice.trim();
                if !trimmed.is_empty() {
                    sentences.push(trimmed.to_string());
                }
                let mut j = end;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }

    if start < chars.len() {
        let slice: String = chars[start..].iter().collect();
        let trimmed = slice.trim();
        if !trimmed.is_empty() {
            sentences.push(trimmed.to_string());
        }
    }

    if sentences.is_empty() {
        sentences.push(text.to_string());
    }
    sentences
}

fn hard_wrap(text: &str, max_chars: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    let mut current = String::new();

    for word in words {
        let extra = if current.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };

        if current.chars().count() + extra > max_chars && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }

        if current.is_empty() {
            current.push_str(word);
        } else {
            current.push(' ');
            current.push_str(word);
        }
    }

    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_simple_html() {
        let html = r#"<html><body><p>Hello <b>world</b>.</p><p>Second paragraph.</p></body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(text.contains("Second paragraph"));
    }

    #[test]
    fn chunks_respect_max() {
        let text = "Alpha sentence number one. Beta sentence number two. Gamma sentence number three.";
        let chunks = chunk_for_tts(text, 40);
        assert!(
            chunks.iter().all(|c| c.chars().count() <= 40),
            "oversized chunks: {chunks:?}"
        );
        assert!(chunks.len() >= 2, "expected multiple chunks, got {chunks:?}");
    }
}
