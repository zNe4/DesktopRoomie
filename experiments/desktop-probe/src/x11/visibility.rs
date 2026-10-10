//! Stage D foundations only: observations are candidates, never operation confirmations.
//! No production command dispatch calls this module's mutation helpers.
#![allow(dead_code)]

use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, GetPropertyReply,
    MapState as NativeMapState, PropMode, Window,
};
use x11rb::protocol::ErrorKind;
use x11rb::wrapper::ConnectionExt as _;

use crate::control::{
    classify, Capability, Evidence, LayerObservation as LayerValue, MapState, VisibilityCandidate,
    VisibilityEvidence, WmState, Workspace,
};

type HostError = Box<dyn std::error::Error>;
const MAX_ATOMS: u32 = 4096;

pub struct VisibilityAtoms {
    pub wm_state: Atom,
    pub change_state: Atom,
    pub state: Atom,
    pub hidden: Atom,
    pub above: Atom,
    pub below: Atom,
    pub desktop: Atom,
    pub current_desktop: Atom,
    pub supported: Atom,
    pub user_time: Atom,
}

/// Complete decoded values retain unrelated state/support atoms for preservation checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub wm_state: WmState,
    pub map_state: Evidence<MapState>,
    pub state: Evidence<Vec<Atom>>,
    pub desktop: Evidence<u32>,
    pub current_desktop: Evidence<u32>,
    pub supported: Evidence<Vec<Atom>>,
}

#[derive(Debug)]
pub struct VisibilitySnapshot {
    pub observation: Option<Observation>,
    pub evidence: VisibilityEvidence,
    pub candidate: VisibilityCandidate,
    /// Unknown unless the complete state and all M03.1 layer capabilities are verifiable.
    pub layer: LayerValue,
}

impl VisibilityAtoms {
    pub fn intern(conn: &impl Connection) -> Result<Self, HostError> {
        let atom = |name: &[u8]| -> Result<Atom, HostError> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        Ok(Self {
            wm_state: atom(b"WM_STATE")?,
            change_state: atom(b"WM_CHANGE_STATE")?,
            state: atom(b"_NET_WM_STATE")?,
            hidden: atom(b"_NET_WM_STATE_HIDDEN")?,
            above: atom(b"_NET_WM_STATE_ABOVE")?,
            below: atom(b"_NET_WM_STATE_BELOW")?,
            desktop: atom(b"_NET_WM_DESKTOP")?,
            current_desktop: atom(b"_NET_CURRENT_DESKTOP")?,
            supported: atom(b"_NET_SUPPORTED")?,
            user_time: atom(b"_NET_WM_USER_TIME")?,
        })
    }

    pub fn observe(
        &self,
        conn: &impl Connection,
        root: Window,
        body: Window,
    ) -> Result<VisibilitySnapshot, HostError> {
        self.observe_with(body, || {
            let property = |window, atom, limit| -> Result<GetPropertyReply, HostError> {
                Ok(conn
                    .get_property(false, window, atom, AtomEnum::ANY, 0, limit)?
                    .reply()?)
            };
            Ok(Observation {
                wm_state: decode_wm_state(&property(body, self.wm_state, 2)?, self.wm_state),
                map_state: decode_map_state(conn.get_window_attributes(body)?.reply()?.map_state),
                state: decode_words(
                    &property(body, self.state, MAX_ATOMS)?,
                    AtomEnum::ATOM.into(),
                    MAX_ATOMS,
                ),
                desktop: decode_desktop(&property(body, self.desktop, 1)?, false),
                current_desktop: decode_desktop(&property(root, self.current_desktop, 1)?, true),
                supported: decode_words(
                    &property(root, self.supported, MAX_ATOMS)?,
                    AtomEnum::ATOM.into(),
                    MAX_ATOMS,
                ),
            })
        })
    }

    /// Two bounded reads check agreement, not atomicity. Call afresh after notifications
    /// and before future completion; no cache, polling, retry, or confirmation is implied.
    fn observe_with(
        &self,
        body: Window,
        mut read: impl FnMut() -> Result<Observation, HostError>,
    ) -> Result<VisibilitySnapshot, HostError> {
        let reads = (|| Ok::<_, HostError>((read()?, read()?)))();
        match reads {
            Ok((before, after)) => Ok(self.snapshot(after.clone(), before == after)),
            Err(error) if is_bad_window(&error, body) => {
                let evidence = VisibilityEvidence {
                    destroyed: true,
                    wm_state: WmState::Malformed,
                    map_state: Evidence::Missing,
                    hidden: Evidence::Missing,
                    workspace: Workspace::Unknown,
                    capability: Capability::Unverifiable,
                    coherent: false,
                };
                Ok(VisibilitySnapshot {
                    observation: None,
                    candidate: classify(evidence),
                    evidence,
                    layer: LayerValue::Unknown,
                })
            }
            Err(error) => Err(error),
        }
    }

    fn snapshot(&self, observation: Observation, coherent: bool) -> VisibilitySnapshot {
        let capability = match &observation.supported {
            Evidence::Readable(atoms) => {
                if [self.state, self.hidden, self.desktop, self.current_desktop]
                    .iter()
                    .all(|a| atoms.contains(a))
                {
                    Capability::Supported
                } else {
                    Capability::Unsupported
                }
            }
            Evidence::Missing | Evidence::Malformed => Capability::Unverifiable,
        };
        let hidden = match &observation.state {
            Evidence::Readable(atoms) => Evidence::Readable(atoms.contains(&self.hidden)),
            Evidence::Missing => Evidence::Missing,
            Evidence::Malformed => Evidence::Malformed,
        };
        let layer = match (&observation.state, &observation.supported) {
            (Evidence::Readable(state), Evidence::Readable(support))
                if coherent
                    && [self.state, self.above, self.below]
                        .iter()
                        .all(|a| support.contains(a)) =>
            {
                match (state.contains(&self.above), state.contains(&self.below)) {
                    (false, false) => LayerValue::Normal,
                    (true, false) => LayerValue::Above,
                    (false, true) => LayerValue::Below,
                    (true, true) => LayerValue::Conflict,
                }
            }
            _ => LayerValue::Unknown,
        };
        let evidence = VisibilityEvidence {
            destroyed: false,
            wm_state: observation.wm_state,
            map_state: observation.map_state,
            hidden,
            workspace: workspace_relation(observation.desktop, observation.current_desktop),
            capability,
            coherent,
        };
        VisibilitySnapshot {
            observation: Some(observation),
            candidate: classify(evidence),
            evidence,
            layer,
        }
    }
}

fn is_bad_window(error: &HostError, body: Window) -> bool {
    matches!(error.downcast_ref::<ReplyError>(), Some(ReplyError::X11Error(e)) if e.error_kind == ErrorKind::Window && e.bad_value == body)
}

/// Exact bounded 32-bit property, including reply length and missing encoding.
fn decode_words(reply: &GetPropertyReply, type_: Atom, maximum: u32) -> Evidence<Vec<u32>> {
    if reply.type_ == x11rb::NONE {
        return if reply.format == 0
            && reply.length == 0
            && reply.value_len == 0
            && reply.value.is_empty()
            && reply.bytes_after == 0
        {
            Evidence::Missing
        } else {
            Evidence::Malformed
        };
    }
    if reply.type_ != type_
        || reply.format != 32
        || reply.bytes_after != 0
        || reply.value_len > maximum
        || reply.length != reply.value_len
        || usize::try_from(reply.value_len)
            .ok()
            .and_then(|n| n.checked_mul(4))
            != Some(reply.value.len())
    {
        return Evidence::Malformed;
    }
    match reply.value32() {
        Some(values) => Evidence::Readable(values.collect()),
        None => Evidence::Malformed,
    }
}

fn decode_wm_state(reply: &GetPropertyReply, atom: Atom) -> WmState {
    match decode_words(reply, atom, 2) {
        Evidence::Missing => WmState::Absent,
        Evidence::Readable(words) if words.len() == 2 => match words[0] {
            0 => WmState::Withdrawn,
            1 => WmState::Normal,
            3 => WmState::Iconic,
            _ => WmState::Malformed,
        },
        _ => WmState::Malformed,
    }
}

fn decode_map_state(state: NativeMapState) -> Evidence<MapState> {
    match state {
        NativeMapState::UNMAPPED => Evidence::Readable(MapState::Unmapped),
        NativeMapState::UNVIEWABLE => Evidence::Readable(MapState::Unviewable),
        NativeMapState::VIEWABLE => Evidence::Readable(MapState::Viewable),
        _ => Evidence::Malformed,
    }
}

fn decode_desktop(reply: &GetPropertyReply, current: bool) -> Evidence<u32> {
    match decode_words(reply, AtomEnum::CARDINAL.into(), 1) {
        Evidence::Missing => Evidence::Missing,
        Evidence::Readable(words) if words.len() == 1 && (!current || words[0] != u32::MAX) => {
            Evidence::Readable(words[0])
        }
        _ => Evidence::Malformed,
    }
}

fn workspace_relation(body: Evidence<u32>, current: Evidence<u32>) -> Workspace {
    match (body, current) {
        (Evidence::Readable(body), Evidence::Readable(current)) if current != u32::MAX => {
            if body == u32::MAX {
                Workspace::AllDesktops
            } else if body == current {
                Workspace::Current
            } else {
                Workspace::Other
            }
        }
        _ => Workspace::Unknown,
    }
}

/// Narrow request seam: every callback represents one checked protocol operation.
/// Its success is only X processing acknowledgement, never a visibility result.
#[derive(Debug)]
enum VisibilityRequest {
    Iconify {
        destination: Window,
        propagate: bool,
        mask: EventMask,
        event: ClientMessageEvent,
    },
    UserTime {
        body: Window,
        property: Atom,
        mode: PropMode,
        type_: Atom,
        data: [u32; 1],
    },
    Map {
        body: Window,
    },
}

fn checked_request(conn: &impl Connection, request: VisibilityRequest) -> Result<(), HostError> {
    match request {
        VisibilityRequest::Iconify {
            destination,
            propagate,
            mask,
            event,
        } => conn
            .send_event(propagate, destination, mask, event)?
            .check()?,
        VisibilityRequest::UserTime {
            body,
            property,
            mode,
            type_,
            data,
        } => conn
            .change_property32(mode, body, property, type_, &data)?
            .check()?,
        VisibilityRequest::Map { body } => conn.map_window(body)?.check()?,
    }
    Ok(())
}

impl VisibilityAtoms {
    pub fn request_iconify(
        &self,
        conn: &impl Connection,
        root: Window,
        body: Window,
    ) -> Result<(), HostError> {
        self.iconify_with(root, body, |request| checked_request(conn, request))
    }
    fn iconify_with(
        &self,
        root: Window,
        body: Window,
        checked: impl FnOnce(VisibilityRequest) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        checked(VisibilityRequest::Iconify {
            destination: root,
            propagate: false,
            mask: EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event: ClientMessageEvent::new(32, body, self.change_state, [3, 0, 0, 0, 0]),
        })
    }
    pub fn request_restore(&self, conn: &impl Connection, body: Window) -> Result<(), HostError> {
        self.restore_with(body, |request| checked_request(conn, request))
    }
    fn restore_with(
        &self,
        body: Window,
        mut checked: impl FnMut(VisibilityRequest) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        checked(VisibilityRequest::UserTime {
            body,
            property: self.user_time,
            mode: PropMode::REPLACE,
            type_: AtomEnum::CARDINAL.into(),
            data: [0],
        })?;
        checked(VisibilityRequest::Map { body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{Reason, Visibility};

    fn atoms() -> VisibilityAtoms {
        VisibilityAtoms {
            wm_state: 10,
            change_state: 11,
            state: 12,
            hidden: 13,
            above: 14,
            below: 15,
            desktop: 16,
            current_desktop: 17,
            supported: 18,
            user_time: 19,
        }
    }
    fn property(type_: Atom, values: &[u32]) -> GetPropertyReply {
        GetPropertyReply {
            type_,
            format: 32,
            length: values.len() as u32,
            value_len: values.len() as u32,
            value: values.iter().flat_map(|v| v.to_ne_bytes()).collect(),
            ..Default::default()
        }
    }
    fn observation() -> Observation {
        Observation {
            wm_state: WmState::Normal,
            map_state: Evidence::Readable(MapState::Viewable),
            state: Evidence::Readable(vec![]),
            desktop: Evidence::Readable(2),
            current_desktop: Evidence::Readable(2),
            supported: Evidence::Readable(vec![12, 13, 14, 15, 16, 17]),
        }
    }
    fn corruptions(reply: GetPropertyReply) -> Vec<GetPropertyReply> {
        let mut cases = vec![];
        let mut r = reply.clone();
        r.type_ = 999;
        cases.push(r);
        let mut r = reply.clone();
        r.format = 8;
        cases.push(r);
        let mut r = reply.clone();
        r.value_len += 1;
        cases.push(r);
        let mut r = reply.clone();
        r.length += 1;
        cases.push(r);
        let mut r = reply.clone();
        r.value.push(0);
        cases.push(r);
        let mut r = reply.clone();
        r.value.pop();
        cases.push(r);
        let mut r = reply.clone();
        r.bytes_after = 4;
        cases.push(r);
        let mut r = reply;
        r.type_ = 0;
        cases.push(r);
        cases
    }
    #[test]
    fn icccm_requires_exact_known_state_and_two_words() {
        for (value, expected) in [
            (0, WmState::Withdrawn),
            (1, WmState::Normal),
            (3, WmState::Iconic),
            (2, WmState::Malformed),
        ] {
            assert_eq!(decode_wm_state(&property(10, &[value, 123]), 10), expected);
        }
        assert_eq!(
            decode_wm_state(&GetPropertyReply::default(), 10),
            WmState::Absent
        );
        for values in [vec![], vec![1], vec![1, 0, 0]] {
            assert_eq!(
                decode_wm_state(&property(10, &values), 10),
                WmState::Malformed
            );
        }
        for reply in corruptions(property(10, &[1, 0])) {
            assert_eq!(decode_wm_state(&reply, 10), WmState::Malformed);
        }
    }
    #[test]
    fn complete_atom_payload_retains_unrelated_atoms_and_explicit_layer_flags() {
        for (values, hidden, layer) in [
            (vec![], false, LayerValue::Normal),
            (vec![13], true, LayerValue::Normal),
            (vec![14, 99], false, LayerValue::Above),
            (vec![15], false, LayerValue::Below),
            (vec![13, 14, 15, 99], true, LayerValue::Conflict),
        ] {
            let decoded = decode_words(
                &property(AtomEnum::ATOM.into(), &values),
                AtomEnum::ATOM.into(),
                MAX_ATOMS,
            );
            assert_eq!(decoded, Evidence::Readable(values));
            let snapshot = atoms().snapshot(
                Observation {
                    state: decoded,
                    ..observation()
                },
                true,
            );
            assert_eq!(snapshot.evidence.hidden, Evidence::Readable(hidden));
            assert_eq!(snapshot.layer, layer);
        }
        assert_eq!(
            decode_words(
                &GetPropertyReply::default(),
                AtomEnum::ATOM.into(),
                MAX_ATOMS
            ),
            Evidence::Missing
        );
        for reply in corruptions(property(AtomEnum::ATOM.into(), &[13]))
            .into_iter()
            .chain([property(
                AtomEnum::ATOM.into(),
                &vec![13; MAX_ATOMS as usize + 1],
            )])
        {
            assert_eq!(
                decode_words(&reply, AtomEnum::ATOM.into(), MAX_ATOMS),
                Evidence::Malformed
            );
        }
    }
    #[test]
    fn workspace_is_exact_cardinal_without_desktop_zero_fallback() {
        for (body, current, expected) in [
            (2, 2, Workspace::Current),
            (3, 2, Workspace::Other),
            (u32::MAX, 2, Workspace::AllDesktops),
        ] {
            assert_eq!(
                workspace_relation(
                    decode_desktop(&property(AtomEnum::CARDINAL.into(), &[body]), false),
                    decode_desktop(&property(AtomEnum::CARDINAL.into(), &[current]), true)
                ),
                expected
            );
        }
        assert_eq!(
            decode_desktop(&GetPropertyReply::default(), true),
            Evidence::Missing
        );
        for reply in corruptions(property(AtomEnum::CARDINAL.into(), &[2]))
            .into_iter()
            .chain([
                property(AtomEnum::CARDINAL.into(), &[]),
                property(AtomEnum::CARDINAL.into(), &[2, 3]),
            ])
        {
            assert_eq!(decode_desktop(&reply, false), Evidence::Malformed);
            assert_eq!(decode_desktop(&reply, true), Evidence::Malformed);
        }
        assert_eq!(
            decode_desktop(&property(AtomEnum::CARDINAL.into(), &[u32::MAX]), true),
            Evidence::Malformed
        );
        for missing in [Evidence::Missing, Evidence::Malformed] {
            assert_eq!(
                workspace_relation(missing, Evidence::Readable(0)),
                Workspace::Unknown
            );
            assert_eq!(
                workspace_relation(Evidence::Readable(u32::MAX), missing),
                Workspace::Unknown
            );
        }
    }
    #[test]
    fn capability_absence_partial_and_malformed_never_make_success() {
        for omitted in [12, 13, 16, 17] {
            let mut o = observation();
            o.supported = Evidence::Readable(
                vec![12, 13, 14, 15, 16, 17]
                    .into_iter()
                    .filter(|a| *a != omitted)
                    .collect(),
            );
            let s = atoms().snapshot(o, true);
            assert_eq!(s.candidate.reason, Reason::Unsupported);
        }
        for evidence in [Evidence::Missing, Evidence::Malformed] {
            let s = atoms().snapshot(
                Observation {
                    supported: evidence.clone(),
                    ..observation()
                },
                true,
            );
            assert_eq!(s.candidate.reason, Reason::Unverifiable);
            assert_eq!(s.layer, LayerValue::Unknown);
            let s = atoms().snapshot(
                Observation {
                    state: evidence,
                    ..observation()
                },
                true,
            );
            assert_eq!(s.candidate.reason, Reason::Unverifiable);
            assert_eq!(s.layer, LayerValue::Unknown);
        }
        let s = atoms().snapshot(
            Observation {
                supported: Evidence::Readable(vec![12, 13, 16, 17]),
                ..observation()
            },
            true,
        );
        assert_eq!(s.layer, LayerValue::Unknown);
    }
    #[test]
    fn native_decoded_combinations_use_stage_a_candidates() {
        use MapState::*;
        use Visibility::*;
        use WmState::{Iconic, Normal};
        for (wm, map, hidden, desktop, expected) in [
            (Iconic, Unmapped, true, 2, Minimized),
            (Iconic, Unmapped, true, 3, Minimized),
            (Iconic, Unmapped, false, 3, NotMinimizedElsewhere),
            (Normal, Unmapped, false, 3, NotMinimizedElsewhere),
            (Normal, Unviewable, false, 3, NotMinimizedElsewhere),
            (Normal, Viewable, false, 2, NotMinimizedHere),
            (Normal, Viewable, false, u32::MAX, NotMinimizedHere),
            (Iconic, Unmapped, false, 2, Transitional),
            (Normal, Unmapped, false, 2, Transitional),
            (Normal, Unviewable, false, 2, Transitional),
            (Normal, Viewable, true, 2, Transitional),
            (Iconic, Viewable, true, 2, Transitional),
            (Iconic, Unviewable, true, 3, Transitional),
            (Normal, Unmapped, true, 3, Transitional),
            (Normal, Viewable, false, 3, Unknown),
            (Iconic, Unviewable, false, 3, Unknown),
            (WmState::Absent, Unmapped, false, 2, Withdrawn),
            (WmState::Withdrawn, Unmapped, false, 2, Withdrawn),
            (WmState::Malformed, Unmapped, false, 2, Unknown),
        ] {
            let s = atoms().snapshot(
                Observation {
                    wm_state: wm,
                    map_state: Evidence::Readable(map),
                    state: Evidence::Readable(if hidden { vec![13] } else { vec![] }),
                    desktop: Evidence::Readable(desktop),
                    ..observation()
                },
                true,
            );
            assert_eq!(
                s.candidate.visibility, expected,
                "{wm:?}/{map:?}/{hidden}/{desktop}"
            );
        }
        for state in [Evidence::Missing, Evidence::Malformed] {
            let s = atoms().snapshot(
                Observation {
                    wm_state: Iconic,
                    map_state: Evidence::Readable(Unmapped),
                    state,
                    ..observation()
                },
                true,
            );
            assert_eq!(s.candidate.visibility, Unknown);
        }
    }
    #[test]
    fn rereads_detect_drift_and_fresh_observations_are_not_cached() {
        let mut count = 0;
        let s = atoms()
            .observe_with(100, || {
                count += 1;
                Ok(Observation {
                    desktop: Evidence::Readable(count),
                    ..observation()
                })
            })
            .unwrap();
        assert_eq!(count, 2);
        assert!(!s.evidence.coherent);
        assert_eq!(s.candidate.visibility, Visibility::Transitional);
        assert_eq!(s.layer, LayerValue::Unknown);
        let s = atoms().observe_with(100, || Ok(observation())).unwrap();
        assert_eq!(s.candidate.visibility, Visibility::NotMinimizedHere);
        assert!(atoms()
            .observe_with(100, || Err("connection lost".into()))
            .is_err());
    }
    #[test]
    fn map_states_are_distinct_and_bad_window_is_destroyed() {
        for (native, decoded) in [
            (NativeMapState::UNMAPPED, MapState::Unmapped),
            (NativeMapState::UNVIEWABLE, MapState::Unviewable),
            (NativeMapState::VIEWABLE, MapState::Viewable),
        ] {
            assert_eq!(decode_map_state(native), Evidence::Readable(decoded));
        }
        assert_eq!(
            decode_map_state(NativeMapState::from(9)),
            Evidence::Malformed
        );
        let s = atoms()
            .observe_with(100, || {
                Err(ReplyError::X11Error(x11rb::x11_utils::X11Error {
                    error_kind: ErrorKind::Window,
                    error_code: 3,
                    sequence: 1,
                    bad_value: 100,
                    minor_opcode: 0,
                    major_opcode: 3,
                    extension_name: None,
                    request_name: None,
                })
                .into())
            })
            .unwrap();
        assert_eq!(s.candidate.visibility, Visibility::Destroyed);
        assert!(s.observation.is_none());
    }
    #[test]
    fn iconify_exact_envelope_and_checked_error_without_retry_or_confirmation() {
        for fail in [false, true] {
            let mut calls = 0;
            let result = atoms().iconify_with(100, 200, |request| {
                calls += 1;
                let VisibilityRequest::Iconify {
                    destination,
                    propagate,
                    mask,
                    event,
                } = request
                else {
                    panic!("unrelated operation")
                };
                assert_eq!(destination, 100);
                assert!(!propagate);
                assert_eq!(
                    mask,
                    EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT
                );
                assert_eq!((event.window, event.type_, event.format), (200, 11, 32));
                assert_eq!(event.data.as_data32(), [3, 0, 0, 0, 0]);
                if fail {
                    Err("checked send failed".into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(calls, 1);
            assert_eq!(result.is_err(), fail);
        }
    }
    #[test]
    fn restoration_checks_only_user_time_then_map_and_stops_at_first_error() {
        for fail_at in [0, 1, 2] {
            let mut calls = 0;
            let result = atoms().restore_with(200, |request| {
                calls += 1;
                match request {
                    VisibilityRequest::UserTime {
                        body,
                        property,
                        mode,
                        type_,
                        data,
                    } => {
                        assert_eq!(calls, 1);
                        assert_eq!((body, property), (200, 19));
                        assert_eq!(mode, PropMode::REPLACE);
                        assert_eq!(type_, AtomEnum::CARDINAL.into());
                        assert_eq!(data, [0]);
                    }
                    VisibilityRequest::Map { body } => {
                        assert_eq!(calls, 2);
                        assert_eq!(body, 200);
                    }
                    _ => panic!("unrelated mutation"),
                }
                if calls == fail_at {
                    Err("checked request failed".into())
                } else {
                    Ok(())
                }
            });
            assert_eq!(calls, if fail_at == 1 { 1 } else { 2 });
            assert_eq!(result.is_err(), fail_at != 0);
        }
    }
}
