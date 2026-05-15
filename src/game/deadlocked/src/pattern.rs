use anyhow::{Context, Result, bail};

const MAX_GAP_BYTES: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternToken {
    Exact(u8),
    Any,
    Gap,
}

pub fn parse_pattern(pattern: &str) -> Result<Vec<PatternToken>> {
    pattern
        .split_whitespace()
        .map(|token| match token {
            "?" | "??" => Ok(PatternToken::Any),
            "..." => Ok(PatternToken::Gap),
            _ => {
                if token.len() != 2 {
                    bail!("invalid pattern token `{token}`");
                }
                let value = u8::from_str_radix(token, 16)
                    .with_context(|| format!("invalid hex token `{token}`"))?;
                Ok(PatternToken::Exact(value))
            }
        })
        .collect()
}

pub fn find_matches(bytes: &[u8], pattern: &[PatternToken], limit: usize) -> Vec<usize> {
    if pattern.is_empty() || bytes.is_empty() || limit == 0 {
        return Vec::new();
    }

    let segments = split_segments(pattern);
    let Some(first) = segments.first() else {
        return Vec::new();
    };
    if bytes.len() < first.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for start in 0..=bytes.len() - first.len() {
        if !segment_matches(bytes, start, first) {
            continue;
        }
        if match_remaining_segments(bytes, &segments, 1, start + first.len()) {
            matches.push(start);
            if matches.len() >= limit {
                break;
            }
        }
    }
    matches
}

fn split_segments(pattern: &[PatternToken]) -> Vec<Vec<PatternToken>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();

    for token in pattern {
        if *token == PatternToken::Gap {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
        } else {
            current.push(token.clone());
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    segments
}

fn match_remaining_segments(
    bytes: &[u8],
    segments: &[Vec<PatternToken>],
    segment_index: usize,
    search_start: usize,
) -> bool {
    if segment_index >= segments.len() {
        return true;
    }

    let segment = &segments[segment_index];
    if segment.is_empty() || search_start >= bytes.len() {
        return false;
    }

    let max_start = bytes.len().saturating_sub(segment.len());
    let search_end = max_start.min(search_start.saturating_add(MAX_GAP_BYTES));

    for start in search_start..=search_end {
        if !segment_matches(bytes, start, segment) {
            continue;
        }
        if match_remaining_segments(bytes, segments, segment_index + 1, start + segment.len()) {
            return true;
        }
    }

    false
}

fn segment_matches(bytes: &[u8], start: usize, segment: &[PatternToken]) -> bool {
    let Some(end) = start.checked_add(segment.len()) else {
        return false;
    };
    let Some(window) = bytes.get(start..end) else {
        return false;
    };

    segment
        .iter()
        .zip(window.iter())
        .all(|(token, byte)| match token {
            PatternToken::Exact(expected) => *expected == *byte,
            PatternToken::Any => true,
            PatternToken::Gap => false,
        })
}
