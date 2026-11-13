# safeeyes-rs

A lightweight, native Wayland Rust rewrite of [SafeEyes](https://github.com/slgobinath/SafeEyes) for **KDE Plasma** and **GNOME**.

`safeeyes-rs` preserves the familiar system tray workflow while delivering a modern, responsive fullscreen break overlay and event-driven zero-polling Wayland integration.

## Features

- **Native Wayland Fullscreen Overlay**
  - Uses `gtk4-layer-shell` (`Layer::Overlay`, `KeyboardMode::Exclusive`, multi-monitor) on KDE Plasma / wlroots compositors.
  - Automatic fallback to undecorated `gtk4::ApplicationWindow::fullscreen_on_monitor` on GNOME Mutter.
  - Responsive layout with a large integer-second countdown (`tabular-nums`), top-right **Lock screen now** button, bottom **Skip** button with `Skipped X/Y breaks` counter, and `Esc` shortcut.
- **Cross-DE System Tray (`ksni` / StatusNotifierItem)**
  - Next break status line, Enable/Disable toggle, timed disable (*15 minutes* / *1 hour*), **Take a 15s break now**, **Take a 60s break now**, **Timer mode** switcher, Settings (*Open config file* / *Reload config*), and Quit.
- **Flexible Timer Modes**
  - **Smart Pause (default):** Uses Wayland `ext-idle-notify-v1` to pause on user inactivity and reset after `idle_reset_seconds` (respects video/media idle inhibitors such as browser playback).
  - **Strict:** Ignores user inactivity and schedules breaks strictly by elapsed time since the last break.
- **Event-Driven & Low Footprint**
  - Zero polling between breaks (single-shot GLib wakeups, D-Bus signal listeners for `org.freedesktop.ScreenSaver` and `org.freedesktop.login1`, and Wayland `ext-idle-notify-v1`).
  - Pre-break desktop notifications via `notify-rust` and lightweight chimes via `pw-play` / `paplay` / `canberra-gtk-play`.

## Requirements

System development libraries for GTK 4 and `gtk4-layer-shell`:

- **Fedora:**
  ```bash
  sudo dnf install gtk4-devel gtk4-layer-shell-devel
  ```
- **Ubuntu / Debian:**
  ```bash
  sudo apt install libgtk-4-dev libgtk4-layer-shell-dev
  ```
- **Arch Linux:**
  ```bash
  sudo pacman -S gtk4 gtk4-layer-shell
  ```

> **GNOME Note:** For the system tray icon to appear on GNOME, ensure the *AppIndicator and KStatusNotifierItem Support* extension is enabled.

## Build & Run

```bash
cargo build --release
cargo run --release
```

To install locally into `~/.cargo/bin`:

```bash
cargo install --path .
```

## Configuration

On first launch, `safeeyes-rs` creates a default configuration file at:

```text
$XDG_CONFIG_HOME/safeeyes-rs/config.toml   # usually ~/.config/safeeyes-rs/config.toml
```

After editing `config.toml`, apply changes immediately via **Settings → Reload config** in the tray menu.


