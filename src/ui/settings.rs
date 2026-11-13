use std::cell::RefCell;
use std::rc::Rc;

use async_channel::Sender;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, DropDown, Frame, Label,
    Orientation, PolicyType, ScrolledWindow, SpinButton, Switch, TextView, WrapMode,
};

use crate::core::config::{Config, TimerMode};

#[derive(Debug, Clone)]
pub enum SettingsAction {
    Save(Config),
}

/// Manages a lazily constructed Preferences window (0 idle memory when closed).
pub struct SettingsWindowManager {
    window: Rc<RefCell<Option<ApplicationWindow>>>,
}

impl SettingsWindowManager {
    pub fn new() -> Self {
        Self {
            window: Rc::new(RefCell::new(None)),
        }
    }

    pub fn open(
        &self,
        app: &Application,
        current: &Config,
        action_tx: &Sender<SettingsAction>,
    ) {
        if let Some(existing) = self.window.borrow().as_ref() {
            existing.present();
            return;
        }

        let win = build_settings_window(app, current, action_tx, &self.window);
        *self.window.borrow_mut() = Some(win.clone());
        win.present();
    }
}

impl Default for SettingsWindowManager {
    fn default() -> Self {
        Self::new()
    }
}

struct SettingsWidgets {
    interval_mins: SpinButton,
    short_secs: SpinButton,
    short_before_long: SpinButton,
    long_secs: SpinButton,
    pre_break_secs: SpinButton,
    max_skips: SpinButton,
    snooze_mins: SpinButton,
    timer_mode: DropDown,
    idle_pause_secs: SpinButton,
    idle_reset_secs: SpinButton,
    enable_sound: Switch,
    short_exercises: TextView,
    long_exercises: TextView,
}

impl SettingsWidgets {
    fn apply_config(&self, cfg: &Config) {
        self.interval_mins
            .set_value(cfg.short_break_interval_minutes as f64);
        self.short_secs
            .set_value(cfg.short_break_duration_seconds as f64);
        self.short_before_long
            .set_value(f64::from(cfg.short_breaks_before_long));
        self.long_secs
            .set_value(cfg.long_break_duration_seconds as f64);
        self.pre_break_secs
            .set_value(cfg.pre_break_warning_seconds as f64);
        self.max_skips.set_value(f64::from(cfg.max_skips));
        self.snooze_mins
            .set_value(cfg.snooze_duration_minutes as f64);
        self.timer_mode.set_selected(match cfg.timer_mode {
            TimerMode::SmartPause => 0,
            TimerMode::Strict => 1,
        });
        self.idle_pause_secs
            .set_value(cfg.idle_pause_seconds as f64);
        self.idle_reset_secs
            .set_value(cfg.idle_reset_seconds as f64);
        self.enable_sound.set_active(cfg.enable_sound);
        self.short_exercises
            .buffer()
            .set_text(&cfg.short_break_exercises.join("\n"));
        self.long_exercises
            .buffer()
            .set_text(&cfg.long_break_exercises.join("\n"));
    }

    fn read_config(&self, base: &Config) -> Config {
        let mut cfg = base.clone();
        cfg.short_break_interval_minutes = self.interval_mins.value_as_int().max(1) as u64;
        cfg.short_break_duration_seconds = self.short_secs.value_as_int().max(1) as u64;
        cfg.short_breaks_before_long = self.short_before_long.value_as_int().max(0) as u32;
        cfg.long_break_duration_seconds = self.long_secs.value_as_int().max(1) as u64;
        cfg.pre_break_warning_seconds = self.pre_break_secs.value_as_int().max(0) as u64;
        cfg.max_skips = self.max_skips.value_as_int().max(0) as u32;
        cfg.snooze_duration_minutes = self.snooze_mins.value_as_int().max(1) as u64;
        cfg.timer_mode = if self.timer_mode.selected() == 1 {
            TimerMode::Strict
        } else {
            TimerMode::SmartPause
        };
        cfg.idle_pause_seconds = self.idle_pause_secs.value_as_int().max(0) as u64;
        cfg.idle_reset_seconds = self.idle_reset_secs.value_as_int().max(0) as u64;
        cfg.enable_sound = self.enable_sound.is_active();
        cfg.short_break_exercises = read_lines(&self.short_exercises);
        cfg.long_break_exercises = read_lines(&self.long_exercises);
        cfg.sanitize();
        cfg
    }
}

fn build_settings_window(
    app: &Application,
    current: &Config,
    action_tx: &Sender<SettingsAction>,
    slot: &Rc<RefCell<Option<ApplicationWindow>>>,
) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("SafeEyes Preferences")
        .default_width(520)
        .default_height(620)
        .build();

    let slot_weak = Rc::downgrade(slot);
    window.connect_close_request(move |_| {
        if let Some(cell) = slot_weak.upgrade() {
            *cell.borrow_mut() = None;
        }
        glib::Propagation::Proceed
    });

    let outer = GtkBox::new(Orientation::Vertical, 0);

    let scroller = ScrolledWindow::new();
    scroller.set_vexpand(true);
    scroller.set_policy(PolicyType::Never, PolicyType::Automatic);

    let content = GtkBox::new(Orientation::Vertical, 18);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(22);
    content.set_margin_end(22);

    // Breaks & Timing
    let breaks_box = build_section(&content, "Breaks & Timing");
    let interval_mins = add_spin_row(
        &breaks_box,
        "Break interval (minutes)",
        "Time between breaks",
        1.0,
        240.0,
        1.0,
    );
    let short_secs = add_spin_row(
        &breaks_box,
        "Short break duration (seconds)",
        "Length of each short break",
        1.0,
        600.0,
        1.0,
    );
    let short_before_long = add_spin_row(
        &breaks_box,
        "Short breaks before long break",
        "Set to 0 to disable long breaks",
        0.0,
        20.0,
        1.0,
    );
    let long_secs = add_spin_row(
        &breaks_box,
        "Long break duration (seconds)",
        "Length of each long break",
        1.0,
        1800.0,
        5.0,
    );
    let pre_break_secs = add_spin_row(
        &breaks_box,
        "Pre-break warning (seconds)",
        "Desktop notification before break (0 = off)",
        0.0,
        120.0,
        1.0,
    );
    let max_skips = add_spin_row(
        &breaks_box,
        "Max skips / snoozes",
        "Allowed skips or snoozes before reset (0 = none)",
        0.0,
        20.0,
        1.0,
    );
    let snooze_mins = add_spin_row(
        &breaks_box,
        "Snooze duration (minutes)",
        "Postpone duration when clicking Snooze",
        1.0,
        60.0,
        1.0,
    );

    // Idle & Timer Mode
    let idle_box = build_section(&content, "Idle & Timer Mode");
    let timer_mode = DropDown::from_strings(&[
        "Smart Pause (idle-aware)",
        "Strict (fixed interval)",
    ]);
    add_widget_row(
        &idle_box,
        "Timer mode",
        "Smart Pause pauses on inactivity and respects video inhibitors",
        &timer_mode,
    );
    let idle_pause_secs = add_spin_row(
        &idle_box,
        "Idle pause threshold (seconds)",
        "Inactivity before pausing in Smart Pause mode (0 = off)",
        0.0,
        3600.0,
        30.0,
    );
    let idle_reset_secs = add_spin_row(
        &idle_box,
        "Idle reset threshold (seconds)",
        "Inactivity after which break interval resets on return",
        0.0,
        3600.0,
        30.0,
    );

    // Audio
    let audio_box = build_section(&content, "Audio");
    let enable_sound = Switch::new();
    enable_sound.set_valign(Align::Center);
    add_widget_row(
        &audio_box,
        "Play break chimes",
        "Subtle chime at break start and completion",
        &enable_sound,
    );

    // Exercises
    let exercises_box = build_section(&content, "Exercises (one per line)");
    let short_exercises = add_text_section(&exercises_box, "Short break prompts", 96);
    let long_exercises = add_text_section(&exercises_box, "Long break prompts", 76);

    scroller.set_child(Some(&content));
    outer.append(&scroller);

    let widgets = Rc::new(SettingsWidgets {
        interval_mins,
        short_secs,
        short_before_long,
        long_secs,
        pre_break_secs,
        max_skips,
        snooze_mins,
        timer_mode,
        idle_pause_secs,
        idle_reset_secs,
        enable_sound,
        short_exercises,
        long_exercises,
    });
    widgets.apply_config(current);

    // Action bar
    let footer = GtkBox::new(Orientation::Horizontal, 10);
    footer.set_margin_top(12);
    footer.set_margin_bottom(14);
    footer.set_margin_start(22);
    footer.set_margin_end(22);

    let reset_btn = Button::with_label("Reset Defaults");
    {
        let w = Rc::clone(&widgets);
        reset_btn.connect_clicked(move |_| {
            w.apply_config(&Config::default());
        });
    }

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);

    let cancel_btn = Button::with_label("Cancel");
    {
        let win_weak = window.downgrade();
        cancel_btn.connect_clicked(move |_| {
            if let Some(w) = win_weak.upgrade() {
                w.close();
            }
        });
    }

    let save_btn = Button::with_label("Save");
    save_btn.add_css_class("suggested-action");
    {
        let w = Rc::clone(&widgets);
        let base = current.clone();
        let tx = action_tx.clone();
        let win_weak = window.downgrade();
        save_btn.connect_clicked(move |_| {
            let updated = w.read_config(&base);
            let _ = tx.try_send(SettingsAction::Save(updated));
            if let Some(win) = win_weak.upgrade() {
                win.close();
            }
        });
    }

    footer.append(&reset_btn);
    footer.append(&spacer);
    footer.append(&cancel_btn);
    footer.append(&save_btn);
    outer.append(&footer);

    window.set_child(Some(&outer));
    window
}

fn build_section(parent: &GtkBox, title: &str) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 6);

    let heading = Label::new(None);
    heading.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(title)));
    heading.set_halign(Align::Start);
    container.append(&heading);

    let frame = Frame::new(None);
    let inner = GtkBox::new(Orientation::Vertical, 10);
    inner.set_margin_top(12);
    inner.set_margin_bottom(12);
    inner.set_margin_start(14);
    inner.set_margin_end(14);
    frame.set_child(Some(&inner));
    container.append(&frame);

    parent.append(&container);
    inner
}

fn add_spin_row(
    parent: &GtkBox,
    title: &str,
    subtitle: &str,
    min: f64,
    max: f64,
    step: f64,
) -> SpinButton {
    let spin = SpinButton::with_range(min, max, step);
    spin.set_digits(0);
    spin.set_valign(Align::Center);
    add_widget_row(parent, title, subtitle, &spin);
    spin
}

fn add_widget_row(
    parent: &GtkBox,
    title: &str,
    subtitle: &str,
    control: &impl IsA<gtk4::Widget>,
) {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    let labels = GtkBox::new(Orientation::Vertical, 2);
    labels.set_hexpand(true);

    let title_lbl = Label::new(Some(title));
    title_lbl.set_halign(Align::Start);
    let sub_lbl = Label::new(Some(subtitle));
    sub_lbl.set_halign(Align::Start);
    sub_lbl.set_wrap(true);
    sub_lbl.add_css_class("dim-label");

    labels.append(&title_lbl);
    labels.append(&sub_lbl);
    row.append(&labels);
    row.append(control);
    parent.append(&row);
}

fn add_text_section(parent: &GtkBox, title: &str, min_height: i32) -> TextView {
    let lbl = Label::new(Some(title));
    lbl.set_halign(Align::Start);
    parent.append(&lbl);

    let view = TextView::new();
    view.set_wrap_mode(WrapMode::WordChar);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.set_left_margin(8);
    view.set_right_margin(8);

    let scroller = ScrolledWindow::new();
    scroller.set_min_content_height(min_height);
    scroller.set_child(Some(&view));

    let frame = Frame::new(None);
    frame.set_child(Some(&scroller));
    parent.append(&frame);

    view
}

fn read_lines(view: &TextView) -> Vec<String> {
    let buf = view.buffer();
    let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}
