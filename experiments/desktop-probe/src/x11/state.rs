//! Checked EWMH layer transport and strict property decoding.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, GetPropertyReply, MapState,
    Window,
};

use super::visibility::{VisibilityAtoms, VisibilitySnapshot};
use crate::control::{Evidence, LayerObservation, Visibility, VisibilityBaseline};
use crate::layer::{Flags, Mutation, ObservedLayer};

type HostError = Box<dyn std::error::Error>;
pub type PropertyResult<T> = Result<T, &'static str>;
const MAX_ATOMS: u32 = 4096;

pub struct LayerAtoms {
    pub supported: Atom,
    pub state: Atom,
    pub above: Atom,
    pub below: Atom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerSupport {
    pub state: bool,
    pub above: bool,
    pub below: bool,
}

/// Remote qualification only; the local menu continues to require Viewable.
/// Waiting is transitional evidence, never permission to dispatch a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteLayerIssue {
    Waiting,
    Unsupported,
    Unverifiable,
    WorkspaceChanged,
    Unavailable(Visibility),
}
impl LayerSupport {
    pub fn allows(self, required: Flags) -> bool {
        self.state && (!required.above || self.above) && (!required.below || self.below)
    }
}

impl LayerAtoms {
    pub fn intern(conn: &impl Connection) -> Result<Self, HostError> {
        Ok(Self {
            supported: conn.intern_atom(false, b"_NET_SUPPORTED")?.reply()?.atom,
            state: conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom,
            above: conn
                .intern_atom(false, b"_NET_WM_STATE_ABOVE")?
                .reply()?
                .atom,
            below: conn
                .intern_atom(false, b"_NET_WM_STATE_BELOW")?
                .reply()?
                .atom,
        })
    }

    pub fn read_support(
        &self,
        conn: &impl Connection,
        root: Window,
    ) -> Result<PropertyResult<LayerSupport>, HostError> {
        let reply = conn
            .get_property(false, root, self.supported, AtomEnum::ANY, 0, MAX_ATOMS)?
            .reply()?;
        Ok(self.decode_support(&reply))
    }

    pub fn read_body(
        &self,
        conn: &impl Connection,
        body: Window,
    ) -> Result<PropertyResult<ObservedLayer>, HostError> {
        // A queued property event can precede the corresponding unmap notification.
        // Never interpret withdrawal/property deletion as a successful Normal result.
        let map_state = conn.get_window_attributes(body)?.reply()?.map_state;
        self.read_viewable_with(map_state, || {
            Ok(conn
                .get_property(false, body, self.state, AtomEnum::ANY, 0, MAX_ATOMS)?
                .reply()?)
        })
    }

    fn read_viewable_with(
        &self,
        map_state: MapState,
        read: impl FnOnce() -> Result<GetPropertyReply, HostError>,
    ) -> Result<PropertyResult<ObservedLayer>, HostError> {
        if map_state != MapState::VIEWABLE {
            return Ok(Err("body is not viewable; layer observation unavailable"));
        }
        Ok(self.decode_body(&read()?))
    }

    /// Remote-only path: observe the same body afresh, including repeated-read
    /// coherence and BadWindow classification from the accepted visibility reader.
    /// Each layer reread uses this adapter again, never cached permission.
    pub fn read_remote(
        &self,
        visibility: &VisibilityAtoms,
        conn: &impl Connection,
        root: Window,
        body: Window,
        baseline: VisibilityBaseline,
    ) -> Result<Result<ObservedLayer, RemoteLayerIssue>, HostError> {
        let snapshot = visibility.observe(conn, root, body)?;
        Ok(self.qualify_remote(&snapshot, baseline))
    }

    pub fn qualify_remote(
        &self,
        snapshot: &VisibilitySnapshot,
        baseline: VisibilityBaseline,
    ) -> Result<ObservedLayer, RemoteLayerIssue> {
        use RemoteLayerIssue::*;
        if snapshot.evidence.destroyed {
            return Err(Unavailable(Visibility::Destroyed));
        }
        let observed = snapshot.observation.as_ref().ok_or(Unverifiable)?;
        if matches!(
            observed.wm_state,
            crate::control::WmState::Absent | crate::control::WmState::Withdrawn
        ) {
            return Err(Unavailable(Visibility::Withdrawn));
        }
        let Evidence::Readable(support) = &observed.supported else {
            return Err(Unverifiable);
        };
        if snapshot.evidence.capability == crate::control::Capability::Unsupported
            || ![self.state, self.above, self.below]
                .iter()
                .all(|a| support.contains(a))
        {
            return Err(Unsupported);
        }
        if snapshot.evidence.capability != crate::control::Capability::Supported {
            return Err(Unverifiable);
        }
        if !snapshot.evidence.coherent {
            return Err(Waiting);
        }
        if snapshot.evidence.workspace == crate::control::Workspace::Unknown {
            return Err(Unverifiable);
        }
        let (Evidence::Readable(desktop), Evidence::Readable(current)) =
            (observed.desktop, observed.current_desktop)
        else {
            return Err(Unverifiable);
        };
        if desktop != baseline.desktop || current != baseline.current_desktop {
            return Err(WorkspaceChanged);
        }
        let layer = match snapshot.layer {
            LayerObservation::Normal => ObservedLayer::Normal,
            LayerObservation::Above => ObservedLayer::Above,
            LayerObservation::Below => ObservedLayer::Below,
            LayerObservation::Unknown | LayerObservation::Conflict => return Err(Unverifiable),
        };
        match snapshot.candidate.visibility {
            Visibility::NotMinimizedHere | Visibility::NotMinimizedElsewhere => Ok(layer),
            Visibility::Transitional => Err(Waiting),
            Visibility::Minimized | Visibility::Withdrawn | Visibility::Destroyed => {
                Err(Unavailable(snapshot.candidate.visibility))
            }
            Visibility::Unknown => Err(Unverifiable),
        }
    }

    pub fn decode_body(&self, reply: &GetPropertyReply) -> PropertyResult<ObservedLayer> {
        let atoms =
            atom_values(reply)?.ok_or("mapped body _NET_WM_STATE is absent; unverifiable")?;
        Ok(ObservedLayer::from_flags(
            atoms.contains(&self.above),
            atoms.contains(&self.below),
        ))
    }

    pub fn decode_support(&self, reply: &GetPropertyReply) -> PropertyResult<LayerSupport> {
        let atoms = atom_values(reply)?.unwrap_or_default();
        Ok(LayerSupport {
            state: atoms.contains(&self.state),
            above: atoms.contains(&self.above),
            below: atoms.contains(&self.below),
        })
    }

    pub fn request(
        &self,
        root: Window,
        body: Window,
        mutation: Mutation,
    ) -> Result<LayerRequest, HostError> {
        let (action, first, second) = match mutation {
            Mutation::Remove(flags) if !flags.above && !flags.below => {
                return Err("cannot remove an empty set of layer flags".into());
            }
            Mutation::Remove(flags) => (
                0,
                if flags.above { self.above } else { self.below },
                if flags.above && flags.below {
                    self.below
                } else {
                    0
                },
            ),
            Mutation::AddAbove => (1, self.above, 0),
            Mutation::AddBelow => (1, self.below, 0),
        };
        Ok(LayerRequest {
            destination: root,
            propagate: false,
            mask: EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event: ClientMessageEvent::new(32, body, self.state, [action, first, second, 1, 0]),
        })
    }

    pub fn send(
        &self,
        conn: &impl Connection,
        root: Window,
        body: Window,
        mutation: Mutation,
    ) -> Result<(), HostError> {
        self.send_marked(conn, root, body, mutation, &mut false)
    }

    /// Cookie acquisition marks possible host mutation before the checked reply.
    /// Err with dispatched=true is uncertain native work, never a reason to retry.
    pub fn send_marked(
        &self,
        conn: &impl Connection,
        root: Window,
        body: Window,
        mutation: Mutation,
        dispatched: &mut bool,
    ) -> Result<(), HostError> {
        let request = self.request(root, body, mutation)?;
        send_marked_with(
            request,
            dispatched,
            |request| {
                Ok(conn.send_event(
                    request.propagate,
                    request.destination,
                    request.mask,
                    request.event,
                )?)
            },
            |cookie| Ok(cookie.check()?),
        )
    }
}

fn send_marked_with<T>(
    request: LayerRequest,
    dispatched: &mut bool,
    send: impl FnOnce(LayerRequest) -> Result<T, HostError>,
    check: impl FnOnce(T) -> Result<(), HostError>,
) -> Result<(), HostError> {
    let cookie = send(request)?;
    *dispatched = true;
    // X processing acknowledgement alone never confirms the WM's layer state.
    check(cookie)
}

pub struct LayerRequest {
    pub destination: Window,
    pub propagate: bool,
    pub mask: EventMask,
    pub event: ClientMessageEvent,
}

fn atom_values(reply: &GetPropertyReply) -> PropertyResult<Option<Vec<Atom>>> {
    if reply.type_ == x11rb::NONE {
        return if reply.format == 0
            && reply.value_len == 0
            && reply.value.is_empty()
            && reply.bytes_after == 0
        {
            Ok(None)
        } else {
            Err("invalid absent ATOM property encoding")
        };
    }
    if reply.type_ != AtomEnum::ATOM.into()
        || reply.format != 32
        || reply.bytes_after != 0
        || reply.value_len > MAX_ATOMS
        || usize::try_from(reply.value_len)
            .ok()
            .and_then(|len| len.checked_mul(4))
            != Some(reply.value.len())
    {
        return Err("malformed or truncated ATOM/32 property");
    }
    Ok(Some(
        reply.value32().ok_or("invalid ATOM payload")?.collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::visibility::Observation;
    use super::*;
    use crate::control::{
        classify, Capability, MapState as BodyMapState, VisibilityEvidence, WmState, Workspace,
    };

    fn atoms() -> LayerAtoms {
        LayerAtoms {
            supported: 10,
            state: 11,
            above: 12,
            below: 13,
        }
    }
    fn property(values: &[u32]) -> GetPropertyReply {
        GetPropertyReply {
            format: 32,
            type_: AtomEnum::ATOM.into(),
            bytes_after: 0,
            value_len: values.len() as u32,
            value: values
                .iter()
                .flat_map(|value| value.to_ne_bytes())
                .collect(),
            ..Default::default()
        }
    }

    // Synthetic decoded snapshots exercise only qualification. Repeated native
    // reads/decoding are covered by visibility's existing observer tests.
    fn remote_observation() -> Observation {
        Observation {
            wm_state: WmState::Normal,
            map_state: Evidence::Readable(BodyMapState::Viewable),
            state: Evidence::Readable(vec![]),
            desktop: Evidence::Readable(2),
            current_desktop: Evidence::Readable(2),
            supported: Evidence::Readable(vec![11, 12, 13, 14, 15, 16]),
        }
    }

    fn remote_snapshot(observation: Observation, coherent: bool) -> VisibilitySnapshot {
        let capability = match &observation.supported {
            Evidence::Readable(support) => {
                if [11, 14, 15, 16].iter().all(|a| support.contains(a)) {
                    Capability::Supported
                } else {
                    Capability::Unsupported
                }
            }
            _ => Capability::Unverifiable,
        };
        let hidden = match &observation.state {
            Evidence::Readable(state) => Evidence::Readable(state.contains(&14)),
            Evidence::Missing => Evidence::Missing,
            Evidence::Malformed => Evidence::Malformed,
        };
        let workspace = match (observation.desktop, observation.current_desktop) {
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
        };
        let layer = match (&observation.state, &observation.supported) {
            (Evidence::Readable(state), Evidence::Readable(support))
                if coherent && [11, 12, 13].iter().all(|a| support.contains(a)) =>
            {
                match atoms().decode_body(&property(state)).unwrap() {
                    ObservedLayer::Normal => LayerObservation::Normal,
                    ObservedLayer::Above => LayerObservation::Above,
                    ObservedLayer::Below => LayerObservation::Below,
                    ObservedLayer::Conflict => LayerObservation::Conflict,
                }
            }
            _ => LayerObservation::Unknown,
        };
        let evidence = VisibilityEvidence {
            destroyed: false,
            wm_state: observation.wm_state,
            map_state: observation.map_state,
            hidden,
            workspace,
            capability,
            coherent,
        };
        VisibilitySnapshot {
            observation: Some(observation),
            evidence,
            candidate: classify(evidence),
            layer,
        }
    }

    fn baseline(desktop: u32, current_desktop: u32) -> VisibilityBaseline {
        VisibilityBaseline {
            desktop,
            current_desktop,
            layer: LayerObservation::Normal,
        }
    }

    #[test]
    fn remote_qualification_accepts_exact_layers_locally_and_on_all_desktops() {
        for desktop in [2, u32::MAX] {
            for (state, layer) in [
                (vec![], ObservedLayer::Normal),
                (vec![12, 99], ObservedLayer::Above),
                (vec![13], ObservedLayer::Below),
            ] {
                let mut observed = remote_observation();
                observed.desktop = Evidence::Readable(desktop);
                observed.state = Evidence::Readable(state);
                let snapshot = remote_snapshot(observed, true);
                assert_eq!(snapshot.candidate.visibility, Visibility::NotMinimizedHere);
                assert_eq!(
                    atoms().qualify_remote(&snapshot, baseline(desktop, 2)),
                    Ok(layer)
                );
            }
        }
    }

    #[test]
    fn remote_qualification_accepts_only_the_approved_off_workspace_representations() {
        // The Normal variants are deterministic cases, not real-host observations.
        for (wm, map) in [
            (WmState::Iconic, BodyMapState::Unmapped),
            (WmState::Normal, BodyMapState::Unmapped),
            (WmState::Normal, BodyMapState::Unviewable),
        ] {
            for (state, layer) in [
                (vec![], ObservedLayer::Normal),
                (vec![12], ObservedLayer::Above),
                (vec![13], ObservedLayer::Below),
            ] {
                let mut observed = remote_observation();
                observed.wm_state = wm;
                observed.map_state = Evidence::Readable(map);
                observed.desktop = Evidence::Readable(7);
                observed.state = Evidence::Readable(state);
                let snapshot = remote_snapshot(observed, true);
                assert_eq!(
                    snapshot.candidate.visibility,
                    Visibility::NotMinimizedElsewhere
                );
                assert_eq!(atoms().qualify_remote(&snapshot, baseline(7, 2)), Ok(layer));
            }
        }
    }

    #[test]
    fn remote_conflicting_or_unknown_layer_is_unverifiable_without_repair() {
        let mut observed = remote_observation();
        observed.state = Evidence::Readable(vec![12, 13]);
        let mut snapshot = remote_snapshot(observed, true);
        assert_eq!(
            atoms().qualify_remote(&snapshot, baseline(2, 2)),
            Err(RemoteLayerIssue::Unverifiable)
        );
        snapshot.layer = LayerObservation::Unknown;
        assert_eq!(
            atoms().qualify_remote(&snapshot, baseline(2, 2)),
            Err(RemoteLayerIssue::Unverifiable)
        );
    }

    #[test]
    fn remote_minimized_withdrawn_and_destroyed_are_ineligible() {
        for desktop in [2, 7] {
            let mut observed = remote_observation();
            observed.desktop = Evidence::Readable(desktop);
            observed.wm_state = WmState::Iconic;
            observed.map_state = Evidence::Readable(BodyMapState::Unmapped);
            observed.state = Evidence::Readable(vec![12, 14]);
            let snapshot = remote_snapshot(observed, true);
            assert_eq!(
                atoms().qualify_remote(&snapshot, baseline(desktop, 2)),
                Err(RemoteLayerIssue::Unavailable(Visibility::Minimized))
            );
        }
        for wm in [WmState::Absent, WmState::Withdrawn] {
            let mut observed = remote_observation();
            observed.wm_state = wm;
            let snapshot = remote_snapshot(observed, true);
            assert_eq!(
                atoms().qualify_remote(&snapshot, baseline(2, 2)),
                Err(RemoteLayerIssue::Unavailable(Visibility::Withdrawn))
            );
        }
        let mut snapshot = remote_snapshot(remote_observation(), true);
        snapshot.evidence.destroyed = true;
        snapshot.observation = None;
        snapshot.candidate = classify(snapshot.evidence);
        assert_eq!(
            atoms().qualify_remote(&snapshot, baseline(2, 2)),
            Err(RemoteLayerIssue::Unavailable(Visibility::Destroyed))
        );
    }

    #[test]
    fn remote_missing_advertised_visibility_or_layer_support_is_unsupported() {
        for missing in [11, 12, 13, 14, 15, 16] {
            let mut observed = remote_observation();
            observed.supported = Evidence::Readable(
                vec![11, 12, 13, 14, 15, 16]
                    .into_iter()
                    .filter(|a| *a != missing)
                    .collect(),
            );
            assert_eq!(
                atoms().qualify_remote(&remote_snapshot(observed, true), baseline(2, 2)),
                Err(RemoteLayerIssue::Unsupported)
            );
        }
    }

    #[test]
    fn remote_malformed_or_missing_required_evidence_is_unverifiable() {
        for case in 0..12 {
            let mut observed = remote_observation();
            match case {
                0 => observed.state = Evidence::Malformed,
                1 => observed.state = Evidence::Missing,
                2 => observed.supported = Evidence::Malformed,
                3 => observed.supported = Evidence::Missing,
                4 => observed.desktop = Evidence::Malformed,
                5 => observed.desktop = Evidence::Missing,
                6 => observed.current_desktop = Evidence::Malformed,
                7 => observed.current_desktop = Evidence::Missing,
                8 => observed.current_desktop = Evidence::Readable(u32::MAX),
                9 => observed.map_state = Evidence::Malformed,
                10 => observed.map_state = Evidence::Missing,
                _ => observed.wm_state = WmState::Malformed,
            }
            assert_eq!(
                atoms().qualify_remote(&remote_snapshot(observed, true), baseline(2, 2)),
                Err(RemoteLayerIssue::Unverifiable),
                "case {case}"
            );
        }
    }

    #[test]
    fn remote_both_workspace_baselines_must_match_and_incoherence_waits() {
        for body_changed in [false, true] {
            let mut observed = remote_observation();
            if body_changed {
                observed.desktop = Evidence::Readable(7);
            } else {
                observed.current_desktop = Evidence::Readable(7);
            }
            assert_eq!(
                atoms().qualify_remote(&remote_snapshot(observed, true), baseline(2, 2)),
                Err(RemoteLayerIssue::WorkspaceChanged)
            );
        }
        let mut observed = remote_observation();
        observed.state = Evidence::Readable(vec![12]);
        assert_eq!(
            atoms().qualify_remote(&remote_snapshot(observed, false), baseline(2, 2)),
            Err(RemoteLayerIssue::Waiting)
        );
    }

    #[test]
    fn remote_transitional_visibility_waits_without_guessing_minimization() {
        for (wm, map, state) in [
            (WmState::Normal, BodyMapState::Unmapped, vec![]),
            (WmState::Iconic, BodyMapState::Unmapped, vec![]),
            (WmState::Normal, BodyMapState::Viewable, vec![14]),
        ] {
            let mut observed = remote_observation();
            observed.wm_state = wm;
            observed.map_state = Evidence::Readable(map);
            observed.state = Evidence::Readable(state);
            assert_eq!(
                atoms().qualify_remote(&remote_snapshot(observed, true), baseline(2, 2)),
                Err(RemoteLayerIssue::Waiting)
            );
        }
    }

    #[test]
    fn local_reader_still_rejects_non_viewable_before_reading_layer_properties() {
        for map in [MapState::UNMAPPED, MapState::UNVIEWABLE] {
            assert_eq!(
                atoms()
                    .read_viewable_with(map, || panic!(
                        "local reader cannot inspect off-workspace layer"
                    ))
                    .unwrap(),
                Err("body is not viewable; layer observation unavailable")
            );
        }
        for (state, expected) in [
            (vec![], ObservedLayer::Normal),
            (vec![12], ObservedLayer::Above),
            (vec![13], ObservedLayer::Below),
        ] {
            assert_eq!(
                atoms()
                    .read_viewable_with(MapState::VIEWABLE, || Ok(property(&state)))
                    .unwrap(),
                Ok(expected)
            );
        }
    }

    #[test]
    fn marked_layer_send_preserves_exact_envelopes_and_dispatch_after_check_failure() {
        use std::cell::RefCell;
        use x11rb::x11_utils::Serialize;
        for (mutation, payload) in [
            (Mutation::AddAbove, [1, 12, 0, 1, 0]),
            (Mutation::AddBelow, [1, 13, 0, 1, 0]),
            (
                Mutation::Remove(Flags {
                    above: true,
                    below: false,
                }),
                [0, 12, 0, 1, 0],
            ),
            (
                Mutation::Remove(Flags {
                    above: false,
                    below: true,
                }),
                [0, 13, 0, 1, 0],
            ),
            (
                Mutation::Remove(Flags {
                    above: true,
                    below: true,
                }),
                [0, 12, 13, 1, 0],
            ),
        ] {
            for fail_at in [None, Some("send"), Some("check")] {
                let mut dispatched = false;
                let trace = RefCell::new(vec![]);
                let result = send_marked_with(
                    atoms().request(100, 200, mutation).unwrap(),
                    &mut dispatched,
                    |request| {
                        trace.borrow_mut().push("send");
                        assert!(!request.propagate);
                        assert_eq!(request.destination, 100);
                        assert_eq!(
                            request.mask,
                            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT
                        );
                        let mut expected = vec![33, 32, 0, 0];
                        for word in [200_u32, 11].into_iter().chain(payload) {
                            expected.extend(word.to_ne_bytes());
                        }
                        assert_eq!(request.event.serialize().as_slice(), expected);
                        if fail_at == Some("send") {
                            Err("send".into())
                        } else {
                            Ok(77)
                        }
                    },
                    |cookie| {
                        trace.borrow_mut().push("check");
                        assert_eq!(cookie, 77);
                        if fail_at == Some("check") {
                            Err("check".into())
                        } else {
                            Ok(())
                        }
                    },
                );
                assert_eq!(dispatched, fail_at != Some("send"));
                assert_eq!(
                    *trace.borrow(),
                    if fail_at == Some("send") {
                        vec!["send"]
                    } else {
                        vec!["send", "check"]
                    }
                );
                assert_eq!(
                    result.err().map(|e| e.to_string()),
                    fail_at.map(str::to_string)
                );
            }
        }
    }

    #[test]
    fn body_decodes_present_states_ignores_unrelated_atoms_but_rejects_absence() {
        for (values, expected) in [
            (vec![], ObservedLayer::Normal),
            (vec![99], ObservedLayer::Normal),
            (vec![12, 99, 12], ObservedLayer::Above),
            (vec![13], ObservedLayer::Below),
            (vec![12, 13], ObservedLayer::Conflict),
        ] {
            assert_eq!(atoms().decode_body(&property(&values)), Ok(expected));
        }
        assert!(atoms().decode_body(&GetPropertyReply::default()).is_err());
    }

    #[test]
    fn support_requires_state_and_every_required_flag() {
        let required = Flags {
            above: true,
            below: true,
        };
        for values in [vec![], vec![11], vec![11, 12], vec![11, 13], vec![12, 13]] {
            assert!(!atoms()
                .decode_support(&property(&values))
                .unwrap()
                .allows(required));
        }
        assert!(!atoms()
            .decode_support(&GetPropertyReply::default())
            .unwrap()
            .allows(required));
        assert!(atoms()
            .decode_support(&property(&[11, 12, 13, 99]))
            .unwrap()
            .allows(required));
        assert!(atoms()
            .decode_support(&property(&[11, 12]))
            .unwrap()
            .allows(Flags {
                above: true,
                below: false
            }));
    }

    #[test]
    fn both_decoders_reject_wrong_type_format_truncation_and_inconsistent_payload() {
        let mut cases = Vec::new();
        let mut reply = property(&[12]);
        reply.type_ = AtomEnum::CARDINAL.into();
        cases.push(reply);
        let mut reply = property(&[12]);
        reply.format = 8;
        cases.push(reply);
        let mut reply = property(&[12]);
        reply.bytes_after = 4;
        cases.push(reply);
        let mut reply = property(&[12]);
        reply.value_len = 2;
        cases.push(reply);
        let mut reply = property(&[12]);
        reply.value.push(0);
        cases.push(reply);
        let mut reply = property(&[12]);
        reply.type_ = x11rb::NONE;
        cases.push(reply);
        cases.push(property(&vec![12; MAX_ATOMS as usize + 1]));
        for reply in cases {
            assert!(atoms().decode_body(&reply).is_err());
            assert!(atoms().decode_support(&reply).is_err());
        }
    }

    #[test]
    fn empty_removal_cannot_construct_an_x11_request() {
        assert!(atoms()
            .request(
                100,
                200,
                Mutation::Remove(Flags {
                    above: false,
                    below: false,
                }),
            )
            .is_err());
    }

    #[test]
    fn ewmh_envelope_is_root_directed_absolute_and_nonactivating() {
        for (mutation, payload) in [
            (Mutation::AddAbove, [1, 12, 0, 1, 0]),
            (Mutation::AddBelow, [1, 13, 0, 1, 0]),
            (
                Mutation::Remove(Flags {
                    above: true,
                    below: false,
                }),
                [0, 12, 0, 1, 0],
            ),
            (
                Mutation::Remove(Flags {
                    above: false,
                    below: true,
                }),
                [0, 13, 0, 1, 0],
            ),
            (
                Mutation::Remove(Flags {
                    above: true,
                    below: true,
                }),
                [0, 12, 13, 1, 0],
            ),
        ] {
            let request = atoms().request(100, 200, mutation).unwrap();
            assert!(!request.propagate);
            assert_eq!(request.destination, 100);
            assert_eq!(
                request.mask,
                EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT
            );
            assert_eq!(request.event.window, 200);
            assert_eq!(request.event.type_, 11);
            assert_eq!(request.event.format, 32);
            assert_eq!(request.event.data.as_data32(), payload);
        }
    }
}
