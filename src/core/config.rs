use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

const APP_DIR_NAME: &str = "safeeyes-rs";
const CONFIG_FILE_NAME: &str = "config.toml";
const PROFILES_DIR_NAME: &str = "profiles";
const ACTIVE_PROFILE_FILE_NAME: &str = "active_profile";
pub const DEFAULT_PROFILE_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusProfile {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub is_default: bool,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine XDG config directory")]
    MissingConfigDir,
    #[error("invalid focus mode name '{0}'")]
    InvalidProfileName(String),
    #[error("focus mode '{0}' already exists")]
    ProfileAlreadyExists(String),
    #[error("cannot delete the default focus mode")]
    CannotDeleteDefault,
    #[error("focus mode '{0}' not found")]
    ProfileNotFound(String),
    #[error("failed to read config file at {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write config file at {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse TOML config at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimerMode {
    /// Pause countdown on user idle and reset after `idle_reset_seconds` (respects video/media idle inhibitors).
    #[default]
    SmartPause,
    /// Ignore user inactivity and always schedule breaks strictly by elapsed time since the last break.
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Timer behavior regarding user inactivity (`smart_pause` or `strict`).
    pub timer_mode: TimerMode,
    /// Interval between breaks in minutes.
    pub short_break_interval_minutes: u64,
    /// Duration of a short break in seconds.
    pub short_break_duration_seconds: u64,
    /// Number of short breaks before a long break (0 disables long breaks).
    pub short_breaks_before_long: u32,
    /// Duration of a long break in seconds.
    pub long_break_duration_seconds: u64,
    /// Seconds before a break to show a pre-break notification (0 disables notifications).
    pub pre_break_warning_seconds: u64,
    /// Maximum number of consecutive break skips/snoozes allowed before reset.
    pub max_skips: u32,
    /// Duration in minutes to postpone a break when clicking Snooze.
    pub snooze_duration_minutes: u64,
    /// Seconds of user inactivity before pausing the break countdown (0 disables idle detection).
    pub idle_pause_seconds: u64,
    /// Seconds of user inactivity after which the break timer resets on resume.
    pub idle_reset_seconds: u64,
    /// Play subtle chimes when a break starts and ends.
    pub enable_sound: bool,
    /// Optional custom audio file path played when a break starts.
    pub start_sound_path: Option<String>,
    /// Optional custom audio file path played when a break completes.
    pub end_sound_path: Option<String>,
    /// Prompts cycled during short breaks.
    pub short_break_exercises: Vec<String>,
    /// Prompts cycled during long breaks.
    pub long_break_exercises: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timer_mode: TimerMode::SmartPause,
            short_break_interval_minutes: 15,
            short_break_duration_seconds: 15,
            short_breaks_before_long: 3,
            long_break_duration_seconds: 60,
            pre_break_warning_seconds: 10,
            max_skips: 2,
            snooze_duration_minutes: 5,
            idle_pause_seconds: 300,
            idle_reset_seconds: 300,
            enable_sound: true,
            start_sound_path: None,
            end_sound_path: None,
            short_break_exercises: vec![
                "Look at something at least 6 meters away".to_string(),
                "Close your eyes tightly, then relax and blink slowly".to_string(),
                "Roll your eyes gently in slow circles".to_string(),
                "Focus far away, then slowly focus on a nearby object".to_string(),
                "Rest your palms gently over your closed eyes".to_string(),
            ],
            long_break_exercises: vec![
                "Stand up, stretch your back and shoulders, and look out a window".to_string(),
                "Walk around for a moment and drink a glass of water".to_string(),
                "Roll your shoulders back and gently stretch your neck".to_string(),
            ],
        }
    }
}

impl Config {
    /// Returns `$XDG_CONFIG_HOME/safeeyes-rs` or `~/.config/safeeyes-rs`.
    pub fn app_config_dir() -> Result<PathBuf, ConfigError> {
        let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
            && !xdg.is_empty()
        {
            PathBuf::from(xdg)
        } else if let Some(home) = std::env::var_os("HOME")
            && !home.is_empty()
        {
            PathBuf::from(home).join(".config")
        } else {
            return Err(ConfigError::MissingConfigDir);
        };

        Ok(base.join(APP_DIR_NAME))
    }

    /// Returns `$XDG_CONFIG_HOME/safeeyes-rs/config.toml` or `~/.config/safeeyes-rs/config.toml`.
    pub fn config_path() -> Result<PathBuf, ConfigError> {
        Ok(Self::app_config_dir()?.join(CONFIG_FILE_NAME))
    }

    /// Returns `$XDG_CONFIG_HOME/safeeyes-rs/profiles`.
    pub fn profiles_dir() -> Result<PathBuf, ConfigError> {
        Ok(Self::app_config_dir()?.join(PROFILES_DIR_NAME))
    }

    /// Loads config from disk, creating a documented default file if missing.
    pub fn load_or_create_default() -> Result<(Self, PathBuf), ConfigError> {
        let dir = Self::app_config_dir()?;
        Self::load_or_create_default_in(&dir)
    }

    /// Lists all focus profiles (`Default` + `profiles/*.toml`), seeding starter profiles once.
    pub fn list_profiles() -> Result<Vec<FocusProfile>, ConfigError> {
        let dir = Self::app_config_dir()?;
        Self::list_profiles_in(&dir)
    }

    /// Loads the active focus profile, its sanitized `Config`, and all available profiles.
    pub fn load_active_profile() -> Result<(FocusProfile, Config, Vec<FocusProfile>), ConfigError> {
        let dir = Self::app_config_dir()?;
        Self::load_active_profile_in(&dir)
    }

    /// Persists the active focus mode identifier (`default` or profile slug).
    pub fn save_active_profile_id(id: &str) -> Result<(), ConfigError> {
        let dir = Self::app_config_dir()?;
        Self::save_active_profile_id_in(&dir, id)
    }

    /// Creates a new focus mode file in `profiles/` and marks it active.
    pub fn create_profile(name: &str, cfg: &Self) -> Result<FocusProfile, ConfigError> {
        let dir = Self::app_config_dir()?;
        Self::create_profile_in(&dir, name, cfg)
    }

    /// Deletes a custom focus mode file in `profiles/` and resets active mode if needed.
    pub fn delete_profile(id: &str) -> Result<(), ConfigError> {
        let dir = Self::app_config_dir()?;
        Self::delete_profile_in(&dir, id)
    }

    fn load_or_create_default_in(dir: &Path) -> Result<(Self, PathBuf), ConfigError> {
        let path = dir.join(CONFIG_FILE_NAME);
        if !path.exists() {
            if let Err(err) = Self::write_default_template(&path) {
                warn!("could not write default config to {}: {err}", path.display());
            } else {
                info!("created default config at {}", path.display());
            }
            let mut cfg = Self::default();
            cfg.sanitize();
            return Ok((cfg, path));
        }

        let cfg = Self::load_from_path(&path)?;
        Ok((cfg, path))
    }

    fn list_profiles_in(dir: &Path) -> Result<Vec<FocusProfile>, ConfigError> {
        let default_path = dir.join(CONFIG_FILE_NAME);
        if !default_path.exists()
            && let Err(err) = Self::write_default_template(&default_path)
        {
            warn!(
                "could not write default config to {}: {err}",
                default_path.display()
            );
        }

        let profiles_dir = dir.join(PROFILES_DIR_NAME);
        if !profiles_dir.exists() {
            Self::seed_starter_profiles(&profiles_dir);
        }

        let mut profiles = vec![FocusProfile {
            id: DEFAULT_PROFILE_ID.to_string(),
            name: "Default".to_string(),
            path: default_path,
            is_default: true,
        }];

        let mut custom = Vec::new();
        if let Ok(entries) = fs::read_dir(&profiles_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if stem == DEFAULT_PROFILE_ID || stem.is_empty() {
                    continue;
                }
                custom.push(FocusProfile {
                    id: stem.to_string(),
                    name: profile_display_name(stem),
                    path,
                    is_default: false,
                });
            }
        }
        custom.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        profiles.extend(custom);
        Ok(profiles)
    }

    fn seed_starter_profiles(profiles_dir: &Path) {
        if let Err(err) = fs::create_dir_all(profiles_dir) {
            warn!(
                "could not create profiles directory {}: {err}",
                profiles_dir.display()
            );
            return;
        }

        let study = Config {
            short_break_interval_minutes: 15,
            short_break_duration_seconds: 15,
            ..Config::default()
        };
        let work = Config {
            short_break_interval_minutes: 20,
            short_break_duration_seconds: 20,
            ..Config::default()
        };

        for (slug, cfg) in [("study", study), ("work", work)] {
            let path = profiles_dir.join(format!("{slug}.toml"));
            if !path.exists()
                && let Err(err) = Self::write_config_file(&path, &cfg)
            {
                warn!(
                    "could not write starter profile {}: {err}",
                    path.display()
                );
            }
        }
        info!("created starter focus profiles in {}", profiles_dir.display());
    }

    fn load_active_profile_in(
        dir: &Path,
    ) -> Result<(FocusProfile, Config, Vec<FocusProfile>), ConfigError> {
        let profiles = Self::list_profiles_in(dir)?;
        let active_id = Self::read_active_profile_id_in(dir);

        let selected = profiles
            .iter()
            .find(|p| p.id == active_id)
            .or_else(|| profiles.first())
            .cloned()
            .ok_or_else(|| ConfigError::ProfileNotFound(active_id.clone()))?;

        if selected.id != active_id {
            let _ = Self::save_active_profile_id_in(dir, &selected.id);
        }

        let cfg = match Self::load_from_path(&selected.path) {
            Ok(cfg) => cfg,
            Err(err) if !selected.is_default => {
                warn!(
                    "failed to load focus mode '{}' ({}): {err}; falling back to Default",
                    selected.name,
                    selected.path.display()
                );
                let default_profile = profiles
                    .first()
                    .cloned()
                    .ok_or_else(|| ConfigError::ProfileNotFound(DEFAULT_PROFILE_ID.to_string()))?;
                let _ = Self::save_active_profile_id_in(dir, DEFAULT_PROFILE_ID);
                let default_cfg = Self::load_or_create_default_in(dir)?.0;
                return Ok((default_profile, default_cfg, profiles));
            }
            Err(err) => return Err(err),
        };

        Ok((selected, cfg, profiles))
    }

    fn read_active_profile_id_in(dir: &Path) -> String {
        let marker = dir.join(ACTIVE_PROFILE_FILE_NAME);
        fs::read_to_string(marker)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_PROFILE_ID.to_string())
    }

    fn save_active_profile_id_in(dir: &Path, id: &str) -> Result<(), ConfigError> {
        fs::create_dir_all(dir).map_err(|source| ConfigError::Write {
            path: dir.to_path_buf(),
            source,
        })?;
        let marker = dir.join(ACTIVE_PROFILE_FILE_NAME);
        fs::write(&marker, format!("{id}\n")).map_err(|source| ConfigError::Write {
            path: marker,
            source,
        })
    }

    fn create_profile_in(
        dir: &Path,
        name: &str,
        cfg: &Self,
    ) -> Result<FocusProfile, ConfigError> {
        let Some(slug) = sanitize_profile_slug(name) else {
            return Err(ConfigError::InvalidProfileName(name.to_string()));
        };
        if slug == DEFAULT_PROFILE_ID {
            return Err(ConfigError::InvalidProfileName(name.to_string()));
        }

        let profiles_dir = dir.join(PROFILES_DIR_NAME);
        if !profiles_dir.exists() {
            Self::seed_starter_profiles(&profiles_dir);
        }
        fs::create_dir_all(&profiles_dir).map_err(|source| ConfigError::Write {
            path: profiles_dir.clone(),
            source,
        })?;

        let path = profiles_dir.join(format!("{slug}.toml"));
        if path.exists() {
            return Err(ConfigError::ProfileAlreadyExists(
                profile_display_name(&slug),
            ));
        }

        cfg.save_to_path(&path)?;
        Self::save_active_profile_id_in(dir, &slug)?;
        Ok(FocusProfile {
            name: profile_display_name(&slug),
            id: slug,
            path,
            is_default: false,
        })
    }

    fn delete_profile_in(dir: &Path, id: &str) -> Result<(), ConfigError> {
        if id == DEFAULT_PROFILE_ID {
            return Err(ConfigError::CannotDeleteDefault);
        }
        let path = dir.join(PROFILES_DIR_NAME).join(format!("{id}.toml"));
        if !path.exists() {
            return Err(ConfigError::ProfileNotFound(id.to_string()));
        }
        fs::remove_file(&path).map_err(|source| ConfigError::Write {
            path: path.clone(),
            source,
        })?;

        if Self::read_active_profile_id_in(dir) == id {
            let _ = Self::save_active_profile_id_in(dir, DEFAULT_PROFILE_ID);
        }
        Ok(())
    }

    /// Loads and sanitizes config from an explicit path.
    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut cfg: Self = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        cfg.sanitize();
        Ok(cfg)
    }

    /// Clamps invalid zero/out-of-range values to safe operational bounds.
    pub fn sanitize(&mut self) {
        if self.short_break_interval_minutes == 0 {
            self.short_break_interval_minutes = 1;
        }
        if self.short_break_duration_seconds == 0 {
            self.short_break_duration_seconds = 1;
        }
        if self.long_break_duration_seconds == 0 {
            self.long_break_duration_seconds = 1;
        }
        if self.snooze_duration_minutes == 0 {
            self.snooze_duration_minutes = 1;
        }
        let interval_secs = self.interval_seconds();
        if self.pre_break_warning_seconds >= interval_secs {
            self.pre_break_warning_seconds = interval_secs.saturating_sub(1);
        }
        if self.idle_pause_seconds > 0 && self.idle_reset_seconds < self.idle_pause_seconds {
            self.idle_reset_seconds = self.idle_pause_seconds;
        }
        if self.short_break_exercises.is_empty() {
            self.short_break_exercises = Self::default().short_break_exercises;
        }
        if self.long_break_exercises.is_empty() {
            self.long_break_exercises = Self::default().long_break_exercises;
        }
    }

    pub fn interval_seconds(&self) -> u64 {
        self.short_break_interval_minutes.saturating_mul(60)
    }

    /// Saves this configuration to `path` after sanitizing values.
    pub fn save_to_path(&self, path: &Path) -> Result<(), ConfigError> {
        let mut sanitized = self.clone();
        sanitized.sanitize();
        Self::write_config_file(path, &sanitized)
    }

    /// Persists an updated `timer_mode` into an existing config file while preserving comments.
    pub fn persist_timer_mode(path: &Path, mode: TimerMode) -> Result<(), ConfigError> {
        let mode_str = match mode {
            TimerMode::SmartPause => "smart_pause",
            TimerMode::Strict => "strict",
        };
        let new_line = format!("timer_mode = \"{mode_str}\"");

        let existing = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut cfg = Self::default();
                cfg.timer_mode = mode;
                Self::write_config_file(path, &cfg)?;
                return Ok(());
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        let mut replaced = false;
        let mut out_lines: Vec<String> = existing
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if !trimmed.starts_with('#')
                    && trimmed.starts_with("timer_mode")
                    && trimmed["timer_mode".len()..].trim_start().starts_with('=')
                {
                    replaced = true;
                    new_line.clone()
                } else {
                    line.to_string()
                }
            })
            .collect();

        if !replaced {
            let insert_at = out_lines
                .iter()
                .position(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .unwrap_or(out_lines.len());
            out_lines.insert(insert_at, new_line);
        }

        let mut updated = out_lines.join("\n");
        if existing.ends_with('\n') || !updated.ends_with('\n') {
            updated.push('\n');
        }

        fs::write(path, updated).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    fn write_default_template(path: &Path) -> Result<(), ConfigError> {
        Self::write_config_file(path, &Self::default())
    }

    fn write_config_file(path: &Path, cfg: &Self) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let serialized = toml::to_string_pretty(cfg).unwrap_or_default();
        let content = format!(
            "# SafeEyes-RS configuration\n\
             # Reload from the system tray menu after editing.\n\n\
             {serialized}"
        );
        fs::write(path, content).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Converts a human-readable focus mode name into a safe filename stem (`deep-work`).
pub fn sanitize_profile_slug(name: &str) -> Option<String> {
    let mut slug = String::with_capacity(name.len());
    let mut prev_dash = true;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() { None } else { Some(slug) }
}

/// Formats a profile slug (`deep-work`) into a display label (`Deep Work`).
pub fn profile_display_name(slug: &str) -> String {
    if slug == DEFAULT_PROFILE_ID {
        return "Default".to_string();
    }
    slug.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().to_string();
                    word.push_str(chars.as_str());
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "safeeyes-rs-test-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn sanitize_clamps_invalid_values() {
        let mut cfg = Config {
            short_break_interval_minutes: 0,
            short_break_duration_seconds: 0,
            long_break_duration_seconds: 0,
            pre_break_warning_seconds: 999,
            idle_pause_seconds: 120,
            idle_reset_seconds: 30,
            short_break_exercises: vec![],
            long_break_exercises: vec![],
            ..Default::default()
        };
        cfg.sanitize();
        assert_eq!(cfg.short_break_interval_minutes, 1);
        assert_eq!(cfg.short_break_duration_seconds, 1);
        assert_eq!(cfg.long_break_duration_seconds, 1);
        assert_eq!(cfg.pre_break_warning_seconds, 59);
        assert_eq!(cfg.idle_reset_seconds, 120);
        assert!(!cfg.short_break_exercises.is_empty());
        assert!(!cfg.long_break_exercises.is_empty());
    }

    #[test]
    fn default_toml_roundtrip() {
        let cfg = Config::default();
        let encoded = toml::to_string_pretty(&cfg).expect("serialize default config");
        let decoded: Config = toml::from_str(&encoded).expect("deserialize default config");
        assert_eq!(cfg, decoded);
    }

    #[test]
    fn profile_slug_and_display_name() {
        assert_eq!(sanitize_profile_slug("Deep Work!"), Some("deep-work".to_string()));
        assert_eq!(sanitize_profile_slug("  Deep  Work _ 2 "), Some("deep-work-2".to_string()));
        assert_eq!(sanitize_profile_slug("   ---  "), None);
        assert_eq!(profile_display_name("default"), "Default");
        assert_eq!(profile_display_name("deep-work"), "Deep Work");
        assert_eq!(profile_display_name("study_mode"), "Study Mode");
    }

    #[test]
    fn focus_profiles_lifecycle() {
        let dir = unique_temp_dir("profiles");

        // First call seeds starter profiles (Study & Work) alongside Default.
        let (active, cfg, profiles) =
            Config::load_active_profile_in(&dir).expect("load active profile");
        assert_eq!(active.id, DEFAULT_PROFILE_ID);
        assert_eq!(cfg.short_break_interval_minutes, 15);
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles[0].name, "Default");
        assert_eq!(profiles[1].name, "Study");
        assert_eq!(profiles[2].name, "Work");

        // Deleting a starter profile should NOT cause it to be re-seeded on next list.
        Config::delete_profile_in(&dir, "study").expect("delete study");
        let after_delete = Config::list_profiles_in(&dir).expect("list after delete");
        assert_eq!(after_delete.len(), 2);
        assert!(after_delete.iter().all(|p| p.id != "study"));

        // Creating a custom profile makes it active and persists its config.
        let custom_cfg = Config {
            short_break_interval_minutes: 45,
            short_break_duration_seconds: 30,
            ..Config::default()
        };
        let created =
            Config::create_profile_in(&dir, "Deep Focus", &custom_cfg).expect("create custom");
        assert_eq!(created.id, "deep-focus");
        assert_eq!(created.name, "Deep Focus");

        let (reloaded, reloaded_cfg, _) =
            Config::load_active_profile_in(&dir).expect("reload custom active");
        assert_eq!(reloaded.id, "deep-focus");
        assert_eq!(reloaded_cfg.short_break_interval_minutes, 45);

        // Cannot delete default; deleting active custom profile falls back to Default.
        assert!(matches!(
            Config::delete_profile_in(&dir, DEFAULT_PROFILE_ID),
            Err(ConfigError::CannotDeleteDefault)
        ));
        Config::delete_profile_in(&dir, "deep-focus").expect("delete custom");
        let (fallback, _, _) =
            Config::load_active_profile_in(&dir).expect("fallback to default");
        assert_eq!(fallback.id, DEFAULT_PROFILE_ID);

        let _ = fs::remove_dir_all(&dir);
    }
}
