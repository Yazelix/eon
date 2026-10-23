//! The one-time desktop notice that a detached Eon window keeps running
//! (EON-C8). It is a freedesktop notification, so only Linux sends it; on
//! macOS the native route needs the app bundle's notification permission and
//! belongs with that bundle's owner.

/// Tells this user, once, that Eon still runs after its window detached. A
/// marker in Eon's state directory claims the notice before sending; a failed
/// send removes it so a later detach tries again. The send runs on its own
/// thread, so detach never waits on the desktop.
#[cfg(target_os = "linux")]
pub(super) fn notify_first_detach(application_id: &str) {
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        thread,
    };

    let Some(state) = super::diagnostics::state_directory() else {
        return;
    };
    let application_id = application_id.to_owned();
    // A refused thread only loses the notice.
    let _ = thread::Builder::new()
        .name("eon-detach-notice".into())
        .spawn(move || {
            let marker = state.join("first-detach-notified");
            let claimed = fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&state)
                .and_then(|()| {
                    fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&marker)
                });
            if claimed.is_ok() && send(&application_id).is_err() {
                let _ = fs::remove_file(&marker);
            }
        });
}

#[cfg(not(target_os = "linux"))]
pub(super) fn notify_first_detach(_application_id: &str) {}

/// One notification with no actions; Eon raises no window for it, and the
/// `desktop-entry` hint only names Eon's icon and launcher to the server. A
/// server that never answers fails after D-Bus's usual 25-second call timeout,
/// so it cannot keep the marker claimed.
#[cfg(target_os = "linux")]
fn send(application_id: &str) -> zbus::Result<()> {
    use std::{collections::HashMap, time::Duration};
    use zbus::zvariant::Value;

    let hints = HashMap::from([("desktop-entry", Value::from(application_id))]);
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(25))
        .build()?;
    connection.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "Eon",
            0u32,
            "",
            "Eon is still running in the background",
            "Open Eon to return; close the last tab or use Commands > Quit this window to stop it.",
            Vec::<&str>::new(),
            hints,
            -1i32,
        ),
    )?;
    Ok(())
}
