use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conf {
    pub window_title: String,
    pub window_width: u32,
    pub window_height: u32,
}

impl Default for Conf {
    fn default() -> Self {
        Self {
            window_title: "SLUDGE \\m/".to_string(),
            window_width: 800,
            window_height: 680,
        }
    }
}
