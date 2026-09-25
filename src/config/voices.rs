//! Curated Qwen-TTS voice catalogue (embedded from `src/voices.json`)

use serde::Deserialize;

/// A selectable DashScope TTS voice
#[derive(Debug, Clone, Deserialize)]
pub struct Voice {
    pub id: String,
    #[serde(default)]
    pub gender: String,
    #[serde(default)]
    pub description: String,
}

/// Load the bundled voice catalogue.
pub fn load_voices() -> Vec<Voice> {
    let data = include_str!("../voices.json");
    let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("voices")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Whether `id` matches a known bundled voice (case-insensitive).
pub fn is_known_voice(id: &str) -> bool {
    load_voices()
        .iter()
        .any(|v| v.id.eq_ignore_ascii_case(id))
}
