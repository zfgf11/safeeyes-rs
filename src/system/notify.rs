use notify_rust::{Notification, Timeout};
use tracing::warn;

use crate::core::scheduler::BreakKind;

/// Sends a lightweight pre-break desktop notification in a background thread
/// so D-Bus never blocks the GTK UI main loop.
pub fn send_pre_break_notification(kind: BreakKind, seconds_until_break: u64) {
    let summary = match kind {
        BreakKind::Short => "Short break soon",
        BreakKind::Long => "Long break soon",
    };
    let body = format!("Preparing for a break in {seconds_until_break} seconds.");

    let _ = std::thread::Builder::new()
        .name("safeeyes-notify".to_string())
        .spawn(move || {
            let timeout_ms = i32::try_from(seconds_until_break.saturating_mul(1000))
                .unwrap_or(10_000)
                .clamp(2_000, 15_000);
            if let Err(err) = Notification::new()
                .appname("SafeEyes")
                .summary(summary)
                .body(&body)
                .icon("appointment-soon")
                .timeout(Timeout::Milliseconds(timeout_ms as u32))
                .show()
            {
                warn!("failed to display pre-break notification: {err}");
            }
        });
}
