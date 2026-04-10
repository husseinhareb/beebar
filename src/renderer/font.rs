/// Font loading and management utilities.
pub struct FontManager {
    pub default_family: String,
    pub default_size: f64,
}

impl Default for FontManager {
    fn default() -> Self {
        Self {
            default_family: "monospace".to_string(),
            default_size: 14.0,
        }
    }
}
