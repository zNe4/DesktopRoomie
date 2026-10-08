//! Checked EWMH layer transport and strict property decoding.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, GetPropertyReply, MapState,
    Window,
};

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
        if conn.get_window_attributes(body)?.reply()?.map_state != MapState::VIEWABLE {
            return Ok(Err("body is not viewable; layer observation unavailable"));
        }
        let reply = conn
            .get_property(false, body, self.state, AtomEnum::ANY, 0, MAX_ATOMS)?
            .reply()?;
        Ok(self.decode_body(&reply))
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
        let request = self.request(root, body, mutation)?;
        conn.send_event(
            request.propagate,
            request.destination,
            request.mask,
            request.event,
        )?
        .check()?;
        // This acknowledges X protocol processing only, not WM acceptance.
        Ok(())
    }
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
    use super::*;

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
