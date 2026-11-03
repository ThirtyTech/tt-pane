//! GNOME on Wayland: the bundled Shell extension owns window control and is
//! reached over the session bus (see packaging/gnome).

use crate::geometry::Rect;
use anyhow::{Context, Result, bail};

const BUS_NAME: &str = "net.thirtytech.TtPane";
const OBJECT_PATH: &str = "/net/thirtytech/TtPane";
const INTERFACE: &str = "net.thirtytech.TtPane";

const MISSING: &str = "The tt-pane GNOME Shell extension is not running. Open tt-pane Settings (or run `tt-pane gnome setup`), then log out and back in so GNOME loads it.";

/// GNOME reports itself as e.g. "GNOME" or "ubuntu:GNOME".
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktop| desktop.split(':').any(|d| d.eq_ignore_ascii_case("GNOME")))
}

type Wire = (i32, i32, i32, i32);

fn rect((x, y, width, height): Wire) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn call<B>(connection: &zbus::blocking::Connection, method: &str, body: &B) -> Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    connection
        .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), method, body)
        .map_err(|error| match &error {
            zbus::Error::MethodError(name, _, _)
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.ServiceUnknown"
                        | "org.freedesktop.DBus.Error.NameHasNoOwner"
                        | "org.freedesktop.DBus.Error.UnknownObject"
                ) =>
            {
                anyhow::anyhow!(MISSING)
            }
            // The extension's thrown errors arrive as the method error's message.
            zbus::Error::MethodError(_, Some(message), _) => anyhow::anyhow!("{message}"),
            _ => anyhow::Error::new(error).context("Call the tt-pane GNOME Shell extension"),
        })
}

pub fn apply(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Result<Rect>) -> Result<Rect> {
    let connection =
        zbus::blocking::Connection::session().context("Connect to the D-Bus session bus")?;
    let reply = call(&connection, "GetFocusedWindow", &())?;
    let (window, frame, area, fullscreen): (u64, Wire, Wire, bool) = reply
        .body()
        .deserialize()
        .context("Unexpected reply from the tt-pane GNOME Shell extension")?;
    if fullscreen {
        bail!("Exit fullscreen before positioning this window");
    }
    let target = resolve(&format!("gnome:{window}"), rect(area), rect(frame))?;
    // Pin the move to the window captured above, even if focus changed meanwhile.
    call(
        &connection,
        "MoveResize",
        &(window, target.x, target.y, target.width, target.height),
    )?;
    Ok(target)
}
