use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

const APP_DIR_NAME: &str = "safeeyes-rs";
const CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine XDG config directory")]
    MissingConfigDir,
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
    /// Returns `$XDG_CONFIG_HOME/safeeyes-rs/config.toml` or `~/.config/safeeyes-rs/config.toml`.
    pub fn config_path() -> Result<PathBuf, ConfigError> {
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

        Ok(base.join(APP_DIR_NAME).join(CONFIG_FILE_NAME))
    }

    /// Loads config from disk, creating a documented default file if missing.
    pub fn load_or_create_default() -> Result<(Self, PathBuf), ConfigError> {
        let path = Self::config_path()?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
