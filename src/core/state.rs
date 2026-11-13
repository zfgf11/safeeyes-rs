use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::core::config::{Config, TimerMode};
use crate::core::scheduler::{BreakKind, BreakPlan, BreakScheduler};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    UserIdle,
    ScreenLocked,
    Suspended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorePhase {
    /// Waiting until pre-break warning or break start.
    Waiting {
        next_break_at: Instant,
        next_break_wall: DateTime<Local>,
        next_kind: BreakKind,
        pre_break_notified: bool,
    },
    /// Break overlay is currently active.
    InBreak {
        plan: BreakPlan,
        remaining_seconds: u64,
    },
    /// Temporarily paused due to idle, lockscreen, or system sleep.
    Paused {
        reason: PauseReason,
        paused_at: Instant,
        remaining_until_break: Duration,
    },
    /// Manually disabled by the user (indefinitely or until `reenable_at`).
    Disabled {
        reenable_at: Option<Instant>,
        reenable_wall: Option<DateTime<Local>>,
    },
}

/// Commands emitted by the pure state machine for the runtime coordinator to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    ScheduleNextWakeup { delay_seconds: u64 },
    ScheduleReenable { delay_seconds: u64 },
    CancelTimers,
    ShowPreBreakNotification { kind: BreakKind, warning_seconds: u64 },
    OpenBreakOverlay {
        plan: BreakPlan,
        remaining_seconds: u64,
        skips_used: u32,
        max_skips: u32,
    },
    UpdateBreakCountdown { remaining_seconds: u64 },
    CloseBreakOverlay,
    PlayBreakStartChime,
    PlayBreakEndChime,
    RefreshTray,
}

#[derive(Debug)]
pub struct CoreState {
    config: Config,
    scheduler: BreakScheduler,
    phase: CorePhase,
    skips_used: u32,
    is_locked_or_sleeping: bool,
    is_user_idle: bool,
}

impl CoreState {
    pub fn new(config: Config, now: Instant) -> (Self, Vec<CoreEffect>) {
        let scheduler = BreakScheduler::new();
        let next_kind = scheduler.next_break_kind(&config);
        let interval = Duration::from_secs(config.interval_seconds());
        let next_break_at = now + interval;
        let next_break_wall = wall_after(interval);

        let state = Self {
            config,
            scheduler,
            phase: CorePhase::Waiting {
                next_break_at,
                next_break_wall,
                next_kind,
                pre_break_notified: false,
            },
            skips_used: 0,
            is_locked_or_sleeping: false,
            is_user_idle: false,
        };

        let delay = state.compute_wait_delay(now);
        let effects = vec![
            CoreEffect::ScheduleNextWakeup {
                delay_seconds: delay,
            },
            CoreEffect::RefreshTray,
        ];
        (state, effects)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    #[cfg(test)]
    pub fn skips_used(&self) -> u32 {
        self.skips_used
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self.phase, CorePhase::Disabled { .. })
    }

    pub fn is_in_break(&self) -> bool {
        matches!(self.phase, CorePhase::InBreak { .. })
    }

    pub fn can_skip(&self) -> bool {
        self.skips_used < self.config.max_skips
    }

    pub fn tray_status_line(&self) -> String {
        match &self.phase {
            CorePhase::Waiting {
                next_break_wall,
                next_kind,
                ..
            } => {
                let rounded = *next_break_wall + chrono::Duration::seconds(30);
                format!(
                    "Next {} at {}",
                    next_kind.label().to_lowercase(),
                    rounded.format("%H:%M")
                )
            }
            CorePhase::InBreak {
                plan,
                remaining_seconds,
            } => format!("{} in progress ({remaining_seconds}s)", plan.kind.label()),
            CorePhase::Paused { reason, .. } => match reason {
                PauseReason::UserIdle => "Paused (user idle)".to_string(),
                PauseReason::ScreenLocked => "Paused (screen locked)".to_string(),
                PauseReason::Suspended => "Paused (system suspended)".to_string(),
            },
            CorePhase::Disabled { reenable_wall, .. } => match reenable_wall {
                Some(wall) => {
                    let rounded = *wall + chrono::Duration::seconds(30);
                    format!("Disabled until {}", rounded.format("%H:%M"))
                }
                None => "Disabled".to_string(),
            },
        }
    }

    /// Handles the single-shot wait / pre-break timer firing.
    pub fn on_wait_timer(&mut self, now: Instant) -> Vec<CoreEffect> {
        let CorePhase::Waiting {
            next_break_at,
            next_kind,
            pre_break_notified,
            ..
        } = self.phase.clone()
        else {
            return Vec::new();
        };

        let remaining_dur = next_break_at.saturating_duration_since(now);
        // Treat sub-500ms remainder as reached so coarse wakeups never stall at 0s.
        let remaining = if remaining_dur.as_millis() <= 500 {
            0
        } else {
            (remaining_dur.as_millis() as u64).div_ceil(1000)
        };
        let warn_secs = self.config.pre_break_warning_seconds;

        if !pre_break_notified && warn_secs > 0 && remaining <= warn_secs && remaining > 0 {
            if let CorePhase::Waiting {
                pre_break_notified, ..
            } = &mut self.phase
            {
                *pre_break_notified = true;
            }
            return vec![
                CoreEffect::ShowPreBreakNotification {
                    kind: next_kind,
                    warning_seconds: remaining,
                },
                CoreEffect::ScheduleNextWakeup {
                    delay_seconds: remaining.max(1),
                },
            ];
        }

        if remaining > 0 {
            // Woke up slightly early; reschedule exact remaining wait.
            let delay = self.compute_wait_delay(now);
            return vec![CoreEffect::ScheduleNextWakeup {
                delay_seconds: delay,
            }];
        }

        let plan = self.scheduler.take_next_break(&self.config);
        self.enter_break(plan)
    }

    /// Handles the 1-second countdown tick while in a break.
    pub fn on_break_tick(&mut self, now: Instant) -> Vec<CoreEffect> {
        let CorePhase::InBreak {
            remaining_seconds, ..
        } = &mut self.phase
        else {
            return Vec::new();
        };

        if *remaining_seconds > 1 {
            *remaining_seconds -= 1;
            let secs = *remaining_seconds;
            return vec![CoreEffect::UpdateBreakCountdown {
                remaining_seconds: secs,
            }];
        }

        // Break completed naturally: reset skip counter.
        self.skips_used = 0;
        let mut effects = vec![
            CoreEffect::CloseBreakOverlay,
            CoreEffect::PlayBreakEndChime,
        ];
        effects.extend(self.reset_to_waiting(now));
        effects
    }

    /// Triggers a break of the requested kind immediately from the tray menu.
    pub fn take_break_now(&mut self, kind: BreakKind) -> Vec<CoreEffect> {
        if self.is_in_break() {
            return Vec::new();
        }
        let plan = self.scheduler.take_manual_break(&self.config, kind);
        self.enter_break(plan)
    }

    /// Skips the current break if skips remain.
    pub fn skip_break(&mut self, now: Instant) -> Vec<CoreEffect> {
        if !self.is_in_break() || !self.can_skip() {
            return Vec::new();
        }
        self.skips_used = self.skips_used.saturating_add(1);
        let mut effects = vec![CoreEffect::CloseBreakOverlay];
        effects.extend(self.reset_to_waiting(now));
        effects
    }

    /// Enables SafeEyes (from disabled state or re-enables immediately).
    pub fn enable(&mut self, now: Instant) -> Vec<CoreEffect> {
        if self.is_enabled() {
            return Vec::new();
        }
        if self.is_locked_or_sleeping {
            self.phase = CorePhase::Paused {
                reason: PauseReason::ScreenLocked,
                paused_at: now,
                remaining_until_break: Duration::from_secs(self.config.interval_seconds()),
            };
            return vec![CoreEffect::CancelTimers, CoreEffect::RefreshTray];
        }
        if self.config.timer_mode == TimerMode::SmartPause && self.is_user_idle {
            self.phase = CorePhase::Paused {
                reason: PauseReason::UserIdle,
                paused_at: now,
                remaining_until_break: Duration::from_secs(self.config.interval_seconds()),
            };
            return vec![CoreEffect::CancelTimers, CoreEffect::RefreshTray];
        }
        self.reset_to_waiting(now)
    }

    /// Switches between Smart Pause (idle-aware) and Strict (fixed interval) timer modes.
    pub fn set_timer_mode(&mut self, mode: TimerMode, now: Instant) -> Vec<CoreEffect> {
        if self.config.timer_mode == mode {
            return Vec::new();
        }
        self.config.timer_mode = mode;

        if mode == TimerMode::Strict {
            self.is_user_idle = false;
            if let CorePhase::Paused {
                reason: PauseReason::UserIdle,
                remaining_until_break,
                ..
            } = self.phase
            {
                return self.resume_waiting_with_remaining(now, remaining_until_break);
            }
        }
        vec![CoreEffect::RefreshTray]
    }

    /// Disables SafeEyes indefinitely or for a timed duration.
    pub fn disable(&mut self, duration: Option<Duration>) -> Vec<CoreEffect> {
        let was_in_break = self.is_in_break();
        let (reenable_at, reenable_wall) = match duration {
            Some(dur) => (Some(Instant::now() + dur), Some(wall_after(dur))),
            None => (None, None),
        };
        self.phase = CorePhase::Disabled {
            reenable_at,
            reenable_wall,
        };
        let mut effects = vec![CoreEffect::CancelTimers];
        if was_in_break {
            effects.push(CoreEffect::CloseBreakOverlay);
        }
        if let Some(dur) = duration {
            effects.push(CoreEffect::ScheduleReenable {
                delay_seconds: dur.as_secs().max(1),
            });
        }
        effects.push(CoreEffect::RefreshTray);
        effects
    }

    /// Updates configuration (e.g. after Reload Config) without losing active state unnecessarily.
    pub fn update_config(&mut self, new_config: Config, now: Instant) -> Vec<CoreEffect> {
        self.config = new_config;
        if self.config.timer_mode == TimerMode::Strict {
            self.is_user_idle = false;
        }
        match &self.phase {
            CorePhase::Waiting { .. } => self.reset_to_waiting(now),
            CorePhase::Paused {
                reason: PauseReason::UserIdle,
                ..
            } if self.config.timer_mode == TimerMode::Strict => self.reset_to_waiting(now),
            CorePhase::InBreak { .. } | CorePhase::Paused { .. } | CorePhase::Disabled { .. } => {
                vec![CoreEffect::RefreshTray]
            }
        }
    }

    pub fn on_screen_locked(&mut self, now: Instant, reason: PauseReason) -> Vec<CoreEffect> {
        self.is_locked_or_sleeping = true;
        if !self.is_enabled() {
            return Vec::new();
        }
        let was_in_break = self.is_in_break();
        self.phase = CorePhase::Paused {
            reason,
            paused_at: now,
            remaining_until_break: Duration::from_secs(self.config.interval_seconds()),
        };
        let mut effects = vec![CoreEffect::CancelTimers];
        if was_in_break {
            effects.push(CoreEffect::CloseBreakOverlay);
        }
        effects.push(CoreEffect::RefreshTray);
        effects
    }

    pub fn on_screen_unlocked(&mut self, now: Instant) -> Vec<CoreEffect> {
        self.is_locked_or_sleeping = false;
        if !self.is_enabled() {
            return Vec::new();
        }
        if self.config.timer_mode == TimerMode::SmartPause && self.is_user_idle {
            self.phase = CorePhase::Paused {
                reason: PauseReason::UserIdle,
                paused_at: now,
                remaining_until_break: Duration::from_secs(self.config.interval_seconds()),
            };
            return vec![CoreEffect::RefreshTray];
        }
        self.skips_used = 0;
        self.reset_to_waiting(now)
    }

    pub fn on_user_idled(&mut self, now: Instant) -> Vec<CoreEffect> {
        if self.config.timer_mode == TimerMode::Strict {
            return Vec::new();
        }
        self.is_user_idle = true;
        if self.config.idle_pause_seconds == 0 || !self.is_enabled() || self.is_locked_or_sleeping {
            return Vec::new();
        }
        // Do not pause an already visible break overlay because user isn't touching input during a break.
        let CorePhase::Waiting { next_break_at, .. } = &self.phase else {
            return Vec::new();
        };
        let remaining = next_break_at.saturating_duration_since(now);
        self.phase = CorePhase::Paused {
            reason: PauseReason::UserIdle,
            paused_at: now,
            remaining_until_break: remaining,
        };
        vec![CoreEffect::CancelTimers, CoreEffect::RefreshTray]
    }

    pub fn on_user_resumed(&mut self, now: Instant) -> Vec<CoreEffect> {
        if self.config.timer_mode == TimerMode::Strict {
            return Vec::new();
        }
        self.is_user_idle = false;
        if !self.is_enabled() || self.is_locked_or_sleeping {
            return Vec::new();
        }
        let CorePhase::Paused {
            reason: PauseReason::UserIdle,
            paused_at,
            remaining_until_break,
        } = self.phase
        else {
            return Vec::new();
        };

        let idle_elapsed = now.saturating_duration_since(paused_at);
        let reset_threshold = Duration::from_secs(
            self.config
                .idle_reset_seconds
                .saturating_sub(self.config.idle_pause_seconds),
        );

        if idle_elapsed >= reset_threshold {
            self.skips_used = 0;
            self.reset_to_waiting(now)
        } else {
            self.resume_waiting_with_remaining(now, remaining_until_break)
        }
    }

    fn resume_waiting_with_remaining(
        &mut self,
        now: Instant,
        remaining_until_break: Duration,
    ) -> Vec<CoreEffect> {
        let dur = remaining_until_break.max(Duration::from_secs(1));
        let next_break_at = now + dur;
        let next_break_wall = wall_after(dur);
        let next_kind = self.scheduler.next_break_kind(&self.config);
        self.phase = CorePhase::Waiting {
            next_break_at,
            next_break_wall,
            next_kind,
            pre_break_notified: false,
        };
        let delay = self.compute_wait_delay(now);
        vec![
            CoreEffect::ScheduleNextWakeup {
                delay_seconds: delay,
            },
            CoreEffect::RefreshTray,
        ]
    }

    fn enter_break(&mut self, plan: BreakPlan) -> Vec<CoreEffect> {
        let duration = plan.duration_seconds;
        self.phase = CorePhase::InBreak {
            plan: plan.clone(),
            remaining_seconds: duration,
        };
        vec![
            CoreEffect::CancelTimers,
            CoreEffect::OpenBreakOverlay {
                plan,
                remaining_seconds: duration,
                skips_used: self.skips_used,
                max_skips: self.config.max_skips,
            },
            CoreEffect::PlayBreakStartChime,
            CoreEffect::RefreshTray,
        ]
    }

    fn reset_to_waiting(&mut self, now: Instant) -> Vec<CoreEffect> {
        let interval = Duration::from_secs(self.config.interval_seconds());
        let next_break_at = now + interval;
        let next_break_wall = wall_after(interval);
        let next_kind = self.scheduler.next_break_kind(&self.config);
        self.phase = CorePhase::Waiting {
            next_break_at,
            next_break_wall,
            next_kind,
            pre_break_notified: false,
        };
        let delay = self.compute_wait_delay(now);
        vec![
            CoreEffect::CancelTimers,
            CoreEffect::ScheduleNextWakeup {
                delay_seconds: delay,
            },
            CoreEffect::RefreshTray,
        ]
    }

    fn compute_wait_delay(&self, now: Instant) -> u64 {
        let CorePhase::Waiting {
            next_break_at,
            pre_break_notified,
            ..
        } = self.phase
        else {
            return 1;
        };
        let remaining = next_break_at.saturating_duration_since(now).as_secs();
        let warn = self.config.pre_break_warning_seconds;
        if !pre_break_notified && warn > 0 && remaining > warn {
            (remaining - warn).max(1)
        } else {
            remaining.max(1)
        }
    }
}

fn wall_after(duration: Duration) -> DateTime<Local> {
    let chrono_dur = chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::zero());
    Local::now() + chrono_dur
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_to_prebreak_to_break_to_complete() {
        let cfg = Config {
            short_break_interval_minutes: 1, // 60s
            short_break_duration_seconds: 3,
            pre_break_warning_seconds: 10,
            max_skips: 2,
            ..Default::default()
        };
        let t0 = Instant::now();
        let (mut state, init_effects) = CoreState::new(cfg, t0);
        assert!(init_effects.contains(&CoreEffect::ScheduleNextWakeup { delay_seconds: 50 }));

        // At t0 + 50s -> pre-break notification + 10s wakeup
        let t1 = t0 + Duration::from_secs(50);
        let eff1 = state.on_wait_timer(t1);
        assert!(eff1.contains(&CoreEffect::ShowPreBreakNotification {
            kind: BreakKind::Short,
            warning_seconds: 10,
        }));
        assert!(eff1.contains(&CoreEffect::ScheduleNextWakeup { delay_seconds: 10 }));

        // At t0 + 60s -> enter break
        let t2 = t0 + Duration::from_secs(60);
        let eff2 = state.on_wait_timer(t2);
        assert!(eff2.iter().any(|e| matches!(
            e,
            CoreEffect::OpenBreakOverlay {
                remaining_seconds: 3,
                skips_used: 0,
                max_skips: 2,
                ..
            }
        )));
        assert!(state.is_in_break());

        // Countdown ticks: 3 -> 2 -> 1 -> done
        assert_eq!(
            state.on_break_tick(t2),
            vec![CoreEffect::UpdateBreakCountdown {
                remaining_seconds: 2
            }]
        );
        assert_eq!(
            state.on_break_tick(t2),
            vec![CoreEffect::UpdateBreakCountdown {
                remaining_seconds: 1
            }]
        );
        let done_eff = state.on_break_tick(t2);
        assert!(done_eff.contains(&CoreEffect::CloseBreakOverlay));
        assert!(done_eff.contains(&CoreEffect::PlayBreakEndChime));
        assert!(!state.is_in_break());
    }

    #[test]
    fn skip_respects_max_skips() {
        let cfg = Config {
            max_skips: 1,
            ..Default::default()
        };
        let t0 = Instant::now();
        let (mut state, _) = CoreState::new(cfg, t0);

        state.take_break_now(BreakKind::Short);
        assert!(state.is_in_break());
        let skip1 = state.skip_break(t0);
        assert!(skip1.contains(&CoreEffect::CloseBreakOverlay));
        assert_eq!(state.skips_used(), 1);
        assert!(!state.can_skip());

        // Second break cannot be skipped
        state.take_break_now(BreakKind::Short);
        let skip2 = state.skip_break(t0);
        assert!(skip2.is_empty());
        assert!(state.is_in_break());
    }

    #[test]
    fn manual_long_break_uses_long_duration_and_wait_boundary_triggers() {
        let cfg = Config {
            short_break_interval_minutes: 1,
            short_break_duration_seconds: 15,
            long_break_duration_seconds: 60,
            pre_break_warning_seconds: 10,
            ..Default::default()
        };
        let t0 = Instant::now();
        let (mut state, _) = CoreState::new(cfg, t0);

        let long_eff = state.take_break_now(BreakKind::Long);
        assert!(long_eff.iter().any(|e| matches!(
            e,
            CoreEffect::OpenBreakOverlay {
                plan,
                remaining_seconds: 60,
                ..
            } if plan.kind == BreakKind::Long
        )));

        // Skip to reset to waiting, then fire 300ms before target (coarse timer jitter).
        state.skip_break(t0);
        let t_pre = t0 + Duration::from_secs(50);
        let eff_pre = state.on_wait_timer(t_pre);
        assert!(eff_pre.iter().any(|e| matches!(
            e,
            CoreEffect::ShowPreBreakNotification { .. }
        )));

        let t_early = t0 + Duration::from_millis(59_700);
        let eff_break = state.on_wait_timer(t_early);
        assert!(eff_break.iter().any(|e| matches!(
            e,
            CoreEffect::OpenBreakOverlay {
                remaining_seconds: 15,
                ..
            }
        )));
    }

    #[test]
    fn idle_short_resumes_and_long_resets() {
        let cfg = Config {
            short_break_interval_minutes: 10,
            idle_pause_seconds: 60,
            idle_reset_seconds: 180,
            ..Default::default()
        };
        let t0 = Instant::now();
        let (mut state, _) = CoreState::new(cfg, t0);

        // User goes idle at t0 + 120s (480s remaining)
        let t1 = t0 + Duration::from_secs(120);
        let eff_idle = state.on_user_idled(t1);
        assert!(eff_idle.contains(&CoreEffect::CancelTimers));

        // Short idle (30s < 180-60=120s) resumes with ~480s remaining
        let t2 = t1 + Duration::from_secs(30);
        let eff_res = state.on_user_resumed(t2);
        assert!(eff_res.contains(&CoreEffect::ScheduleNextWakeup { delay_seconds: 470 }));

        // Long idle (150s >= 120s) resets to full 600s - 10s = 590s
        let t3 = t2 + Duration::from_secs(10);
        state.on_user_idled(t3);
        let t4 = t3 + Duration::from_secs(150);
        let eff_reset = state.on_user_resumed(t4);
        assert!(eff_reset.contains(&CoreEffect::ScheduleNextWakeup { delay_seconds: 590 }));
    }

    #[test]
    fn strict_mode_ignores_idle_and_resumes_from_idle_pause() {
        let cfg = Config {
            timer_mode: TimerMode::SmartPause,
            short_break_interval_minutes: 10,
            pre_break_warning_seconds: 10,
            idle_pause_seconds: 300,
            idle_reset_seconds: 300,
            ..Default::default()
        };
        let t0 = Instant::now();
        let (mut state, _) = CoreState::new(cfg, t0);

        // Pause due to idle in SmartPause mode
        let t1 = t0 + Duration::from_secs(300);
        let idle_eff = state.on_user_idled(t1);
        assert!(idle_eff.contains(&CoreEffect::CancelTimers));

        // Switching to Strict immediately resumes waiting with remaining 300s (- 10s warning = 290s)
        let switch_eff = state.set_timer_mode(TimerMode::Strict, t1 + Duration::from_secs(10));
        assert!(switch_eff.contains(&CoreEffect::ScheduleNextWakeup { delay_seconds: 290 }));

        // Subsequent idle events are ignored in Strict mode
        assert!(state.on_user_idled(t1 + Duration::from_secs(20)).is_empty());
        assert!(state.on_user_resumed(t1 + Duration::from_secs(60)).is_empty());
    }
}
