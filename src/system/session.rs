use async_channel::Sender;
use tracing::{debug, info, warn};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::OwnedObjectPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    ScreenLocked,
    ScreenUnlocked,
    Sleeping,
    WokeUp,
}

/// Starts event-driven D-Bus listeners for screen lock (`org.freedesktop.ScreenSaver` + `login1`)
/// and system sleep (`org.freedesktop.login1.Manager.PrepareForSleep`).
pub fn spawn_session_monitors(tx: Sender<SessionEvent>) {
    let tx_ss = tx.clone();
    if let Err(err) = std::thread::Builder::new()
        .name("safeeyes-dbus-ss".to_string())
        .spawn(move || {
            if let Err(e) = watch_screensaver_signals(&tx_ss) {
                warn!("ScreenSaver D-Bus monitor stopped: {e}");
            }
        })
    {
        warn!("could not spawn ScreenSaver monitor thread: {err}");
    }

    if let Err(err) = std::thread::Builder::new()
        .name("safeeyes-dbus-login1".to_string())
        .spawn(move || {
            if let Err(e) = watch_login1_signals(&tx) {
                warn!("login1 D-Bus monitor stopped: {e}");
            }
        })
    {
        warn!("could not spawn login1 monitor thread: {err}");
    }
}

fn watch_screensaver_signals(tx: &Sender<SessionEvent>) -> zbus::Result<()> {
    let conn = Connection::session()?;
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.ScreenSaver")?
        .member("ActiveChanged")?
        .build();
    let iter = MessageIterator::for_match_rule(rule, &conn, None)?;
    info!("listening for org.freedesktop.ScreenSaver.ActiveChanged signals");

    for msg_res in iter {
        let msg = match msg_res {
            Ok(m) => m,
            Err(err) => {
                warn!("D-Bus ScreenSaver signal error: {err}");
                continue;
            }
        };
        if let Ok(active) = msg.body().deserialize::<bool>() {
            let ev = if active {
                SessionEvent::ScreenLocked
            } else {
                SessionEvent::ScreenUnlocked
            };
            debug!("ScreenSaver ActiveChanged={active}");
            if tx.send_blocking(ev).is_err() {
                break;
            }
        }
    }
    Ok(())
}

fn watch_login1_signals(tx: &Sender<SessionEvent>) -> zbus::Result<()> {
    let conn = Connection::system()?;

    let sleep_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.login1")?
        .interface("org.freedesktop.login1.Manager")?
        .member("PrepareForSleep")?
        .build();

    let session_path = resolve_current_session_path(&conn);

    // Build combined match rules on the system connection by registering both rules
    // via MessageIterator (or watching Manager + Session on the same connection).
    let dbus_proxy = zbus::blocking::fdo::DBusProxy::new(&conn)?;
    dbus_proxy.add_match_rule(sleep_rule)?;

    if let Some(ref path) = session_path {
        let lock_rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.login1")?
            .path(path.as_str())?
            .interface("org.freedesktop.login1.Session")?
            .member("Lock")?
            .build();
        let unlock_rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.login1")?
            .path(path.as_str())?
            .interface("org.freedesktop.login1.Session")?
            .member("Unlock")?
            .build();
        let _ = dbus_proxy.add_match_rule(lock_rule);
        let _ = dbus_proxy.add_match_rule(unlock_rule);
        info!("listening for login1 Session Lock/Unlock on {}", path.as_str());
    }

    let iter = MessageIterator::from(&conn);
    for msg_res in iter {
        let msg = match msg_res {
            Ok(m) => m,
            Err(err) => {
                warn!("login1 D-Bus signal error: {err}");
                continue;
            }
        };
        let header = msg.header();
        let Some(iface) = header.interface() else {
            continue;
        };
        let Some(member) = header.member() else {
            continue;
        };

        if iface.as_str() == "org.freedesktop.login1.Manager"
            && member.as_str() == "PrepareForSleep"
            && let Ok(going_to_sleep) = msg.body().deserialize::<bool>()
        {
            let ev = if going_to_sleep {
                SessionEvent::Sleeping
            } else {
                SessionEvent::WokeUp
            };
            debug!("login1 PrepareForSleep={going_to_sleep}");
            if tx.send_blocking(ev).is_err() {
                break;
            }
        } else if iface.as_str() == "org.freedesktop.login1.Session" {
            let ev = match member.as_str() {
                "Lock" => Some(SessionEvent::ScreenLocked),
                "Unlock" => Some(SessionEvent::ScreenUnlocked),
                _ => None,
            };
            if let Some(ev) = ev {
                debug!("login1 Session {member}");
                if tx.send_blocking(ev).is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn resolve_current_session_path(conn: &Connection) -> Option<OwnedObjectPath> {
    let manager = Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .ok()?;

    if let Ok(session_id) = std::env::var("XDG_SESSION_ID")
        && !session_id.is_empty()
        && let Ok(path) = manager.call::<_, _, OwnedObjectPath>("GetSession", &(session_id.as_str(),))
    {
        return Some(path);
    }

    manager
        .call::<_, _, OwnedObjectPath>("GetSessionByPID", &(std::process::id(),))
        .ok()
}
