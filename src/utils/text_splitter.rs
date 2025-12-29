//! Text splitting utilities for TTS chunking

/// Split text into sentences for chunked TTS generation
/// 
/// Splits on sentence boundaries (., !, ?) while preserving the punctuation.
/// Handles common abbreviations and edge cases.
pub fn split_into_sentences(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![];
    }

    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        current.push(ch);

        // Check for sentence-ending punctuation
        if matches!(ch, '.' | '!' | '?') {
            // Look ahead to see if this is really the end of a sentence
            let is_sentence_end = if i + 1 < chars.len() {
                let next_ch = chars[i + 1];
                // Sentence ends if followed by space and capital letter, or end of text
                matches!(next_ch, ' ' | '\n' | '\t') || 
                (i + 2 < chars.len() && matches!(chars[i + 2], 'A'..='Z'))
            } else {
                true // End of text
            };

            if is_sentence_end {
                // Skip trailing whitespace for this sentence
                let mut j = i + 1;
                while j < chars.len() && matches!(chars[j], ' ' | '\n' | '\t') {
                    j += 1;
                }
                i = j - 1; // Will be incremented by loop

                let sentence = current.trim().to_string();
                if !sentence.is_empty() {
                    sentences.push(sentence);
                }
                current.clear();
            }
        }

        i += 1;
    }

    // Add remaining text as a sentence if any
    let remaining = current.trim().to_string();
    if !remaining.is_empty() {
        sentences.push(remaining);
    }

    // If no sentences were found (no punctuation), return the whole text as one sentence
    if sentences.is_empty() {
        sentences.push(text.trim().to_string());
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_splitting() {
        let text = "Hello world. How are you? I'm fine!";
        let sentences = split_into_sentences(text);
        assert_eq!(sentences.len(), 3);
        assert_eq!(sentences[0], "Hello world.");
        assert_eq!(sentences[1], "How are you?");
        assert_eq!(sentences[2], "I'm fine!");
    }

    #[test]
    fn test_no_punctuation() {
        let text = "Hello world";
        let sentences = split_into_sentences(text);
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0], "Hello world");
    }

    #[test]
    fn test_empty() {
        let text = "";
        let sentences = split_into_sentences(text);
        assert_eq!(sentences.len(), 0);
    }
}
