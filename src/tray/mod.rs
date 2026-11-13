use std::sync::LazyLock;

use async_channel::Sender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::{Icon, MenuItem, Status, ToolTip, Tray};
use tracing::{error, info, warn};

use crate::core::config::TimerMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleEnabled,
    DisableForMinutes(u64),
    DisableIndefinitely,
    SetTimerMode(TimerMode),
    TakeShortBreakNow,
    TakeLongBreakNow,
    OpenPreferences,
    ReloadConfig,
    OpenConfig,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraySnapshot {
    pub enabled: bool,
    pub in_break: bool,
    pub status_line: String,
    pub short_break_seconds: u64,
    pub long_break_seconds: u64,
    pub timer_mode: TimerMode,
    pub idle_reset_seconds: u64,
}

pub struct SafeEyesTray {
    tx: Sender<TrayCommand>,
    snapshot: TraySnapshot,
}

impl SafeEyesTray {
    pub fn spawn(tx: Sender<TrayCommand>, initial: TraySnapshot) -> Option<Handle<Self>> {
        let tray = Self {
            tx,
            snapshot: initial,
        };
        let is_sandboxed = std::path::Path::new("/.flatpak-info").exists()
            || std::env::var_os("FLATPAK_ID").is_some();
        match tray
            .disable_dbus_name(is_sandboxed)
            .assume_sni_available(true)
            .spawn()
        {
            Ok(handle) => {
                info!("system tray (StatusNotifierItem) initialized");
                Some(handle)
            }
            Err(err) => {
                error!("failed to initialize system tray: {err}");
                None
            }
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: TraySnapshot) {
        self.snapshot = snapshot;
    }

    fn send(&self, cmd: TrayCommand) {
        if let Err(err) = self.tx.send_blocking(cmd) {
            warn!("failed to send tray command {cmd:?}: {err}");
        }
    }
}

impl Tray for SafeEyesTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "safeeyes-rs".to_string()
    }

    fn title(&self) -> String {
        "SafeEyes".to_string()
    }

    fn status(&self) -> Status {
        // Always stay Active so KDE/GNOME panels keep the disabled (red-slashed)
        // icon visible rather than hiding it under Status::Passive.
        Status::Active
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        if self.snapshot.enabled {
            vec![ICON_ACTIVE.clone()]
        } else {
            vec![ICON_DISABLED.clone()]
        }
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: "SafeEyes".to_string(),
            description: self.snapshot.status_line.clone(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};

        let toggle_label = if self.snapshot.enabled {
            "Enabled"
        } else {
            "Enable SafeEyes"
        };

        let idle_label = if self.snapshot.idle_reset_seconds == 0 {
            "Smart Pause (pause on idle)".to_string()
        } else if self.snapshot.idle_reset_seconds >= 60
            && self.snapshot.idle_reset_seconds.is_multiple_of(60)
        {
            format!(
                "Smart Pause (reset after {}m idle)",
                self.snapshot.idle_reset_seconds / 60
            )
        } else {
            format!(
                "Smart Pause (reset after {}s idle)",
                self.snapshot.idle_reset_seconds
            )
        };

        let selected_mode = match self.snapshot.timer_mode {
            TimerMode::SmartPause => 0,
            TimerMode::Strict => 1,
        };

        let mut disable_submenu: Vec<MenuItem<Self>> = [
            ("15 minutes", 15_u64),
            ("30 minutes", 30),
            ("1 hour", 60),
            ("2 hours", 120),
            ("4 hours", 240),
        ]
        .into_iter()
        .map(|(label, mins)| {
            StandardItem {
                label: label.to_string(),
                activate: Box::new(move |this: &mut Self| {
                    this.send(TrayCommand::DisableForMinutes(mins))
                }),
                ..Default::default()
            }
            .into()
        })
        .collect();
        disable_submenu.push(MenuItem::Separator);
        disable_submenu.push(
            StandardItem {
                label: "Until manually enabled".to_string(),
                enabled: self.snapshot.enabled,
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::DisableIndefinitely)),
                ..Default::default()
            }
            .into(),
        );

        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: self.snapshot.status_line.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: toggle_label.to_string(),
                checked: self.snapshot.enabled,
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::ToggleEnabled)),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Disable for...".to_string(),
                submenu: disable_submenu,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: format!("Take a {}s break now", self.snapshot.short_break_seconds),
                icon_name: "media-playback-pause".to_string(),
                enabled: !self.snapshot.in_break,
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::TakeShortBreakNow)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: format!("Take a {}s break now", self.snapshot.long_break_seconds),
                icon_name: "media-playback-pause".to_string(),
                enabled: !self.snapshot.in_break,
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::TakeLongBreakNow)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            SubMenu {
                label: "Timer mode".to_string(),
                icon_name: "preferences-system-time".to_string(),
                submenu: vec![
                    RadioGroup {
                        selected: selected_mode,
                        select: Box::new(|this: &mut Self, current| {
                            let mode = if current == 1 {
                                TimerMode::Strict
                            } else {
                                TimerMode::SmartPause
                            };
                            this.send(TrayCommand::SetTimerMode(mode));
                        }),
                        options: vec![
                            RadioItem {
                                label: idle_label,
                                ..Default::default()
                            },
                            RadioItem {
                                label: "Strict (fixed interval)".to_string(),
                                ..Default::default()
                            },
                        ],
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Settings".to_string(),
                icon_name: "preferences-system".to_string(),
                submenu: vec![
                    StandardItem {
                        label: "Preferences...".to_string(),
                        icon_name: "preferences-other".to_string(),
                        activate: Box::new(|this: &mut Self| {
                            this.send(TrayCommand::OpenPreferences)
                        }),
                        ..Default::default()
                    }
                    .into(),
                    MenuItem::Separator,
                    StandardItem {
                        label: "Open config file".to_string(),
                        icon_name: "document-edit".to_string(),
                        activate: Box::new(|this: &mut Self| this.send(TrayCommand::OpenConfig)),
                        ..Default::default()
                    }
                    .into(),
                    StandardItem {
                        label: "Reload config".to_string(),
                        icon_name: "view-refresh".to_string(),
                        activate: Box::new(|this: &mut Self| this.send(TrayCommand::ReloadConfig)),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".to_string(),
                icon_name: "application-exit".to_string(),
                activate: Box::new(|this: &mut Self| this.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ];

        // Keep menu compact and responsive.
        items.shrink_to_fit();
        items
    }
}

static ICON_ACTIVE: LazyLock<Icon> = LazyLock::new(|| render_eye_icon(true));
static ICON_DISABLED: LazyLock<Icon> = LazyLock::new(|| render_eye_icon(false));

/// Renders a crisp 32x32 antialiased eye icon in ARGB32 network byte order.
fn render_eye_icon(active: bool) -> Icon {
    const SIZE: usize = 32;
    let mut data = vec![0u8; SIZE * SIZE * 4];
    let cx = 15.5_f32;
    let cy = 15.5_f32;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = x as f32 - cx;
            let fy = y as f32 - cy;

            // Almond-shaped eye outline from two intersecting circles.
            let nx = fx / 13.5;
            let ny = fy / 7.8;
            let eye_dist = nx * nx + ny * ny;

            let r2 = fx * fx + fy * fy;
            let dist = r2.sqrt();

            let mut argb: [u8; 4] = [0, 0, 0, 0];

            // Soft almond sclera / ring
            if eye_dist <= 1.0 {
                let edge_alpha = ((1.0 - eye_dist) * 4.5).clamp(0.0, 1.0);
                if eye_dist > 0.62 {
                    // Outer eye contour
                    let a = (edge_alpha * if active { 240.0 } else { 145.0 }) as u8;
                    argb = if active {
                        [a, 235, 242, 255]
                    } else {
                        [a, 150, 158, 170]
                    };
                } else if dist <= 5.5 {
                    // Iris + pupil
                    let iris_alpha = ((5.5 - dist) * 1.5).clamp(0.0, 1.0);
                    if dist <= 2.3 {
                        // Pupil
                        let a = if active { 245 } else { 150 };
                        argb = [a, 24, 30, 42];
                    } else if active {
                        let a = (iris_alpha * 245.0) as u8;
                        argb = [a, 76, 175, 245];
                    } else {
                        let a = (iris_alpha * 150.0) as u8;
                        argb = [a, 120, 128, 140];
                    }
                } else {
                    // Sclera interior
                    let a = if active { 215 } else { 110 };
                    argb = [a, 235, 242, 255];
                }
            }

            // Diagonal slash when disabled.
            if !active {
                let diag = (fx - fy).abs();
                if diag <= 1.35 && dist <= 13.0 {
                    let a = ((1.35 - diag) * 240.0).clamp(0.0, 240.0) as u8;
                    argb = [a, 220, 95, 95];
                }
            }

            let idx = (y * SIZE + x) * 4;
            data[idx..idx + 4].copy_from_slice(&argb);
        }
    }

    Icon {
        width: SIZE as i32,
        height: SIZE as i32,
        data,
    }
}
