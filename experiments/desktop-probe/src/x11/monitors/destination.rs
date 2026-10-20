//! Read-only evidence for an already frozen destination, isolated from M03.2 layout policy.
//! Two complete reads establish agreement, never atomicity or actual body placement.
#![allow(dead_code)] // Stage B has no execution/event-loop consumer yet.

use std::fmt;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, GetPropertyReply, Window};

use super::{cardinal_values, intersect_workarea, query_monitors, LayoutAtoms, MonitorGeometry};
use crate::control::{DestinationIdentity, DestinationViewport};
use crate::geometry::{compute_valid_origin_bounds, Rect, Size, ValidOriginBounds};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationError {
    Unverifiable(String),
    UnsupportedViewport,
    Changed,
}

impl fmt::Display for DestinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unverifiable(message) => write!(f, "Unverifiable destination layout: {message}"),
            Self::UnsupportedViewport => {
                write!(f, "Nonzero desktop viewport unsupported for M03.3")
            }
            Self::Changed => write!(
                f,
                "Destination layout changed or frozen destination is stale"
            ),
        }
    }
}

impl std::error::Error for DestinationError {}

fn unverifiable(error: impl fmt::Display) -> DestinationError {
    DestinationError::Unverifiable(error.to_string())
}

/// Caller identity only. Construction requires two strict agreeing snapshots;
/// no monitor, workarea, body or visibility authority is captured here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallerDesktop {
    desktop_count: u32,
    current_desktop: u32,
}

impl CallerDesktop {
    pub fn frozen_destination(self) -> u32 {
        self.current_desktop
    }

    /// Compare a fresh two-read capture without modifying the original identity.
    pub fn revalidate(self, fresh: Self) -> Result<(), DestinationError> {
        if self == fresh {
            Ok(())
        } else {
            Err(DestinationError::Changed)
        }
    }
}

/// Intern only the two identity atoms; caller capture never subscribes to events
/// or invokes the owner's full destination/monitor reader. Guarded until D4.
pub struct CallerDesktopAtoms {
    count: Atom,
    current: Atom,
}

impl CallerDesktopAtoms {
    pub fn intern(conn: &impl Connection) -> Result<Self, DestinationError> {
        let atom = |name: &[u8]| {
            conn.intern_atom(false, name)
                .map_err(unverifiable)?
                .reply()
                .map(|reply| reply.atom)
                .map_err(unverifiable)
        };
        Ok(Self {
            count: atom(b"_NET_NUMBER_OF_DESKTOPS")?,
            current: atom(b"_NET_CURRENT_DESKTOP")?,
        })
    }

    pub fn capture(
        &self,
        conn: &impl Connection,
        root: Window,
    ) -> Result<CallerDesktop, DestinationError> {
        capture_caller_with(|| {
            let property = |atom| {
                conn.get_property(false, root, atom, AtomEnum::ANY, 0, 1)
                    .map_err(unverifiable)?
                    .reply()
                    .map_err(unverifiable)
            };
            // Complete count/current pairs, including before metadata validation.
            Ok((property(self.count)?, property(self.current)?))
        })
    }

    /// D4's pre-send seam: failure never replaces the frozen destination.
    pub fn revalidate(
        &self,
        conn: &impl Connection,
        root: Window,
        captured: CallerDesktop,
    ) -> Result<(), DestinationError> {
        captured.revalidate(self.capture(conn, root)?)
    }
}

pub(crate) fn capture_caller_with(
    mut read: impl FnMut() -> Result<(GetPropertyReply, GetPropertyReply), DestinationError>,
) -> Result<CallerDesktop, DestinationError> {
    let before = read()?;
    let after = read()?;
    let validate = |(count, current): &(GetPropertyReply, GetPropertyReply)| {
        let desktop_count = parse_count(count)?;
        let current_desktop = parse_current(current, desktop_count)?;
        Ok::<_, DestinationError>(CallerDesktop {
            desktop_count,
            current_desktop,
        })
    };
    let before = validate(&before)?;
    let after = validate(&after)?;
    before.revalidate(after)?;
    Ok(before)
}

/// Names, output lists, primary flags and unrelated monitors are not placement identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetainedMonitor {
    pub name_atom: Atom,
    pub bounds: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportEvidence {
    Absent,
    /// The complete array was present and every coordinate on every desktop was zero.
    AllZero,
}

/// Checked capacity and freshness evidence, not proof of the body's actual origin/membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationLayout {
    pub desktop_count: u32,
    pub destination_desktop: u32,
    pub current_desktop: u32,
    pub monitor: RetainedMonitor,
    pub target_workarea: Option<Rect>,
    pub viewport: ViewportEvidence,
    pub usable_area: Rect,
    pub body_size: Size,
    pub valid_origins: ValidOriginBounds,
}

impl From<&DestinationLayout> for DestinationIdentity {
    fn from(layout: &DestinationLayout) -> Self {
        Self {
            desktop_count: layout.desktop_count,
            destination_desktop: layout.destination_desktop,
            current_desktop: layout.current_desktop,
            monitor_name: layout.monitor.name_atom,
            monitor_bounds: layout.monitor.bounds,
            target_workarea: layout.target_workarea,
            viewport: match layout.viewport {
                ViewportEvidence::Absent => DestinationViewport::Absent,
                ViewportEvidence::AllZero => DestinationViewport::AllZero,
            },
            usable_area: layout.usable_area,
            body_size: layout.body_size,
            valid_origins: layout.valid_origins,
        }
    }
}

/// Raw complete replies keep malformed metadata distinct from genuine absence.
#[derive(Debug, Clone)]
struct DestinationSnapshot {
    count: GetPropertyReply,
    current: GetPropertyReply,
    workarea: GetPropertyReply,
    viewport: GetPropertyReply,
    monitors: Vec<MonitorGeometry>,
}

/// No subscriptions, requests to the WM, body reads or mutations. The supplied target
/// is never captured/replaced here. Later operations must call afresh around mutation.
pub fn query_destination_layout(
    conn: &impl Connection,
    root: Window,
    atoms: &LayoutAtoms,
    selected: Atom,
    frozen_destination: u32,
    body_size: Size,
) -> Result<DestinationLayout, DestinationError> {
    query_with(selected, frozen_destination, body_size, || {
        let property = |atom, words| -> Result<GetPropertyReply, DestinationError> {
            conn.get_property(false, root, atom, AtomEnum::ANY, 0, words)
                .map_err(unverifiable)?
                .reply()
                .map_err(unverifiable)
        };
        let count = property(atoms.number_of_desktops, 1)?;
        // Exact count-derived read bounds, without an arbitrary maximum desktop count.
        // Oversized/incomplete replies are rejected, never paged or retried.
        let desktop_count = parse_count(&count)?;
        let workarea_words = array_words(desktop_count, 4)?;
        let viewport_words = array_words(desktop_count, 2)?;
        Ok(DestinationSnapshot {
            count,
            current: property(atoms.current_desktop, 1)?,
            workarea: property(atoms.workarea, workarea_words)?,
            viewport: property(atoms.desktop_viewport, viewport_words)?,
            monitors: query_monitors(conn, root).map_err(unverifiable)?,
        })
    })
}

fn query_with(
    selected: Atom,
    frozen_destination: u32,
    body_size: Size,
    mut read: impl FnMut() -> Result<DestinationSnapshot, DestinationError>,
) -> Result<DestinationLayout, DestinationError> {
    let before = read()?;
    let after = read()?;
    let before = validate_snapshot(&before, selected, frozen_destination, body_size)?;
    let after = validate_snapshot(&after, selected, frozen_destination, body_size)?;
    if before != after {
        return Err(DestinationError::Changed);
    }
    Ok(after)
}

fn words(property: &GetPropertyReply) -> Result<Option<Vec<u32>>, DestinationError> {
    cardinal_values(property).map_err(unverifiable)
}

fn one_word(property: &GetPropertyReply) -> Result<u32, DestinationError> {
    match words(property)? {
        Some(values) if values.len() == 1 => Ok(values[0]),
        _ => Err(unverifiable("Expected exactly one present CARDINAL")),
    }
}

fn parse_count(property: &GetPropertyReply) -> Result<u32, DestinationError> {
    let count = one_word(property)?;
    if count == 0 {
        return Err(unverifiable("_NET_NUMBER_OF_DESKTOPS must be positive"));
    }
    Ok(count)
}

fn parse_current(property: &GetPropertyReply, count: u32) -> Result<u32, DestinationError> {
    let current = one_word(property)?;
    if current == u32::MAX || current >= count {
        return Err(unverifiable("Invalid current desktop"));
    }
    Ok(current)
}

/// Width is in 32-bit property words. Both the X11 length and host byte size must fit.
fn array_words(count: u32, width: u32) -> Result<u32, DestinationError> {
    let length = count
        .checked_mul(width)
        .ok_or_else(|| unverifiable("Per-desktop property word count overflow"))?;
    usize::try_from(length)
        .ok()
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| unverifiable("Per-desktop property byte count overflow"))?;
    Ok(length)
}

fn complete_array(
    property: &GetPropertyReply,
    count: u32,
    width: u32,
) -> Result<Option<Vec<u32>>, DestinationError> {
    let expected = array_words(count, width)?;
    let values = words(property)?;
    if values
        .as_ref()
        .is_some_and(|v| u32::try_from(v.len()) != Ok(expected))
    {
        return Err(unverifiable("Incomplete or extra per-desktop entries"));
    }
    Ok(values)
}

fn target_workarea(
    property: &GetPropertyReply,
    count: u32,
    target: u32,
) -> Result<Option<Rect>, DestinationError> {
    let Some(values) = complete_array(property, count, 4)? else {
        return Ok(None);
    };
    let mut selected = None;
    for (desktop, entry) in values.as_chunks::<4>().0.iter().enumerate() {
        // Preserve the project's signed root-coordinate convention: CARDINAL origins
        // carry the exact two's-complement i32 bit pattern; dimensions stay unsigned.
        let area = Rect::new(
            i32::from_ne_bytes(entry[0].to_ne_bytes()),
            i32::from_ne_bytes(entry[1].to_ne_bytes()),
            entry[2],
            entry[3],
        );
        if area.width == 0 || area.height == 0 {
            return Err(unverifiable("Empty destination workarea"));
        }
        area.right().map_err(unverifiable)?;
        area.bottom().map_err(unverifiable)?;
        if u32::try_from(desktop) == Ok(target) {
            selected = Some(area);
        }
    }
    selected
        .map(Some)
        .ok_or_else(|| unverifiable("Missing frozen destination workarea"))
}

fn viewport(property: &GetPropertyReply, count: u32) -> Result<ViewportEvidence, DestinationError> {
    match complete_array(property, count, 2)? {
        None => Ok(ViewportEvidence::Absent),
        Some(values) if values.iter().all(|&coordinate| coordinate == 0) => {
            Ok(ViewportEvidence::AllZero)
        }
        Some(_) => Err(DestinationError::UnsupportedViewport),
    }
}

fn validate_snapshot(
    snapshot: &DestinationSnapshot,
    selected: Atom,
    target: u32,
    body_size: Size,
) -> Result<DestinationLayout, DestinationError> {
    let desktop_count = parse_count(&snapshot.count)?;
    if target == u32::MAX || target >= desktop_count {
        return Err(unverifiable("Invalid frozen destination desktop"));
    }
    let current_desktop = parse_current(&snapshot.current, desktop_count)?;
    if current_desktop != target {
        return Err(DestinationError::Changed);
    }
    let target_workarea = target_workarea(&snapshot.workarea, desktop_count, target)?;
    let viewport = viewport(&snapshot.viewport, desktop_count)?;
    let mut retained = snapshot.monitors.iter().filter(|m| m.name_atom == selected);
    let monitor = retained
        .next()
        .ok_or_else(|| unverifiable("Retained monitor disappeared; migration unsupported"))?;
    if retained.next().is_some() {
        return Err(unverifiable("Ambiguous retained monitor identity"));
    }
    if monitor.width == 0 || monitor.height == 0 {
        return Err(unverifiable("Empty retained monitor"));
    }
    let bounds = Rect::new(
        i32::from(monitor.x),
        i32::from(monitor.y),
        u32::from(monitor.width),
        u32::from(monitor.height),
    );
    bounds.right().map_err(unverifiable)?;
    bounds.bottom().map_err(unverifiable)?;
    let usable_area = intersect_workarea(monitor, target_workarea).map_err(unverifiable)?;
    if body_size.width == 0 || body_size.height == 0 {
        return Err(unverifiable("Empty fixed body size"));
    }
    let valid_origins =
        compute_valid_origin_bounds(usable_area, body_size).map_err(unverifiable)?;
    Ok(DestinationLayout {
        desktop_count,
        destination_desktop: target,
        current_desktop,
        monitor: RetainedMonitor {
            name_atom: selected,
            bounds,
        },
        target_workarea,
        viewport,
        usable_area,
        body_size,
        valid_origins,
    })
}

#[cfg(test)]
mod tests;
