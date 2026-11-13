use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use async_channel::Sender;
use gtk4::gio::{self, ApplicationHoldGuard};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::Application;
use ksni::blocking::Handle as TrayHandle;
use tracing::{error, info, warn};

use crate::core::config::{Config, TimerMode};
use crate::core::scheduler::BreakKind;
use crate::core::state::{CoreEffect, CoreState, PauseReason};
use crate::system::audio;
use crate::system::idle::{IdleEvent, IdleWatcherHandle, spawn_idle_monitor};
use crate::system::lock::lock_screen_async;
use crate::system::notify::send_pre_break_notification;
use crate::system::session::{SessionEvent, spawn_session_monitors};
use crate::tray::{SafeEyesTray, TrayCommand, TraySnapshot};
use crate::ui::overlay::{BreakOverlayManager, OverlayAction};

const APP_ID: &str = "io.github.safeeyes_rs";

pub fn run() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();

    let initialized = Rc::new(Cell::new(false));
    let initialized_flag = Rc::clone(&initialized);

    app.connect_activate(move |gtk_app| {
        if initialized_flag.replace(true) {
            info!("application already running in system tray");
            return;
        }
        start_application(gtk_app);
    });

    app.run()
}

struct AppRuntime {
    app: Application,
    _hold: ApplicationHoldGuard,
    config_path: Option<PathBuf>,
    core: CoreState,
    overlay: BreakOverlayManager,
    overlay_tx: Sender<OverlayAction>,
    tray_handle: Option<TrayHandle<SafeEyesTray>>,
    idle_watcher: Option<IdleWatcherHandle>,
    idle_tx: Sender<(u64, IdleEvent)>,
    idle_generation: u64,
    wait_timer: Option<glib::SourceId>,
    break_ticker: Option<glib::SourceId>,
    reenable_timer: Option<glib::SourceId>,
}

impl AppRuntime {
    fn snapshot(&self) -> TraySnapshot {
        let cfg = self.core.config();
        TraySnapshot {
            enabled: self.core.is_enabled(),
            in_break: self.core.is_in_break(),
            status_line: self.core.tray_status_line(),
            short_break_seconds: cfg.short_break_duration_seconds,
            long_break_seconds: cfg.long_break_duration_seconds,
            timer_mode: cfg.timer_mode,
            idle_reset_seconds: cfg.idle_reset_seconds,
        }
    }

    fn cancel_wait_and_break_timers(&mut self) {
        if let Some(id) = self.wait_timer.take() {
            id.remove();
        }
        if let Some(id) = self.break_ticker.take() {
            id.remove();
        }
    }

    fn cancel_all_timers(&mut self) {
        self.cancel_wait_and_break_timers();
        if let Some(id) = self.reenable_timer.take() {
            id.remove();
        }
    }
}

fn start_application(app: &Application) {
    let hold = app.hold();

    let (config, config_path) = match Config::load_or_create_default() {
        Ok((cfg, path)) => (cfg, Some(path)),
        Err(err) => {
            warn!("failed to load config, using defaults: {err}");
            (Config::default(), None)
        }
    };

    let (tray_tx, tray_rx) = async_channel::unbounded::<TrayCommand>();
    let (session_tx, session_rx) = async_channel::unbounded::<SessionEvent>();
    let (idle_tx, idle_rx) = async_channel::unbounded::<(u64, IdleEvent)>();
    let (overlay_tx, overlay_rx) = async_channel::unbounded::<OverlayAction>();

    let (core, initial_effects) = CoreState::new(config.clone(), Instant::now());
    let initial_snapshot = TraySnapshot {
        enabled: core.is_enabled(),
        in_break: core.is_in_break(),
        status_line: core.tray_status_line(),
        short_break_seconds: config.short_break_duration_seconds,
        long_break_seconds: config.long_break_duration_seconds,
        timer_mode: config.timer_mode,
        idle_reset_seconds: config.idle_reset_seconds,
    };

    let tray_handle = SafeEyesTray::spawn(tray_tx, initial_snapshot);
    spawn_session_monitors(session_tx);
    let idle_watcher = if config.timer_mode == TimerMode::SmartPause {
        spawn_idle_with_gen(config.idle_pause_seconds, 1, &idle_tx)
    } else {
        None
    };

    let runtime = Rc::new(RefCell::new(AppRuntime {
        app: app.clone(),
        _hold: hold,
        config_path,
        core,
        overlay: BreakOverlayManager::new(),
        overlay_tx,
        tray_handle,
        idle_watcher,
        idle_tx,
        idle_generation: 1,
        wait_timer: None,
        break_ticker: None,
        reenable_timer: None,
    }));

    apply_effects(&runtime, initial_effects);

    let main_ctx = glib::MainContext::default();

    // Tray events
    {
        let rt = Rc::clone(&runtime);
        main_ctx.spawn_local(async move {
            while let Ok(cmd) = tray_rx.recv().await {
                handle_tray_command(&rt, cmd);
            }
        });
    }

    // D-Bus session lock/unlock & sleep events
    {
        let rt = Rc::clone(&runtime);
        main_ctx.spawn_local(async move {
            while let Ok(ev) = session_rx.recv().await {
                handle_session_event(&rt, ev);
            }
        });
    }

    // Wayland ext-idle-notify-v1 events
    {
        let rt = Rc::clone(&runtime);
        main_ctx.spawn_local(async move {
            while let Ok((generation, ev)) = idle_rx.recv().await {
                handle_idle_event(&rt, generation, ev);
            }
        });
    }

    // Overlay button / key events
    {
        let rt = Rc::clone(&runtime);
        main_ctx.spawn_local(async move {
            while let Ok(action) = overlay_rx.recv().await {
                handle_overlay_action(&rt, action);
            }
        });
    }

    info!("safeeyes-rs started and running in system tray");
}

fn spawn_idle_with_gen(
    idle_pause_seconds: u64,
    generation: u64,
    outer_tx: &Sender<(u64, IdleEvent)>,
) -> Option<IdleWatcherHandle> {
    if idle_pause_seconds == 0 {
        return None;
    }
    let (raw_tx, raw_rx) = async_channel::unbounded::<IdleEvent>();
    let handle = spawn_idle_monitor(idle_pause_seconds, raw_tx)?;

    let forwarded_tx = outer_tx.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(ev) = raw_rx.recv().await {
            if forwarded_tx.send((generation, ev)).await.is_err() {
                break;
            }
        }
    });
    Some(handle)
}

fn handle_tray_command(rt: &Rc<RefCell<AppRuntime>>, cmd: TrayCommand) {
    let now = Instant::now();
    match cmd {
        TrayCommand::ToggleEnabled => {
            let effects = {
                let mut r = rt.borrow_mut();
                if r.core.is_enabled() {
                    r.core.disable(None)
                } else {
                    if let Some(id) = r.reenable_timer.take() {
                        id.remove();
                    }
                    r.core.enable(now)
                }
            };
            apply_effects(rt, effects);
        }
        TrayCommand::DisableForMinutes(mins) => {
            let effects = {
                let mut r = rt.borrow_mut();
                if let Some(id) = r.reenable_timer.take() {
                    id.remove();
                }
                r.core.disable(Some(Duration::from_secs(mins * 60)))
            };
            apply_effects(rt, effects);
        }
        TrayCommand::DisableIndefinitely => {
            let effects = {
                let mut r = rt.borrow_mut();
                if let Some(id) = r.reenable_timer.take() {
                    id.remove();
                }
                r.core.disable(None)
            };
            apply_effects(rt, effects);
        }
        TrayCommand::SetTimerMode(mode) => {
            let (effects, config_path) = {
                let mut r = rt.borrow_mut();
                let prev_mode = r.core.config().timer_mode;
                let effects = r.core.set_timer_mode(mode, now);
                if prev_mode != mode {
                    if let Some(watcher) = r.idle_watcher.take() {
                        watcher.stop();
                    }
                    r.idle_generation = r.idle_generation.saturating_add(1);
                    if mode == TimerMode::SmartPause {
                        let idle_secs = r.core.config().idle_pause_seconds;
                        let generation = r.idle_generation;
                        let tx = r.idle_tx.clone();
                        r.idle_watcher = spawn_idle_with_gen(idle_secs, generation, &tx);
                    }
                    info!("switched timer mode to {mode:?}");
                }
                (effects, r.config_path.clone())
            };
            if let Some(path) = config_path
                && let Err(err) = Config::persist_timer_mode(&path, mode)
            {
                warn!("failed to persist timer_mode to {}: {err}", path.display());
            }
            apply_effects(rt, effects);
        }
        TrayCommand::TakeShortBreakNow => {
            let effects = rt.borrow_mut().core.take_break_now(BreakKind::Short);
            apply_effects(rt, effects);
        }
        TrayCommand::TakeLongBreakNow => {
            let effects = rt.borrow_mut().core.take_break_now(BreakKind::Long);
            apply_effects(rt, effects);
        }
        TrayCommand::ReloadConfig => {
            reload_config(rt, now);
        }
        TrayCommand::OpenConfig => {
            let path_opt = rt.borrow().config_path.clone();
            if let Some(path) = path_opt {
                if !path.exists() {
                    let _ = Config::load_or_create_default();
                }
                let uri = format!("file://{}", path.display());
                if let Err(err) =
                    gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE)
                {
                    warn!("failed to open config file {}: {err}", path.display());
                }
            }
        }
        TrayCommand::Quit => {
            info!("quitting safeeyes-rs");
            let app = {
                let mut r = rt.borrow_mut();
                r.cancel_all_timers();
                r.overlay.close();
                if let Some(watcher) = r.idle_watcher.take() {
                    watcher.stop();
                }
                if let Some(tray) = r.tray_handle.take() {
                    tray.shutdown();
                }
                r.app.clone()
            };
            app.quit();
        }
    }
}

fn reload_config(rt: &Rc<RefCell<AppRuntime>>, now: Instant) {
    let (new_cfg, path) = match Config::load_or_create_default() {
        Ok(res) => res,
        Err(err) => {
            error!("failed to reload config: {err}");
            return;
        }
    };

    let effects = {
        let mut r = rt.borrow_mut();
        let old_idle_secs = r.core.config().idle_pause_seconds;
        let old_mode = r.core.config().timer_mode;
        let new_idle_secs = new_cfg.idle_pause_seconds;
        let new_mode = new_cfg.timer_mode;
        r.config_path = Some(path.clone());

        if old_idle_secs != new_idle_secs || old_mode != new_mode {
            if let Some(watcher) = r.idle_watcher.take() {
                watcher.stop();
            }
            r.idle_generation = r.idle_generation.saturating_add(1);
            if new_mode == TimerMode::SmartPause {
                let generation = r.idle_generation;
                let tx = r.idle_tx.clone();
                r.idle_watcher = spawn_idle_with_gen(new_idle_secs, generation, &tx);
            }
        }

        r.core.update_config(new_cfg, now)
    };

    info!("reloaded config from {}", path.display());
    apply_effects(rt, effects);
}

fn handle_session_event(rt: &Rc<RefCell<AppRuntime>>, ev: SessionEvent) {
    info!("session event: {ev:?}");
    let now = Instant::now();
    let effects = {
        let mut r = rt.borrow_mut();
        match ev {
            SessionEvent::ScreenLocked => r.core.on_screen_locked(now, PauseReason::ScreenLocked),
            SessionEvent::Sleeping => r.core.on_screen_locked(now, PauseReason::Suspended),
            SessionEvent::ScreenUnlocked | SessionEvent::WokeUp => r.core.on_screen_unlocked(now),
        }
    };
    apply_effects(rt, effects);
}

fn handle_idle_event(rt: &Rc<RefCell<AppRuntime>>, generation: u64, ev: IdleEvent) {
    let now = Instant::now();
    let effects = {
        let mut r = rt.borrow_mut();
        if generation != r.idle_generation {
            return;
        }
        info!("idle event: {ev:?}");
        match ev {
            IdleEvent::Idled => r.core.on_user_idled(now),
            IdleEvent::Resumed => r.core.on_user_resumed(now),
        }
    };
    apply_effects(rt, effects);
}

fn handle_overlay_action(rt: &Rc<RefCell<AppRuntime>>, action: OverlayAction) {
    let now = Instant::now();
    match action {
        OverlayAction::SkipBreak => {
            let effects = rt.borrow_mut().core.skip_break(now);
            apply_effects(rt, effects);
        }
        OverlayAction::SnoozeBreak => {
            let effects = rt.borrow_mut().core.snooze_break(now);
            apply_effects(rt, effects);
        }
        OverlayAction::LockScreen => {
            lock_screen_async();
        }
    }
}

fn apply_effects(rt: &Rc<RefCell<AppRuntime>>, effects: Vec<CoreEffect>) {
    for effect in effects {
        match effect {
            CoreEffect::CancelTimers => {
                rt.borrow_mut().cancel_wait_and_break_timers();
            }
            CoreEffect::ScheduleNextWakeup { delay_seconds } => {
                let mut r = rt.borrow_mut();
                if let Some(id) = r.wait_timer.take() {
                    id.remove();
                }
                let delay = u32::try_from(delay_seconds.max(1)).unwrap_or(u32::MAX);
                info!("scheduling next wakeup in {delay}s ({})", r.core.tray_status_line());
                let rt_weak = Rc::downgrade(rt);
                let id = glib::timeout_add_seconds_local_once(delay, move || {
                    if let Some(rt_strong) = rt_weak.upgrade() {
                        let effects = {
                            let mut inner = rt_strong.borrow_mut();
                            inner.wait_timer = None;
                            inner.core.on_wait_timer(Instant::now())
                        };
                        apply_effects(&rt_strong, effects);
                    }
                });
                r.wait_timer = Some(id);
            }
            CoreEffect::ScheduleReenable { delay_seconds } => {
                let mut r = rt.borrow_mut();
                if let Some(id) = r.reenable_timer.take() {
                    id.remove();
                }
                let delay = u32::try_from(delay_seconds.max(1)).unwrap_or(u32::MAX);
                let rt_weak = Rc::downgrade(rt);
                let id = glib::timeout_add_seconds_local_once(delay, move || {
                    if let Some(rt_strong) = rt_weak.upgrade() {
                        let effects = {
                            let mut inner = rt_strong.borrow_mut();
                            inner.reenable_timer = None;
                            inner.core.enable(Instant::now())
                        };
                        apply_effects(&rt_strong, effects);
                    }
                });
                r.reenable_timer = Some(id);
            }
            CoreEffect::ShowPreBreakNotification {
                kind,
                warning_seconds,
            } => {
                send_pre_break_notification(kind, warning_seconds);
            }
            CoreEffect::OpenBreakOverlay {
                plan,
                remaining_seconds,
                skips_used,
                max_skips,
            } => {
                let mut r = rt.borrow_mut();
                let app = r.app.clone();
                let action_tx = r.overlay_tx.clone();
                let snooze_minutes = r.core.config().snooze_duration_minutes;
                r.overlay.show(
                    &app,
                    &plan,
                    remaining_seconds,
                    skips_used,
                    max_skips,
                    snooze_minutes,
                    &action_tx,
                );

                if let Some(id) = r.break_ticker.take() {
                    id.remove();
                }
                let rt_weak = Rc::downgrade(rt);
                let ticker_id = glib::timeout_add_seconds_local(1, move || {
                    let Some(rt_strong) = rt_weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    let effects = rt_strong.borrow_mut().core.on_break_tick(Instant::now());
                    let still_in_break = rt_strong.borrow().core.is_in_break();
                    if !still_in_break {
                        rt_strong.borrow_mut().break_ticker = None;
                    }
                    apply_effects(&rt_strong, effects);
                    if still_in_break {
                        glib::ControlFlow::Continue
                    } else {
                        glib::ControlFlow::Break
                    }
                });
                r.break_ticker = Some(ticker_id);
            }
            CoreEffect::UpdateBreakCountdown { remaining_seconds } => {
                rt.borrow().overlay.update_countdown(remaining_seconds);
            }
            CoreEffect::CloseBreakOverlay => {
                rt.borrow_mut().overlay.close();
            }
            CoreEffect::PlayBreakStartChime => {
                let r = rt.borrow();
                let cfg = r.core.config();
                audio::play_break_start(cfg.enable_sound, cfg.start_sound_path.as_deref());
            }
            CoreEffect::PlayBreakEndChime => {
                let r = rt.borrow();
                let cfg = r.core.config();
                audio::play_break_end(cfg.enable_sound, cfg.end_sound_path.as_deref());
            }
            CoreEffect::RefreshTray => {
                let (tray_opt, snapshot) = {
                    let r = rt.borrow();
                    (r.tray_handle.clone(), r.snapshot())
                };
                if let Some(tray) = tray_opt {
                    tray.update(|t| t.apply_snapshot(snapshot));
                }
            }
        }
    }
}
