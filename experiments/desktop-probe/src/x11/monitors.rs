use std::cmp::{max, min};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrExt;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as XprotoExt, Window};

#[derive(Debug, Clone)]
pub struct MonitorGeometry {
    pub name: String,
    pub is_primary: bool,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub struct DesktopLayout {
    pub primary_monitor: Option<MonitorGeometry>,
    pub current_desktop: u32,
    pub desktop_workarea: Option<Rect>,
    pub usable_area: Rect,
}

pub fn query_desktop_layout(
    conn: &impl Connection,
    root: Window,
    fallback_width: u16,
    fallback_height: u16,
) -> DesktopLayout {
    let monitors = query_monitors(conn, root);
    let mut primary = monitors.iter().find(|m| m.is_primary).cloned();
    if primary.is_none() && !monitors.is_empty() {
        primary = Some(monitors[0].clone());
    }

    let current_desktop = query_current_desktop(conn, root);
    let desktop_workarea = query_desktop_workarea(conn, root, current_desktop);

    // Compute usable area
    let usable_area = match (&primary, desktop_workarea) {
        (Some(pm), Some(wa)) => {
            let eff_x = max(pm.x as i32, wa.x);
            let eff_y = max(pm.y as i32, wa.y);
            let eff_right = min((pm.x as i32) + (pm.width as i32), wa.x + (wa.width as i32));
            let eff_bottom = min(
                (pm.y as i32) + (pm.height as i32),
                wa.y + (wa.height as i32),
            );

            if eff_right > eff_x && eff_bottom > eff_y {
                Rect {
                    x: eff_x,
                    y: eff_y,
                    width: (eff_right - eff_x) as u32,
                    height: (eff_bottom - eff_y) as u32,
                }
            } else {
                Rect {
                    x: pm.x as i32,
                    y: pm.y as i32,
                    width: pm.width as u32,
                    height: pm.height as u32,
                }
            }
        }
        (Some(pm), None) => Rect {
            x: pm.x as i32,
            y: pm.y as i32,
            width: pm.width as u32,
            height: pm.height as u32,
        },
        (None, Some(wa)) => wa,
        (None, None) => Rect {
            x: 0,
            y: 0,
            width: fallback_width as u32,
            height: fallback_height as u32,
        },
    };

    DesktopLayout {
        primary_monitor: primary,
        current_desktop,
        desktop_workarea,
        usable_area,
    }
}

pub fn query_monitors(conn: &impl Connection, root: Window) -> Vec<MonitorGeometry> {
    let mut monitors = Vec::new();
    if let Ok(cookie) = conn.randr_get_monitors(root, true) {
        if let Ok(reply) = cookie.reply() {
            for mon in reply.monitors {
                let name = if let Ok(atom_name) = conn.get_atom_name(mon.name) {
                    if let Ok(name_reply) = atom_name.reply() {
                        String::from_utf8_lossy(&name_reply.name).into_owned()
                    } else {
                        format!("0x{:x}", mon.name)
                    }
                } else {
                    format!("0x{:x}", mon.name)
                };

                monitors.push(MonitorGeometry {
                    name,
                    is_primary: mon.primary,
                    x: mon.x,
                    y: mon.y,
                    width: mon.width,
                    height: mon.height,
                });
            }
        }
    }
    monitors
}

pub fn query_current_desktop(conn: &impl Connection, root: Window) -> u32 {
    let mut current_desktop = 0;
    if let Ok(atom_current) = conn.intern_atom(false, b"_NET_CURRENT_DESKTOP") {
        if let Ok(reply) = atom_current.reply() {
            if let Ok(prop) = conn.get_property(false, root, reply.atom, AtomEnum::CARDINAL, 0, 1) {
                if let Ok(prop_reply) = prop.reply() {
                    if let Some(val) = prop_reply.value32().and_then(|mut iter| iter.next()) {
                        current_desktop = val;
                    }
                }
            }
        }
    }
    current_desktop
}

pub fn query_desktop_workarea(
    conn: &impl Connection,
    root: Window,
    current_desktop: u32,
) -> Option<Rect> {
    let atom_workarea = conn
        .intern_atom(false, b"_NET_WORKAREA")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let prop = conn
        .get_property(false, root, atom_workarea, AtomEnum::CARDINAL, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    let values: Vec<u32> = prop.value32()?.collect();

    let offset = (current_desktop as usize) * 4;
    if offset + 3 < values.len() {
        let x = values[offset] as i32;
        let y = values[offset + 1] as i32;
        let width = values[offset + 2];
        let height = values[offset + 3];
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    } else {
        None
    }
}
