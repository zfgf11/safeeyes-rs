# AGENTS.md — SafeEyes Rust Rewrite (`safeeyes-rs`)

## Project Overview
A lightweight, ultra-low-resource Rust rewrite of **SafeEyes** for Linux (KDE Plasma & GNOME on Wayland).
It preserves the familiar system tray workflow while modernizing the break overlay with a clean, responsive, Apple-inspired fullscreen UI and modern zero-overhead Linux/Wayland APIs.

## Tech Stack & Efficiency Goals
- **Primary Target & Cross-DE Support:** Linux Wayland — built primarily for **KDE Plasma**, with clean out-of-the-box compatibility for **GNOME**.
- **Resource Footprint Target:** Minimal idle CPU (~0% between timer ticks) and minimal RAM footprint. Avoid heavy background runtimes, polling loops, or bloated dependencies.
- **Language:** Rust (latest stable).
- **GUI & Overlay:** `gtk4` (`gtk4-rs`) + `gtk4-layer-shell` (with automatic fallback to standard GTK4 fullscreen windows when `wlr-layer-shell` is unsupported, e.g., on GNOME Mutter). Styled via clean, modern GTK4 CSS.
- **System Tray:** `ksni` (native StatusNotifierItem D-Bus protocol — works natively on KDE Plasma and seamlessly on GNOME via the standard AppIndicator/KStatusNotifierItem extension).
- **System Integration (Idle & Screen Lock):** `zbus` (D-Bus `org.freedesktop.ScreenSaver`, `org.freedesktop.login1`, and optional `org.gnome.Mutter.IdleMonitor`) and/or `wayland-client` (`ext-idle-notify-v1`).
- **Notifications & Audio:** `notify-rust` (optional pre-break notification) and lightweight audio playback for subtle chimes.
- **Configuration:** `serde` + `toml` (`$XDG_CONFIG_HOME/safeeyes-rs/config.toml`).

## UI & UX Specifications (Mandatory)

### 1. System Tray Menu (`ksni`)
Keep the familiar SafeEyes tray menu structure and UX:
- **Status / Toggle:** Enable / Disable SafeEyes (plus timed disable options like *Disable for 15 min / 1 hour* if desired).
- **Next Break Info:** Disabled menu entry showing when the next break is due.
- **Take a break now:** Immediately triggers a break.
- **Settings / Reload Config** & **Quit**.

### 2. Fullscreen Break Overlay (Responsive, Apple-Style + GNOME Fallback)
Must scale cleanly on **any screen size, DPI, or aspect ratio** (from small 13" laptop panels to ultrawide or 4K monitors) using responsive GTK4 layout containers (`gtk4::Overlay`, `gtk4::Box`, `hexpand`/`vexpand`, relative margins/CSS) — **never hardcode pixel coordinates or fixed window sizes**.

**Wayland Compositor / DE Window Strategy (KDE + GNOME):**
- Always check `gtk4_layer_shell::is_supported()` before configuring overlay windows:
  - **When `true` (KDE KWin / wlroots):** Initialize `gtk4_layer_shell` on the window (`Layer::Overlay`, anchored to all 4 edges, `KeyboardMode::Exclusive`, bound to each `gdk::Monitor`).
  - **When `false` (GNOME Mutter fallback):** Do NOT panic or fail; instead configure a regular `gtk4::ApplicationWindow` (undecorated, modal/keep-above hints) and call `window.fullscreen_on_monitor(&monitor)` for each connected `gdk::Monitor`.

**Layout structure per monitor:**
- **Top-Right Corner:**
  - **"Lock screen now"** button anchored cleanly in the top-right corner (`halign = Align::End`, `valign = Align::Start`, generous corner margin).
  - **Cross-DE Lock Implementation:** Attempt D-Bus `org.freedesktop.ScreenSaver.Lock()` first, and fall back to `org.freedesktop.login1.Manager.LockSession` (for the current session) so screen locking works reliably on both KDE Plasma and GNOME.
- **Center Area (Hero Focus):**
  - **Exercise / Break Prompt:** Clean, high-contrast typography above or below the timer.
  - **Large Apple-Style Countdown:** Display remaining time strictly as **total integer seconds** (e.g. `60`, `59`, ..., `15`, `14` — **NOT** `1:00` or `0:15`). Use very large, bold, clean sans-serif typography with `tabular-nums` (`font-variant-numeric: tabular-nums`) so digits never jitter while counting down.
- **Bottom Area (Action Bar):**
  - Positioned relatively low near the bottom of the screen (`valign = Align::End`, bottom margin).
  - **Skip Button** placed side-by-side with a subtle status label showing **`Skipped X/Y breaks`** (e.g., `Skipped 1/2 breaks`, based on configurable `max_skips`). Disable or hide the Skip button once the limit is reached.
- **Visual Polish:** Dark, calm, frosted/translucent aesthetic via GTK4 CSS, smooth visual hierarchy, and keyboard shortcuts (`Esc` to skip if skips remain).

## Architectural & Quality Rules
1. **Modern Linux Native Only:** No X11 dependencies (`xlib`, `xprintidle`, etc.). Use Wayland (`gtk4-layer-shell` with GTK4 fullscreen fallback) and D-Bus.
2. **Event-Driven, Zero Polling:** Drive timers with `glib::timeout_add_seconds_local` and D-Bus signals via `zbus` — never busy-wait or poll in loops.
3. **Clean Separation:** Keep the state machine (`src/core/`), D-Bus/system integrations (`src/system/`), tray (`src/tray/`), and GTK4 overlay (`src/ui/`) decoupled.
4. **Error Handling:** Avoid `.unwrap()` / `.expect()` in runtime paths; log errors cleanly with `tracing`.
5. **Mandatory Verification:** Always run `cargo check` and `cargo clippy --all-targets` after changes and resolve all errors/warnings before finishing.
6. **Release & Packaging Sync (COPR & AUR):** When releasing program changes, bump the version consistently across `Cargo.toml`, `packaging/safeeyes-rs.spec` (`Version` + `%changelog`), `packaging/aur/PKGBUILD` (`pkgver`), and `packaging/aur/.SRCINFO` (`pkgver` + `source` + `sha256sums`), then commit and tag `vX.Y.Z` so Fedora COPR can build `https://raw.githubusercontent.com/zfgf11/safeeyes-rs/vX.Y.Z/packaging/safeeyes-rs.spec`.
