use x11rb::connection::Connection;
use x11rb::protocol::randr::{ConnectionExt as RandrExt, NotifyMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as XprotoExt, EventMask,
    GetPropertyReply, Window,
};

pub use crate::geometry::Rect;
type HostError = Box<dyn std::error::Error>;

#[derive(Debug, Clone)]
pub struct MonitorGeometry {
    pub name: String,
    pub name_atom: Atom,
    pub outputs: Vec<u32>,
    pub is_primary: bool,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

pub struct DesktopLayout {
    // Kept for the diagnostic display; refresh selects by name_atom, never by primary status.
    pub primary_monitor: Option<MonitorGeometry>,
    pub current_desktop: u32,
    pub desktop_workarea: Option<Rect>,
    pub usable_area: Rect,
}

pub struct LayoutAtoms {
    pub workarea: Atom,
    pub current_desktop: Atom,
    pub wm_desktop: Atom,
}
impl LayoutAtoms {
    pub fn subscribe(conn: &impl Connection, root: Window) -> Result<Self, HostError> {
        let version = conn.randr_query_version(1, 5)?.reply()?;
        if (version.major_version, version.minor_version) < (1, 5) {
            return Err("RandR 1.5 monitor identities are required".into());
        }
        conn.randr_select_input(
            root,
            NotifyMask::SCREEN_CHANGE
                | NotifyMask::CRTC_CHANGE
                | NotifyMask::OUTPUT_CHANGE
                | NotifyMask::OUTPUT_PROPERTY
                | NotifyMask::RESOURCE_CHANGE,
        )?
        .check()?;
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )?
        .check()?;
        Ok(Self {
            workarea: conn.intern_atom(false, b"_NET_WORKAREA")?.reply()?.atom,
            current_desktop: conn
                .intern_atom(false, b"_NET_CURRENT_DESKTOP")?
                .reply()?
                .atom,
            wm_desktop: conn.intern_atom(false, b"_NET_WM_DESKTOP")?.reply()?.atom,
        })
    }
}

pub fn query_desktop_layout(
    conn: &impl Connection,
    root: Window,
    atoms: &LayoutAtoms,
    selected: Option<Atom>,
) -> Result<DesktopLayout, HostError> {
    let monitors =
        query_monitors(conn, root).map_err(|e| format!("RandR monitor query failed: {e}"))?;
    let monitor = select_monitor(&monitors, selected)?;
    let desktop = cardinal_values(
        &conn
            .get_property(false, root, atoms.current_desktop, AtomEnum::ANY, 0, 1)?
            .reply()?,
    )?;
    let current_desktop = match desktop {
        None => {
            eprintln!("[WARN] _NET_CURRENT_DESKTOP absent; using desktop 0");
            0
        }
        Some(values) if values.len() == 1 => values[0],
        _ => return Err("Malformed _NET_CURRENT_DESKTOP".into()),
    };
    let property = conn
        .get_property(false, root, atoms.workarea, AtomEnum::ANY, 0, 65536)?
        .reply()?;
    let desktop_workarea = parse_workarea(&property, current_desktop)?;
    if desktop_workarea.is_none() {
        eprintln!("[WARN] _NET_WORKAREA absent; using selected-monitor bounds");
    }
    let usable_area = intersect_workarea(&monitor, desktop_workarea)?;
    Ok(DesktopLayout {
        primary_monitor: Some(monitor),
        current_desktop,
        desktop_workarea,
        usable_area,
    })
}

fn select_monitor(
    monitors: &[MonitorGeometry],
    selected: Option<Atom>,
) -> Result<MonitorGeometry, HostError> {
    if let Some(atom) = selected {
        monitors.iter().find(|m| m.name_atom == atom).cloned().ok_or_else(|| format!("Selected monitor atom 0x{atom:x} disappeared; automatic migration is unsupported").into())
    } else {
        monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| monitors.first())
            .cloned()
            .ok_or_else(|| "RandR returned no active monitors; placement unsupported".into())
    }
}

fn intersect_workarea(
    monitor: &MonitorGeometry,
    workarea: Option<Rect>,
) -> Result<Rect, HostError> {
    let bounds = Rect::new(
        monitor.x.into(),
        monitor.y.into(),
        monitor.width.into(),
        monitor.height.into(),
    );
    let Some(area) = workarea else {
        return Ok(bounds);
    };
    let x = bounds.x.max(area.x);
    let y = bounds.y.max(area.y);
    let right = bounds.right()?.min(area.right()?);
    let bottom = bounds.bottom()?.min(area.bottom()?);
    if right <= x || bottom <= y {
        return Err("Selected monitor and workarea have an empty intersection".into());
    }
    Ok(Rect::new(x, y, (right - x) as u32, (bottom - y) as u32))
}

pub fn cardinal_values(property: &GetPropertyReply) -> Result<Option<Vec<u32>>, HostError> {
    if property.type_ == x11rb::NONE {
        return Ok(None);
    }
    if property.type_ != AtomEnum::CARDINAL.into()
        || property.format != 32
        || property.bytes_after != 0
    {
        return Err("Malformed or truncated CARDINAL property".into());
    }
    Ok(Some(
        property
            .value32()
            .ok_or("Invalid CARDINAL payload")?
            .collect(),
    ))
}

fn parse_workarea(property: &GetPropertyReply, desktop: u32) -> Result<Option<Rect>, HostError> {
    let Some(values) = cardinal_values(property)? else {
        return Ok(None);
    };
    let offset = usize::try_from(desktop)?
        .checked_mul(4)
        .ok_or("Workarea index overflow")?;
    if values.len() % 4 != 0 || offset.checked_add(4).is_none_or(|end| end > values.len()) {
        return Err("Malformed _NET_WORKAREA or missing current desktop entry".into());
    }
    let rect = Rect::new(
        values[offset] as i32,
        values[offset + 1] as i32,
        values[offset + 2],
        values[offset + 3],
    );
    if rect.width == 0 || rect.height == 0 {
        return Err("Empty _NET_WORKAREA".into());
    }
    rect.right()?;
    rect.bottom()?;
    Ok(Some(rect))
}

pub fn query_monitors(
    conn: &impl Connection,
    root: Window,
) -> Result<Vec<MonitorGeometry>, HostError> {
    let reply = conn.randr_get_monitors(root, true)?.reply()?;
    Ok(reply
        .monitors
        .into_iter()
        .map(|mon| {
            let name = conn
                .get_atom_name(mon.name)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
                .unwrap_or_else(|| format!("0x{:x}", mon.name));
            MonitorGeometry {
                name,
                name_atom: mon.name,
                outputs: mon.outputs,
                is_primary: mon.primary,
                x: mon.x,
                y: mon.y,
                width: mon.width,
                height: mon.height,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(atom: u32, primary: bool) -> MonitorGeometry {
        MonitorGeometry {
            name: format!("{atom}"),
            name_atom: atom,
            outputs: vec![atom + 10],
            is_primary: primary,
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        }
    }
    fn property(values: &[u32]) -> GetPropertyReply {
        GetPropertyReply {
            format: 32,
            sequence: 0,
            length: values.len() as u32,
            type_: AtomEnum::CARDINAL.into(),
            bytes_after: 0,
            value_len: values.len() as u32,
            value: values.iter().flat_map(|v| v.to_ne_bytes()).collect(),
        }
    }
    #[test]
    fn primary_change_preserves_identity_and_removal_is_an_error() {
        let monitors = vec![monitor(1, false), monitor(2, true)];
        assert_eq!(select_monitor(&monitors, Some(1)).unwrap().name_atom, 1);
        assert!(select_monitor(&monitors[1..], Some(1))
            .unwrap_err()
            .to_string()
            .contains("disappeared"));
    }
    #[test]
    fn absent_and_malformed_workarea_are_distinct() {
        let mut absent = property(&[]);
        absent.type_ = 0;
        absent.format = 0;
        assert_eq!(parse_workarea(&absent, 0).unwrap(), None);
        assert!(parse_workarea(&property(&[]), 0).is_err());
        assert!(parse_workarea(&property(&[0, 0, 100, 100]), 1).is_err());
        assert!(parse_workarea(&property(&[0, 0, 0, 100]), 0).is_err());
        let mut truncated = property(&[0, 0, 100, 100]);
        truncated.bytes_after = 4;
        assert!(parse_workarea(&truncated, 0).is_err());
        truncated.bytes_after = 0;
        truncated.type_ = AtomEnum::STRING.into();
        assert!(parse_workarea(&truncated, 0).is_err());
    }
    #[test]
    fn intersections_are_checked_and_never_expand_empty_or_invalid_fit() {
        let monitor = monitor(1, true);
        assert!(intersect_workarea(&monitor, Some(Rect::new(0, 0, 100, 100))).is_err());
        assert!(intersect_workarea(&monitor, Some(Rect::new(i32::MAX, 0, u32::MAX, 100))).is_err());
        let small = intersect_workarea(&monitor, Some(Rect::new(-100, 0, 100, 100))).unwrap();
        assert!(crate::geometry::compute_valid_origin_bounds(
            small,
            crate::geometry::Size::new(160, 160)
        )
        .is_err());
        assert_eq!(
            intersect_workarea(&monitor, Some(Rect::new(-1900, 48, 1900, 1032))).unwrap(),
            Rect::new(-1900, 48, 1900, 1032)
        );
    }
}
