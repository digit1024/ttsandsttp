use std::sync::OnceLock;
use std::time::Instant;

/// Format timestamp for logging (HH:MM:SS.mmm) - relative to program start
static START_TIME: OnceLock<Instant> = OnceLock::new();

/// Get a formatted timestamp string (HH:MM:SS.mmm) relative to program start
pub fn format_timestamp() -> String {
    let start = START_TIME.get_or_init(|| Instant::now());
    let elapsed = start.elapsed();
    let total_ms = elapsed.as_millis();
    let total_secs = total_ms / 1000;
    let ms = total_ms % 1000;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = (total_secs / 3600) % 24; // Only show hours 0-23
    format!("{:02}:{:02}:{:02}.{:03}", hours, mins, secs, ms)
}


