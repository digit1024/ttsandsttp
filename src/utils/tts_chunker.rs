//! TTS request text chunking.
//!
//! Port of LunaAI's `tts_text_chunker.dart`: splits text into chunks that fit
//! a provider's per-request character budget, preferring sentence and then
//! word boundaries. The first chunk is capped shorter so speech starts sooner
//! while later chunks are still being synthesized.

/// Default max chars per remote TTS API request.
pub const DEFAULT_TTS_MAX_INPUT_LENGTH: usize = 600;

/// Default max chars for the first chunk of a message.
pub const DEFAULT_TTS_FIRST_CHUNK_LENGTH: usize = 200;

/// Tokens after which a full stop does not end a sentence.
const ABBREVIATIONS: &[&str] = &[
    "al", "approx", "apt", "ave", "blvd", "capt", "cf", "dept", "dr", "e.g", "etc", "fig", "gen",
    "i.e", "inc", "jr", "ltd", "mr", "mrs", "ms", "no", "prof", "rd", "rev", "sr", "st", "univ",
    "vol", "vs",
];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.'
}

/// True when the terminator at `terminator_index` belongs to an abbreviation
/// or an initial ("J. K. Rowling").
fn is_abbreviated(chars: &[char], terminator_index: usize) -> bool {
    let mut begin = terminator_index;
    while begin > 0 && is_token_char(chars[begin - 1]) {
        begin -= 1;
    }
    let token: String = chars[begin..terminator_index]
        .iter()
        .collect::<String>()
        .to_lowercase();
    if token.is_empty() {
        return false;
    }
    if ABBREVIATIONS.contains(&token.as_str()) {
        return true;
    }
    // Single letters are initials.
    token.chars().count() == 1 && token.starts_with(|c: char| c.is_ascii_lowercase())
}

/// Index just past the last sentence terminator at or before `end`, skipping
/// abbreviations and initials. Bounded scan: a run of abbreviations must not
/// cost a pass over the whole message.
fn last_sentence_end(chars: &[char], start: usize, end: usize) -> Option<usize> {
    let end = end.min(chars.len());
    if end == 0 {
        return None;
    }
    let mut from = end - 1;
    for _ in 0..4 {
        if from <= start {
            return None;
        }
        // Find the last terminator at index <= from followed by whitespace.
        let mut found = None;
        let mut i = from;
        loop {
            if i <= start {
                break;
            }
            if matches!(chars[i], '.' | '!' | '?')
                && i + 1 < chars.len()
                && chars[i + 1].is_whitespace()
            {
                found = Some(i);
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        let m = found?;
        if !is_abbreviated(chars, m) {
            return Some(m + 1);
        }
        if m == 0 {
            return None;
        }
        from = m - 1;
    }
    None
}

/// Exclusive end index of the chunk starting at `start`, never further than
/// `end` and always greater than `start` so callers make progress.
fn find_cut(chars: &[char], start: usize, end: usize) -> usize {
    if let Some(sentence) = last_sentence_end(chars, start, end) {
        return sentence;
    }
    let end = end.min(chars.len());
    if let Some(pos) = (start + 1..=end).rev().find(|&i| chars[i - 1] == ' ') {
        // Last space at index <= end: cut after it.
        let last_space = pos - 1;
        if last_space > start {
            return last_space + 1;
        }
    }
    end
}

/// Split text into chunks no longer than `max_len` chars, preferring sentence
/// and then word boundaries. `first_max_len` caps only the first chunk.
pub fn chunk_text_for_tts(text: &str, max_len: usize, first_max_len: Option<usize>) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let limit = max_len.max(1);
    let first_limit = first_max_len.unwrap_or(limit).clamp(1, limit);

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut budget = first_limit;

    while start < chars.len() {
        if chars.len() - start <= budget {
            let tail: String = chars[start..].iter().collect();
            let tail = tail.trim();
            if !tail.is_empty() {
                chunks.push(tail.to_string());
            }
            break;
        }

        let cut = find_cut(&chars, start, start + budget);
        let piece: String = chars[start..cut].iter().collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            chunks.push(piece.to_string());
        }
        start = cut;
        budget = limit;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text() {
        assert!(chunk_text_for_tts("", 600, None).is_empty());
        assert!(chunk_text_for_tts("   ", 600, None).is_empty());
    }

    #[test]
    fn short_text_single_chunk() {
        let chunks = chunk_text_for_tts("Hello world.", 600, Some(200));
        assert_eq!(chunks, vec!["Hello world."]);
    }

    #[test]
    fn splits_at_sentence_boundary() {
        let text = "First sentence here. Second sentence follows it.";
        let chunks = chunk_text_for_tts(text, 25, Some(25));
        assert_eq!(chunks[0], "First sentence here.");
        // Remainder exceeds the budget, so it is cut at word boundaries.
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.chars().count() <= 25));
        assert!(chunks.join(" ").contains("follows"));
    }

    #[test]
    fn first_chunk_is_shorter() {
        let text = "One. Two three four five six seven eight nine ten eleven twelve.";
        let chunks = chunk_text_for_tts(text, 30, Some(6));
        assert_eq!(chunks[0], "One.");
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= 30);
        }
    }

    #[test]
    fn abbreviations_do_not_split() {
        let text = "Talk to Dr. Smith about the e.g. cases and then go home.";
        let chunks = chunk_text_for_tts(text, 30, Some(30));
        assert!(chunks.iter().any(|c| c.contains("Dr. Smith")));
        assert!(chunks.iter().any(|c| c.contains("e.g.")));
    }

    #[test]
    fn long_word_forces_hard_cut() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbb";
        let chunks = chunk_text_for_tts(text, 10, Some(10));
        assert!(chunks.iter().all(|c| c.chars().count() <= 10));
        let rejoined = chunks.join("");
        assert!(rejoined.contains(&"a".repeat(44)));
    }

    #[test]
    fn no_characters_dropped() {
        let text = "Alpha beta. Gamma delta epsilon zeta. Eta theta iota.";
        let chunks = chunk_text_for_tts(text, 20, Some(12));
        let rejoined = chunks.join(" ");
        for word in text.split_whitespace() {
            assert!(rejoined.contains(word.trim_end_matches('.')) || rejoined.contains(word));
        }
    }
}
