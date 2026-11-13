use tracing::{debug, error, warn};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

/// Locks the user's screen using `org.freedesktop.ScreenSaver.Lock()` first,
/// falling back to `org.freedesktop.login1.Manager.LockSession` for the current session.
pub fn lock_screen_async() {
    std::thread::Builder::new()
        .name("safeeyes-lock".to_string())
        .spawn(|| {
            if let Err(err) = lock_screen_blocking() {
                error!("failed to lock screen: {err}");
            }
        })
        .ok();
}

pub fn lock_screen_blocking() -> Result<(), String> {
    if try_freedesktop_screensaver().is_ok() {
        debug!("locked screen via org.freedesktop.ScreenSaver");
        return Ok(());
    }

    if try_login1_lock().is_ok() {
        debug!("locked screen via org.freedesktop.login1");
        return Ok(());
    }

    Err("neither org.freedesktop.ScreenSaver nor org.freedesktop.login1 succeeded".to_string())
}

fn try_freedesktop_screensaver() -> zbus::Result<()> {
    let conn = Connection::session()?;
    for path in ["/org/freedesktop/ScreenSaver", "/ScreenSaver"] {
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.ScreenSaver",
            path,
            "org.freedesktop.ScreenSaver",
        )?;
        match proxy.call_method("Lock", &()) {
            Ok(_) => return Ok(()),
            Err(err) => {
                warn!("ScreenSaver.Lock at {path} failed: {err}");
            }
        }
    }
    Err(zbus::Error::Failure(
        "ScreenSaver.Lock failed on known paths".to_string(),
    ))
}

fn try_login1_lock() -> zbus::Result<()> {
    let conn = Connection::system()?;
    let manager = Proxy::new(
        &conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )?;

    if let Ok(session_id) = std::env::var("XDG_SESSION_ID")
        && !session_id.is_empty()
        && manager.call_method("LockSession", &(session_id.as_str(),)).is_ok()
    {
        return Ok(());
    }

    let pid = std::process::id();
    let session_path: OwnedObjectPath = manager.call("GetSessionByPID", &(pid,))?;
    let session_proxy = Proxy::new(
        &conn,
        "org.freedesktop.login1",
        session_path.as_str(),
        "org.freedesktop.login1.Session",
    )?;
    let session_id: String = session_proxy.get_property("Id")?;
    manager.call_method("LockSession", &(session_id.as_str(),))?;
    Ok(())
}
