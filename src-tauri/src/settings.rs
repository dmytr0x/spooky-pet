use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::{Manager, Runtime};

const SETTINGS_FILE: &str = "settings.json";
const DEFAULT_MODEL: &str = "google/gemma-4-31b-it";
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMER_MIN_SECONDS: u64 = 300;
const DEFAULT_TIMER_MAX_SECONDS: u64 = 600;
const DEFAULT_COOLDOWN_SECONDS: u64 = 60;
const DEFAULT_FONT_SIZE: u32 = 18;
const FONT_SIZE_MIN: u32 = 11;
const FONT_SIZE_MAX: u32 = 32;
const DEFAULT_POPUP_OPACITY: f64 = 0.9;
const POPUP_OPACITY_MIN: f64 = 0.2;
const POPUP_OPACITY_MAX: f64 = 1.0;
const DEFAULT_CHUNK_DELAY_SECONDS: u64 = 8;
const CHUNK_DELAY_MIN: u64 = 1;
const CHUNK_DELAY_MAX: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RoastLevel {
    #[default]
    Gentle,
    Mean,
    Cursed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub role: String,
    pub interests: String,
    pub roast_level: RoastLevel,
    pub do_not_joke: Vec<String>,
    pub pet_name: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: String::new(),
            role: String::new(),
            interests: String::new(),
            roast_level: RoastLevel::Gentle,
            do_not_joke: Vec::new(),
            pet_name: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub timer_min_seconds: u64,
    pub timer_max_seconds: u64,
    pub cooldown_seconds: u64,
    pub sound: bool,
    pub font_size: u32,
    pub popup_opacity: f64,
    pub chunk_delay_seconds: u64,
    pub profile: Profile,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: DEFAULT_MODEL.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timer_min_seconds: DEFAULT_TIMER_MIN_SECONDS,
            timer_max_seconds: DEFAULT_TIMER_MAX_SECONDS,
            cooldown_seconds: DEFAULT_COOLDOWN_SECONDS,
            sound: true,
            font_size: DEFAULT_FONT_SIZE,
            popup_opacity: DEFAULT_POPUP_OPACITY,
            chunk_delay_seconds: DEFAULT_CHUNK_DELAY_SECONDS,
            profile: Profile::default(),
        }
    }
}

impl Settings {
    pub fn joke_font_size(&self) -> u32 {
        self.font_size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX)
    }

    pub fn joke_popup_opacity(&self) -> f64 {
        self.popup_opacity
            .clamp(POPUP_OPACITY_MIN, POPUP_OPACITY_MAX)
    }

    pub fn joke_chunk_delay_seconds(&self) -> u64 {
        self.chunk_delay_seconds
            .clamp(CHUNK_DELAY_MIN, CHUNK_DELAY_MAX)
    }

    pub fn timer_delay_range(&self) -> (u64, u64) {
        let a = self.timer_min_seconds.max(1);
        let b = self.timer_max_seconds.max(1);
        if a <= b { (a, b) } else { (b, a) }
    }
}

/// Re-read `settings.json` on each heckle and before playing sound.
pub fn load_or_init<R: Runtime>(app: &impl Manager<R>) -> Result<Settings, String> {
    let dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    load_or_init_at(&dir)
}

pub fn load_or_init_at(dir: &Path) -> Result<Settings, String> {
    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let path = dir.join(SETTINGS_FILE);
    if !path.exists() {
        let settings = Settings::default();
        write_settings(&path, &settings)?;
        return Ok(settings);
    }
    let raw = fs::read_to_string(&path).map_err(|err| err.to_string())?;
    serde_json::from_str(&raw).map_err(|err| err.to_string())
}

fn write_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    let mut json = serde_json::to_string_pretty(settings).map_err(|err| err.to_string())?;
    json.push('\n');
    fs::write(path, json).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir() -> std::path::PathBuf {
        let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("spooky-pet-settings-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = scratch_dir();
        let settings = load_or_init_at(&dir).expect("init");
        let path = dir.join(SETTINGS_FILE);
        let written = fs::read_to_string(&path).expect("read defaults");

        assert_eq!(settings, Settings::default());
        assert!(written.contains("\"model\": \"google/gemma-4-31b-it\""));
        assert!(written.contains("\"timer_min_seconds\": 300"));
        assert!(written.contains("\"timer_max_seconds\": 600"));
        assert!(written.contains("\"cooldown_seconds\": 60"));
        assert!(written.contains("\"sound\": true"));
        assert!(written.contains("\"font_size\": 18"));
        assert!(written.contains("\"popup_opacity\": 0.9"));
        assert!(written.contains("\"chunk_delay_seconds\": 8"));
        assert!(written.contains("\"roast_level\": \"gentle\""));
        assert!(written.contains("\"base_url\": \"https://openrouter.ai/api/v1\""));
        assert!(written.contains("\"do_not_joke\": []"));
        assert!(written.contains("\"api_key\": \"\""));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hand_edit_is_returned() {
        let dir = scratch_dir();
        load_or_init_at(&dir).expect("init");
        let path = dir.join(SETTINGS_FILE);
        fs::write(
            &path,
            r#"{
  "api_key": "sk-test",
  "model": "gpt-4o",
  "base_url": "http://127.0.0.1:11434/v1",
  "timer_min_seconds": 20,
  "timer_max_seconds": 40,
  "cooldown_seconds": 12,
  "sound": false,
  "font_size": 14,
  "popup_opacity": 0.5,
  "chunk_delay_seconds": 3,
  "profile": {
    "name": "Dmytro",
    "role": "engineer",
    "interests": "cats",
    "roast_level": "cursed",
    "do_not_joke": ["meetings"],
    "pet_name": "Bones"
  }
}
"#,
        )
        .expect("hand-edit");

        let settings = load_or_init_at(&dir).expect("reload");
        assert_eq!(settings.api_key, "sk-test");
        assert_eq!(settings.model, "gpt-4o");
        assert_eq!(settings.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(settings.timer_min_seconds, 20);
        assert_eq!(settings.timer_max_seconds, 40);
        assert_eq!(settings.timer_delay_range(), (20, 40));
        assert_eq!(settings.cooldown_seconds, 12);
        assert!(!settings.sound);
        assert_eq!(settings.font_size, 14);
        assert_eq!(settings.joke_font_size(), 14);
        assert_eq!(settings.popup_opacity, 0.5);
        assert_eq!(settings.joke_popup_opacity(), 0.5);
        assert_eq!(settings.chunk_delay_seconds, 3);
        assert_eq!(settings.joke_chunk_delay_seconds(), 3);
        assert_eq!(settings.profile.name, "Dmytro");
        assert_eq!(settings.profile.roast_level, RoastLevel::Cursed);
        assert_eq!(settings.profile.do_not_joke, vec!["meetings"]);
        assert_eq!(settings.profile.pet_name, "Bones");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn omitted_font_size_uses_default_and_clamps() {
        let dir = scratch_dir();
        load_or_init_at(&dir).expect("init");
        let path = dir.join(SETTINGS_FILE);
        fs::write(
            &path,
            r#"{
  "api_key": "",
  "model": "google/gemma-4-31b-it",
  "base_url": "https://openrouter.ai/api/v1",
  "timer_min_seconds": 300,
  "timer_max_seconds": 600,
  "cooldown_seconds": 60,
  "sound": true,
  "profile": {
    "name": "",
    "role": "",
    "interests": "",
    "roast_level": "gentle",
    "do_not_joke": [],
    "pet_name": ""
  }
}
"#,
        )
        .expect("omit font_size");

        let settings = load_or_init_at(&dir).expect("reload");
        assert_eq!(settings.font_size, 18);
        assert_eq!(settings.joke_font_size(), 18);
        assert_eq!(settings.popup_opacity, 0.9);
        assert_eq!(settings.joke_popup_opacity(), 0.9);
        assert_eq!(settings.chunk_delay_seconds, 8);
        assert_eq!(settings.joke_chunk_delay_seconds(), 8);

        let mut too_small = Settings::default();
        too_small.font_size = 0;
        assert_eq!(too_small.joke_font_size(), 11);
        too_small.popup_opacity = 0.0;
        assert_eq!(too_small.joke_popup_opacity(), 0.2);
        too_small.chunk_delay_seconds = 0;
        assert_eq!(too_small.joke_chunk_delay_seconds(), 1);
        too_small.timer_min_seconds = 0;
        too_small.timer_max_seconds = 0;
        assert_eq!(too_small.timer_delay_range(), (1, 1));
        too_small.timer_min_seconds = 90;
        too_small.timer_max_seconds = 10;
        assert_eq!(too_small.timer_delay_range(), (10, 90));

        let mut too_big = Settings::default();
        too_big.font_size = 99;
        assert_eq!(too_big.joke_font_size(), 32);
        too_big.popup_opacity = 2.0;
        assert_eq!(too_big.joke_popup_opacity(), 1.0);
        too_big.chunk_delay_seconds = 99;
        assert_eq!(too_big.joke_chunk_delay_seconds(), 60);

        let _ = fs::remove_dir_all(&dir);
    }
}
