use crate::geometry::Rect;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

fn stream(path: impl AsRef<std::path::Path>) -> Result<UnixStream> {
    let socket = UnixStream::connect(path).context("Connect to compositor IPC")?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(socket)
}

fn sway_request(kind: u32, payload: &str) -> Result<Value> {
    let mut socket = stream(std::env::var_os("SWAYSOCK").context("SWAYSOCK is not set")?)?;
    let mut request = b"i3-ipc".to_vec();
    request.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    request.extend_from_slice(&kind.to_le_bytes());
    request.extend_from_slice(payload.as_bytes());
    socket.write_all(&request)?;
    let mut header = [0u8; 14];
    socket.read_exact(&mut header)?;
    ensure!(&header[..6] == b"i3-ipc", "Invalid Sway IPC response");
    let length = u32::from_le_bytes(header[6..10].try_into()?) as usize;
    ensure!(length <= 32 * 1024 * 1024, "Sway response too large");
    let mut body = vec![0u8; length];
    socket.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn rect(value: &Value) -> Result<Rect> {
    Ok(Rect {
        x: number(value, "x")?,
        y: number(value, "y")?,
        width: number(value, "width")?,
        height: number(value, "height")?,
    })
}

fn number(value: &Value, key: &str) -> Result<i32> {
    Ok(i32::try_from(value[key].as_i64().with_context(|| {
        format!("Missing compositor field {key}")
    })?)?)
}

/// Carry the containing workspace's usable rectangle down to the focused leaf.
fn focused(node: &Value, area: Option<Rect>) -> Option<(&Value, Rect)> {
    let area = if node["type"] == "workspace" {
        rect(&node["rect"]).ok().or(area)
    } else {
        area
    };
    if node["focused"] == true && (node["app_id"].is_string() || node["window"].is_number()) {
        return area.map(|r| (node, r));
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node[key].as_array() {
            for child in children {
                if let Some(found) = focused(child, area) {
                    return Some(found);
                }
            }
        }
    }
    None
}

pub fn sway(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Rect) -> Result<Rect> {
    let tree = sway_request(4, "")?;
    let (window, area) = focused(&tree, None).context("No focused Sway window/workspace")?;
    let id = window["id"].as_u64().context("Missing Sway container id")?;
    let identity = format!(
        "sway:{}:{id}",
        std::env::var("SWAYSOCK").unwrap_or_default()
    );
    let target = resolve(&identity, area, rect(&window["rect"])?);
    let commands = format!(
        "[con_id={id}] fullscreen disable; [con_id={id}] floating enable; [con_id={id}] resize set width {} px height {} px; [con_id={id}] move absolute position {} px {} px",
        target.width, target.height, target.x, target.y
    );
    let response = sway_request(0, &commands)?;
    let replies = response
        .as_array()
        .context("Invalid Sway command response")?;
    for reply in replies {
        ensure!(reply["success"] == true, "Sway rejected action: {reply}");
    }
    Ok(target)
}

fn hypr_request(command: &str) -> Result<String> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?);
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
    ensure!(
        !signature.contains(['/', '\\']),
        "Invalid Hyprland instance signature"
    );
    let mut socket = stream(runtime.join("hypr").join(signature).join(".socket.sock"))?;
    socket.write_all(command.as_bytes())?;
    let mut response = String::new();
    socket
        .take(32 * 1024 * 1024)
        .read_to_string(&mut response)?;
    Ok(response)
}

fn dispatch(command: &str) -> Result<()> {
    let reply = hypr_request(&format!("dispatch {command}"))?;
    ensure!(reply.trim() == "ok", "Hyprland rejected action: {reply}");
    Ok(())
}

pub fn hyprland(resolve: &mut dyn FnMut(&str, Rect, Rect) -> Rect) -> Result<Rect> {
    let window: Value = serde_json::from_str(&hypr_request("j/activewindow")?)?;
    let address = window["address"]
        .as_str()
        .context("No focused Hyprland window")?;
    ensure!(
        address.starts_with("0x") && address[2..].chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid window address"
    );
    let monitors: Value = serde_json::from_str(&hypr_request("j/monitors")?)?;
    let monitor = monitors
        .as_array()
        .context("Invalid monitor list")?
        .iter()
        .find(|m| m["id"] == window["monitor"])
        .context("Focused window's monitor was not found")?;
    let scale = monitor["scale"].as_f64().context("Missing monitor scale")?;
    ensure!(scale > 0.0, "Invalid monitor scale");
    let mut width = number(monitor, "width")?;
    let mut height = number(monitor, "height")?;
    if monitor["transform"].as_i64().unwrap_or(0) % 2 == 1 {
        std::mem::swap(&mut width, &mut height);
    }
    let reserved = &monitor["reserved"];
    let margin = |i: usize| reserved[i].as_i64().unwrap_or(0) as i32;
    let area = Rect {
        x: number(monitor, "x")? + margin(0),
        y: number(monitor, "y")? + margin(1),
        width: (width as f64 / scale).round() as i32 - margin(0) - margin(2),
        height: (height as f64 / scale).round() as i32 - margin(1) - margin(3),
    };
    let current = Rect {
        x: window["at"][0].as_i64().context("Missing window x")? as i32,
        y: window["at"][1].as_i64().context("Missing window y")? as i32,
        width: window["size"][0].as_i64().context("Missing window width")? as i32,
        height: window["size"][1]
            .as_i64()
            .context("Missing window height")? as i32,
    };
    let id = format!(
        "hyprland:{}:{address}",
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default()
    );
    let target = resolve(&id, area, current);
    // Pin every command to the original window to avoid moving a newly focused app.
    if window["fullscreen"].as_i64().unwrap_or(0) != 0 {
        anyhow::bail!("Exit fullscreen before positioning this Hyprland window");
    }
    dispatch(&format!("setfloating address:{address}"))?;
    dispatch(&format!(
        "resizewindowpixel exact {} {},address:{address}",
        target.width, target.height
    ))?;
    dispatch(&format!(
        "movewindowpixel exact {} {},address:{address}",
        target.x, target.y
    ))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sway_finds_focused_floating_window_on_secondary_workspace() {
        let tree = serde_json::json!({"type":"root","nodes":[{"type":"workspace", "rect":{"x":-1920,"y":30,"width":1920,"height":1050},"floating_nodes":[{"id":42,"focused":true,"app_id":"test"}]}]});
        let (node, area) = focused(&tree, None).unwrap();
        assert_eq!(node["id"], 42);
        assert_eq!(area.x, -1920);
        assert_eq!(area.y, 30);
    }
}
