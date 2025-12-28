//! Language utility functions

/// Normalize a language code by extracting the base language.
/// 
/// Converts language codes like "pl-PL" or "en-US" to their base form "pl" or "en".
/// Also converts to lowercase.
/// 
/// # Examples
/// ```
/// use ttsandsttp::utils::normalize_language_code;
/// 
/// assert_eq!(normalize_language_code("pl-PL"), "pl");
/// assert_eq!(normalize_language_code("en-US"), "en");
/// assert_eq!(normalize_language_code("fr"), "fr");
/// assert_eq!(normalize_language_code("EN"), "en");
/// ```
pub fn normalize_language_code(lang: &str) -> String {
    lang.split('-').next().unwrap_or(lang).to_lowercase()
}

