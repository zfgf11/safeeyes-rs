use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_channel::Sender;
use tracing::{debug, info, warn};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEvent {
    Idled,
    Resumed,
}

/// Handle allowing clean recreation/shutdown of the Wayland idle watcher when config reloads.
#[derive(Debug)]
pub struct IdleWatcherHandle {
    stop: Arc<AtomicBool>,
}

impl IdleWatcherHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Spawns a zero-polling Wayland `ext-idle-notify-v1` listener for the configured timeout.
/// Returns `None` if `idle_pause_seconds == 0`.
pub fn spawn_idle_monitor(
    idle_pause_seconds: u64,
    tx: Sender<IdleEvent>,
) -> Option<IdleWatcherHandle> {
    if idle_pause_seconds == 0 {
        info!("idle detection disabled by configuration");
        return None;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let spawn_res = std::thread::Builder::new()
        .name("safeeyes-wl-idle".to_string())
        .spawn(move || {
            if let Err(err) = run_wayland_idle_loop(idle_pause_seconds, tx, stop_flag) {
                warn!("Wayland idle monitor inactive: {err}");
            }
        });

    if let Err(err) = spawn_res {
        warn!("failed to spawn Wayland idle monitor thread: {err}");
        return None;
    }

    Some(IdleWatcherHandle { stop })
}

struct IdleWaylandState {
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<wl_seat::WlSeat>,
    notification: Option<ExtIdleNotificationV1>,
    tx: Sender<IdleEvent>,
}

delegate_noop!(IdleWaylandState: ignore wl_seat::WlSeat);
delegate_noop!(IdleWaylandState: ignore ExtIdleNotifierV1);

impl Dispatch<wl_registry::WlRegistry, ()> for IdleWaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_idle_notifier_v1" => {
                    let notifier = registry.bind::<ExtIdleNotifierV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    );
                    state.notifier = Some(notifier);
                }
                "wl_seat" if state.seat.is_none() => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(1), qh, ());
                    state.seat = Some(seat);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for IdleWaylandState {
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                debug!("Wayland ext-idle-notify-v1: idled");
                let _ = state.tx.send_blocking(IdleEvent::Idled);
            }
            ext_idle_notification_v1::Event::Resumed => {
                debug!("Wayland ext-idle-notify-v1: resumed");
                let _ = state.tx.send_blocking(IdleEvent::Resumed);
            }
            _ => {}
        }
    }
}

fn run_wayland_idle_loop(
    idle_pause_seconds: u64,
    tx: Sender<IdleEvent>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland connect failed: {e}"))?;
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = IdleWaylandState {
        notifier: None,
        seat: None,
        notification: None,
        tx,
    };

    display.get_registry(&qh, ());
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| format!("Wayland registry roundtrip failed: {e}"))?;

    let Some(notifier) = state.notifier.as_ref() else {
        return Err("compositor does not expose ext_idle_notifier_v1".to_string());
    };
    let Some(seat) = state.seat.as_ref() else {
        return Err("no wl_seat global found".to_string());
    };

    let timeout_ms = u32::try_from(idle_pause_seconds.saturating_mul(1000)).unwrap_or(u32::MAX);
    let notification = notifier.get_idle_notification(timeout_ms, seat, &qh, ());
    state.notification = Some(notification);
    event_queue
        .flush()
        .map_err(|e| format!("Wayland flush failed: {e}"))?;

    info!("Wayland ext-idle-notify-v1 active (timeout={idle_pause_seconds}s)");

    while !stop.load(Ordering::Relaxed) {
        event_queue
            .blocking_dispatch(&mut state)
            .map_err(|e| format!("Wayland dispatch failed: {e}"))?;
    }

    if let Some(notif) = state.notification.take() {
        notif.destroy();
    }
    if let Some(notifier) = state.notifier.take() {
        notifier.destroy();
    }
    let _ = event_queue.flush();
    Ok(())
}
