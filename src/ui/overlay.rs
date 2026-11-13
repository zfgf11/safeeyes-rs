use async_channel::Sender;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider,
    EventControllerKey, Justification, Label, Orientation, Overlay, PropagationPhase,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tracing::{debug, info, warn};

use crate::core::scheduler::BreakPlan;

const OVERLAY_CSS: &str = include_str!("style.css");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAction {
    SkipBreak,
    LockScreen,
}

struct MonitorOverlay {
    window: ApplicationWindow,
    countdown_label: Label,
}

/// Manages one fullscreen break overlay window per connected monitor.
pub struct BreakOverlayManager {
    css_loaded: bool,
    overlays: Vec<MonitorOverlay>,
}

impl BreakOverlayManager {
    pub fn new() -> Self {
        Self {
            css_loaded: false,
            overlays: Vec::new(),
        }
    }

    pub fn show(
        &mut self,
        app: &Application,
        plan: &BreakPlan,
        remaining_seconds: u64,
        skips_used: u32,
        max_skips: u32,
        action_tx: &Sender<OverlayAction>,
    ) {
        self.close();
        self.ensure_css_loaded();

        let use_layer_shell = gtk4_layer_shell::is_supported();
        info!(
            layer_shell = use_layer_shell,
            "opening break overlay ({}, {}s)",
            plan.kind.label(),
            remaining_seconds
        );

        let monitors = collect_monitors();
        if monitors.is_empty() {
            warn!("no GDK monitors reported; opening single fallback overlay window");
            let overlay = build_overlay_window(
                app,
                None,
                use_layer_shell,
                plan,
                remaining_seconds,
                skips_used,
                max_skips,
                action_tx,
            );
            overlay.window.present();
            self.overlays.push(overlay);
            return;
        }

        for monitor in monitors {
            let overlay = build_overlay_window(
                app,
                Some(&monitor),
                use_layer_shell,
                plan,
                remaining_seconds,
                skips_used,
                max_skips,
                action_tx,
            );
            overlay.window.present();
            self.overlays.push(overlay);
        }
    }

    pub fn update_countdown(&self, remaining_seconds: u64) {
        let text = remaining_seconds.to_string();
        for overlay in &self.overlays {
            overlay.countdown_label.set_text(&text);
        }
    }

    pub fn close(&mut self) {
        if self.overlays.is_empty() {
            return;
        }
        debug!("closing {} break overlay window(s)", self.overlays.len());
        for overlay in self.overlays.drain(..) {
            overlay.window.close();
        }
    }

    fn ensure_css_loaded(&mut self) {
        if self.css_loaded {
            return;
        }
        if let Some(display) = gdk::Display::default() {
            let provider = CssProvider::new();
            provider.load_from_string(OVERLAY_CSS);
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            self.css_loaded = true;
        } else {
            warn!("no default GDK display found; cannot attach overlay CSS");
        }
    }
}

impl Default for BreakOverlayManager {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_monitors() -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else {
        return Vec::new();
    };
    let model: gio::ListModel = display.monitors();
    let count = model.n_items();
    let mut monitors = Vec::with_capacity(count as usize);
    for idx in 0..count {
        if let Some(obj) = model.item(idx)
            && let Ok(monitor) = obj.downcast::<gdk::Monitor>()
        {
            monitors.push(monitor);
        }
    }
    monitors
}

#[allow(clippy::too_many_arguments)]
fn build_overlay_window(
    app: &Application,
    monitor: Option<&gdk::Monitor>,
    use_layer_shell: bool,
    plan: &BreakPlan,
    remaining_seconds: u64,
    skips_used: u32,
    max_skips: u32,
    action_tx: &Sender<OverlayAction>,
) -> MonitorOverlay {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("SafeEyes Break")
        .decorated(false)
        .build();
    window.add_css_class("safeeyes-overlay-window");

    if use_layer_shell {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_exclusive_zone(-1);
        window.set_namespace(Some("safeeyes-rs"));
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);
        if let Some(mon) = monitor {
            window.set_monitor(Some(mon));
        }
    } else {
        // GNOME Mutter / non-layer-shell Wayland fallback
        window.set_modal(true);
        if let Some(mon) = monitor {
            window.fullscreen_on_monitor(mon);
        } else {
            window.fullscreen();
        }
    }

    let root = Overlay::new();
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.add_css_class("safeeyes-root");

    // Center hero content
    let center_wrapper = GtkBox::new(Orientation::Vertical, 0);
    center_wrapper.set_hexpand(true);
    center_wrapper.set_vexpand(true);
    center_wrapper.set_halign(Align::Fill);
    center_wrapper.set_valign(Align::Fill);

    let top_spacer = GtkBox::new(Orientation::Vertical, 0);
    top_spacer.set_vexpand(true);

    let hero = GtkBox::new(Orientation::Vertical, 6);
    hero.set_halign(Align::Center);
    hero.set_valign(Align::Center);
    hero.set_margin_start(32);
    hero.set_margin_end(32);

    let badge = Label::new(Some(&plan.kind.label().to_uppercase()));
    badge.set_halign(Align::Center);
    badge.add_css_class("safeeyes-kind-badge");

    let prompt_label = Label::new(Some(&plan.prompt));
    prompt_label.set_halign(Align::Center);
    prompt_label.set_justify(Justification::Center);
    prompt_label.set_wrap(true);
    prompt_label.set_max_width_chars(38);
    prompt_label.add_css_class("safeeyes-prompt");

    // Display strictly as total integer seconds (e.g., 60, 59, ..., 15, 14).
    let countdown_label = Label::new(Some(&remaining_seconds.to_string()));
    countdown_label.set_halign(Align::Center);
    countdown_label.add_css_class("safeeyes-countdown");

    let subtitle = Label::new(Some("Relax your eyes and breathe slowly"));
    subtitle.set_halign(Align::Center);
    subtitle.add_css_class("safeeyes-subtitle");

    hero.append(&badge);
    hero.append(&prompt_label);
    hero.append(&countdown_label);
    hero.append(&subtitle);

    let bottom_spacer = GtkBox::new(Orientation::Vertical, 0);
    bottom_spacer.set_vexpand(true);

    center_wrapper.append(&top_spacer);
    center_wrapper.append(&hero);
    center_wrapper.append(&bottom_spacer);
    root.set_child(Some(&center_wrapper));

    // Top-right "Lock screen now" button
    let lock_btn = Button::with_label("Lock screen now");
    lock_btn.set_halign(Align::End);
    lock_btn.set_valign(Align::Start);
    lock_btn.set_margin_top(28);
    lock_btn.set_margin_end(32);
    lock_btn.set_focus_on_click(false);
    lock_btn.add_css_class("safeeyes-lock-btn");
    {
        let tx = action_tx.clone();
        lock_btn.connect_clicked(move |_| {
            let _ = tx.try_send(OverlayAction::LockScreen);
        });
    }
    root.add_overlay(&lock_btn);

    // Bottom action bar: Skip button + "Skipped X/Y breaks"
    let bottom_bar = GtkBox::new(Orientation::Horizontal, 8);
    bottom_bar.set_halign(Align::Center);
    bottom_bar.set_valign(Align::End);
    bottom_bar.set_margin_bottom(44);
    bottom_bar.add_css_class("safeeyes-bottom-bar");

    let can_skip = skips_used < max_skips;
    let skip_btn = Button::with_label("Skip");
    skip_btn.set_sensitive(can_skip);
    skip_btn.set_focus_on_click(false);
    skip_btn.add_css_class("safeeyes-skip-btn");
    {
        let tx = action_tx.clone();
        skip_btn.connect_clicked(move |_| {
            let _ = tx.try_send(OverlayAction::SkipBreak);
        });
    }

    let skip_status = Label::new(Some(&format!("Skipped {skips_used}/{max_skips} breaks")));
    skip_status.add_css_class("safeeyes-skip-status");

    bottom_bar.append(&skip_btn);
    bottom_bar.append(&skip_status);
    root.add_overlay(&bottom_bar);

    // Esc shortcut to skip when skips remain.
    if can_skip {
        let key_controller = EventControllerKey::new();
        key_controller.set_propagation_phase(PropagationPhase::Capture);
        let tx = action_tx.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                let _ = tx.try_send(OverlayAction::SkipBreak);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);
    }

    window.set_child(Some(&root));

    MonitorOverlay {
        window,
        countdown_label,
    }
}
