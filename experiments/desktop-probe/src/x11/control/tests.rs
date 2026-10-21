use super::*;
use std::cell::Cell;
use std::collections::VecDeque;
use x11rb::protocol::xproto::{DestroyNotifyEvent, SelectionClearEvent, PROPERTY_NOTIFY_EVENT};

fn property(sequence: u64) -> PropertyNotifyEvent {
    PropertyNotifyEvent {
        response_type: PROPERTY_NOTIFY_EVENT,
        sequence: sequence as u16,
        window: 10,
        atom: 20,
        time: 1234,
        state: Property::NEW_VALUE,
    }
}

#[test]
fn timestamp_accepts_only_exact_native_notification() {
    let valid = property(42);
    assert_eq!(timestamp_match(&valid, 42, 42, 10, 20).unwrap().get(), 1234);
    let invalid = [
        PropertyNotifyEvent {
            window: 11,
            ..valid
        },
        PropertyNotifyEvent { atom: 21, ..valid },
        PropertyNotifyEvent {
            state: Property::DELETE,
            ..valid
        },
        PropertyNotifyEvent {
            sequence: 41,
            ..valid
        },
        PropertyNotifyEvent {
            response_type: PROPERTY_NOTIFY_EVENT | 0x80,
            ..valid
        },
        PropertyNotifyEvent { time: 0, ..valid },
    ];
    for event in invalid {
        assert_eq!(timestamp_match(&event, 42, 42, 10, 20), None);
    }
    assert_eq!(timestamp_match(&valid, 41, 42, 10, 20), None);
}

#[test]
fn timestamp_full_sequence_preserves_wrap_and_rejects_low_word_alias() {
    for request in [65535, 65536, 65537, u32::MAX as u64, u32::MAX as u64 + 1] {
        let event = property(request);
        assert!(timestamp_match(&event, request, request, 10, 20).is_some());
        assert!(timestamp_match(&event, request + 65536, request, 10, 20).is_none());
        if request >= 65536 {
            assert!(timestamp_match(&event, request - 65536, request, 10, 20).is_none());
        }
    }
}

#[test]
fn timestamp_drains_stale_buffered_events_before_sleep() {
    let now = Instant::now();
    let mut events = VecDeque::from([
        (Event::PropertyNotify(property(41)), 41),
        (
            Event::PropertyNotify(PropertyNotifyEvent {
                time: 0,
                ..property(42)
            }),
            42,
        ),
        (Event::PropertyNotify(property(42)), 42),
    ]);
    let result = wait_events_with(
        now + PROBE_BUDGET,
        || now,
        || Ok(events.pop_front()),
        |_| panic!("valid event already buffered"),
        |event, seq| {
            Ok(match event {
                Event::PropertyNotify(ev) => timestamp_match(&ev, seq, 42, 10, 20),
                _ => None,
            })
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(result.get(), 1234);
    assert!(events.is_empty());
}

#[test]
fn irrelevant_traffic_never_extends_probe_or_delay_deadline() {
    for budget in [PROBE_BUDGET, Duration::from_secs(3)] {
        let start = Instant::now();
        let now = Cell::new(start);
        let calls = Cell::new(0);
        let result = wait_events_with::<()>(
            start + budget,
            || now.get(),
            || {
                calls.set(calls.get() + 1);
                now.set(now.get() + Duration::from_millis(100));
                Ok(Some((Event::PropertyNotify(property(1)), 1)))
            },
            |_| panic!("continuous buffered traffic"),
            |_, _| Ok(None),
        )
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(calls.get(), budget.as_millis() / 100);
        assert_eq!(now.get(), start + budget);
    }
}

#[test]
fn empty_probe_waits_to_fixed_deadline_and_expires() {
    let start = Instant::now();
    let now = Cell::new(start);
    let polls = Cell::new(0);
    let result = wait_events_with::<()>(
        start + PROBE_BUDGET,
        || now.get(),
        || {
            polls.set(polls.get() + 1);
            Ok(None)
        },
        |deadline| {
            assert_eq!(deadline, start + PROBE_BUDGET);
            now.set(deadline);
            Ok(())
        },
        |_, _| panic!("no event"),
    )
    .unwrap();
    assert_eq!(result, None);
    assert_eq!(polls.get(), 1);
}

fn endpoint() -> ControlWindow {
    ControlWindow::candidate(0, 10, 77, 30, 40, 20)
}
fn destroy() -> Event {
    Event::DestroyNotify(DestroyNotifyEvent {
        response_type: 17,
        sequence: 3,
        event: 10,
        window: 10,
    })
}
fn clear(synthetic: bool) -> Event {
    Event::SelectionClear(SelectionClearEvent {
        response_type: 29 | if synthetic { 0x80 } else { 0 },
        sequence: 3,
        time: 77,
        owner: 10,
        selection: 30,
    })
}

#[test]
fn startup_wait_aborts_on_control_destruction_and_host_errors() {
    let mut endpoint = endpoint();
    let start = Instant::now();
    let error = wait_events_with::<()>(
        start + PROBE_BUDGET,
        || start,
        || Ok(Some((destroy(), 3))),
        |_| panic!("must not wait"),
        |event, _| {
            if endpoint.handle_event_with(&event, || panic!("no selection query"))? {
                Err("endpoint lost".into())
            } else {
                Ok(None)
            }
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "endpoint lost");
    assert!(!endpoint.exists);
    for poll_failure in [true, false] {
        let error = wait_events_with::<()>(
            start + PROBE_BUDGET,
            || start,
            || {
                if poll_failure {
                    Err("connection failure".into())
                } else {
                    Ok(None)
                }
            },
            |_| Err("socket failure".into()),
            |_, _| Ok(None),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            if poll_failure {
                "connection failure"
            } else {
                "socket failure"
            }
        );
    }
}

#[test]
fn asynchronous_protocol_error_aborts_wait_before_inspection() {
    let start = Instant::now();
    let error = x11rb::x11_utils::X11Error {
        error_kind: x11rb::protocol::ErrorKind::Window,
        error_code: 3,
        sequence: 1,
        bad_value: 10,
        minor_opcode: 0,
        major_opcode: 18,
        extension_name: None,
        request_name: None,
    };
    let result = wait_events_with::<()>(
        start + PROBE_BUDGET,
        || start,
        || Ok(Some((Event::Error(error.clone()), 1))),
        |_| panic!(),
        |_, _| panic!(),
    )
    .unwrap_err();
    assert!(result.to_string().contains("Asynchronous X11 error"));
}

struct FakeHost {
    now: Instant,
    owners: VecDeque<std::result::Result<u32, &'static str>>,
    timestamps: VecDeque<u32>,
    log: Vec<String>,
    grabbed: bool,
    failure: Option<&'static str>,
    release_failure: bool,
    probe_cost: Duration,
    transaction_cost: Duration,
    deadlines: Vec<Instant>,
}
impl FakeHost {
    fn new(owners: &[u32]) -> Self {
        Self {
            now: Instant::now(),
            owners: owners.iter().map(|v| Ok(*v)).collect(),
            timestamps: VecDeque::from([1234, 2345]),
            log: vec![],
            grabbed: false,
            failure: None,
            release_failure: false,
            probe_cost: Duration::ZERO,
            transaction_cost: Duration::ZERO,
            deadlines: vec![],
        }
    }
}
impl AcquisitionHost for FakeHost {
    fn now(&self) -> Instant {
        self.now
    }
    fn timestamp(&mut self, deadline: Instant) -> Result<NonZeroU32> {
        assert!(!self.grabbed, "timestamp must be outside grab");
        self.log.push("timestamp".into());
        self.deadlines.push(deadline);
        self.now += self.probe_cost;
        if self.failure == Some("timestamp") {
            return Err("timestamp".into());
        }
        NonZeroU32::new(self.timestamps.pop_front().expect("at most two probes"))
            .ok_or_else(|| "unusable zero timestamp".into())
    }
    fn grab(&mut self) -> Result<()> {
        assert!(!self.grabbed);
        self.grabbed = true;
        self.log.push("grab".into());
        if self.failure == Some("grab") {
            Err("grab".into())
        } else {
            Ok(())
        }
    }
    fn owner(&mut self) -> Result<u32> {
        assert!(self.grabbed);
        self.log.push("owner".into());
        self.owners
            .pop_front()
            .expect("unexpected lookup")
            .map_err(Into::into)
    }
    fn claim(&mut self, timestamp: NonZeroU32) -> Result<()> {
        assert!(self.grabbed);
        self.log.push(format!("claim:{}", timestamp.get()));
        if self.failure == Some("claim") {
            Err("claim".into())
        } else {
            Ok(())
        }
    }
    fn ungrab(&mut self) -> Result<()> {
        self.log.push("ungrab".into());
        self.now += self.transaction_cost;
        if self.release_failure {
            Err("release".into())
        } else {
            self.grabbed = false;
            Ok(())
        }
    }
}

#[test]
fn claim_uses_exact_nonzero_timestamp_and_verified_ownership() {
    let mut host = FakeHost::new(&[0, 10]);
    assert_eq!(
        acquire_with(&mut host, 10).unwrap(),
        (Acquisition::Owned, Some(1234))
    );
    assert_eq!(
        host.log,
        [
            "timestamp",
            "grab",
            "owner",
            "claim:1234",
            "owner",
            "ungrab"
        ]
    );
    assert!(!host.grabbed);
}

#[test]
fn existing_owner_refuses_without_claim_or_retry() {
    let mut host = FakeHost::new(&[99]);
    assert_eq!(
        acquire_with(&mut host, 10).unwrap(),
        (Acquisition::Duplicate, None)
    );
    assert_eq!(host.log, ["timestamp", "grab", "owner", "ungrab"]);
}

mod cooperative_contenders {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct SelectionServer {
        owner: u32,
        grabbed_by: Option<u32>,
        timestamp: u32,
        samples: Vec<(u32, u32)>,
        claims: Vec<(u32, u32)>,
        trace: Vec<(u32, &'static str)>,
    }

    struct Contender {
        server: Rc<RefCell<SelectionServer>>,
        control: u32,
        now: Instant,
        before_grab: Option<Box<dyn FnOnce()>>,
    }

    impl Contender {
        fn new(server: &Rc<RefCell<SelectionServer>>, control: u32, now: Instant) -> Self {
            Self {
                server: Rc::clone(server),
                control,
                now,
                before_grab: None,
            }
        }
    }

    impl AcquisitionHost for Contender {
        fn now(&self) -> Instant {
            self.now
        }

        fn timestamp(&mut self, deadline: Instant) -> Result<NonZeroU32> {
            assert!(self.now < deadline);
            let mut server = self.server.borrow_mut();
            assert_eq!(server.grabbed_by, None);
            server.timestamp += 1;
            let timestamp = server.timestamp;
            server.samples.push((self.control, timestamp));
            server.trace.push((self.control, "timestamp"));
            Ok(NonZeroU32::new(timestamp).unwrap())
        }

        fn grab(&mut self) -> Result<()> {
            // Schedule the other real acquisition only after this contender
            // sampled its timestamp, without holding a simulated server grab.
            if let Some(other) = self.before_grab.take() {
                other();
            }
            let mut server = self.server.borrow_mut();
            assert_eq!(server.grabbed_by, None);
            server.grabbed_by = Some(self.control);
            server.trace.push((self.control, "grab"));
            Ok(())
        }

        fn owner(&mut self) -> Result<u32> {
            let mut server = self.server.borrow_mut();
            assert_eq!(server.grabbed_by, Some(self.control));
            server.trace.push((self.control, "owner"));
            Ok(server.owner)
        }

        fn claim(&mut self, timestamp: NonZeroU32) -> Result<()> {
            let mut server = self.server.borrow_mut();
            assert_eq!(server.grabbed_by, Some(self.control));
            assert!(server.samples.contains(&(self.control, timestamp.get())));
            server.claims.push((self.control, timestamp.get()));
            // Like SetSelectionOwner, this permits replacement. Cooperation
            // must come from guarded_claim's lookup, not a fake-host refusal.
            server.owner = self.control;
            server.trace.push((self.control, "claim"));
            Ok(())
        }

        fn ungrab(&mut self) -> Result<()> {
            let mut server = self.server.borrow_mut();
            assert_eq!(server.grabbed_by, Some(self.control));
            server.grabbed_by = None;
            server.trace.push((self.control, "ungrab"));
            Ok(())
        }
    }

    #[test]
    fn two_cooperative_contenders_keep_first_owner_and_release_both_grabs() {
        for (a, b) in [(10, 20), (20, 10)] {
            let server = Rc::new(RefCell::new(SelectionServer::default()));
            let now = Instant::now();
            let mut first = Contender::new(&server, a, now);
            let mut second = Contender::new(&server, b, now);
            let results = [
                acquire_with(&mut first, a).unwrap(),
                acquire_with(&mut second, b).unwrap(),
            ];
            assert_eq!(
                results,
                [
                    (Acquisition::Owned, Some(1)),
                    (Acquisition::Duplicate, None)
                ]
            );
            let server = server.borrow();
            assert_eq!(server.owner, a);
            assert_eq!(server.grabbed_by, None);
            assert_eq!(server.samples, [(a, 1), (b, 2)]);
            assert_eq!(server.claims, [(a, 1)]);
            assert_eq!(
                server.trace,
                [
                    (a, "timestamp"),
                    (a, "grab"),
                    (a, "owner"),
                    (a, "claim"),
                    (a, "owner"),
                    (a, "ungrab"),
                    (b, "timestamp"),
                    (b, "grab"),
                    (b, "owner"),
                    (b, "ungrab"),
                ]
            );
        }
    }

    #[test]
    fn two_cooperative_contenders_recheck_owner_after_earlier_timestamp_sample() {
        for (a, b) in [(10, 20), (20, 10)] {
            let server = Rc::new(RefCell::new(SelectionServer::default()));
            let now = Instant::now();
            let mut prepared_first = Contender::new(&server, a, now);
            let winner = Rc::new(Cell::new(None));
            let winner_result = Rc::clone(&winner);
            let shared_server = Rc::clone(&server);
            prepared_first.before_grab = Some(Box::new(move || {
                assert_eq!(shared_server.borrow().samples, [(a, 1)]);
                let mut other = Contender::new(&shared_server, b, now);
                winner_result.set(Some(acquire_with(&mut other, b).unwrap()));
            }));
            assert_eq!(
                acquire_with(&mut prepared_first, a).unwrap(),
                (Acquisition::Duplicate, None)
            );
            assert_eq!(winner.get(), Some((Acquisition::Owned, Some(2))));
            let server = server.borrow();
            assert_eq!(server.owner, b);
            assert_eq!(server.grabbed_by, None);
            assert_eq!(server.samples, [(a, 1), (b, 2)]);
            assert_eq!(server.claims, [(b, 2)]);
            assert_eq!(
                server.trace,
                [
                    (a, "timestamp"),
                    (b, "timestamp"),
                    (b, "grab"),
                    (b, "owner"),
                    (b, "claim"),
                    (b, "owner"),
                    (b, "ungrab"),
                    (a, "grab"),
                    (a, "owner"),
                    (a, "ungrab"),
                ]
            );
        }
    }
}

#[test]
fn stale_claim_releases_then_samples_fresh_timestamp_before_second_grab() {
    let mut host = FakeHost::new(&[0, 0, 0, 10]);
    assert_eq!(
        acquire_with(&mut host, 10).unwrap(),
        (Acquisition::Owned, Some(2345))
    );
    assert_eq!(
        host.log,
        [
            "timestamp",
            "grab",
            "owner",
            "claim:1234",
            "owner",
            "ungrab",
            "timestamp",
            "grab",
            "owner",
            "claim:2345",
            "owner",
            "ungrab"
        ]
    );
}

#[test]
fn second_unconfirmed_claim_stops_without_third_attempt() {
    let mut host = FakeHost::new(&[0, 0, 0, 0]);
    assert!(acquire_with(&mut host, 10)
        .unwrap_err()
        .to_string()
        .contains("one fresh-timestamp retry"));
    assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 2);
    assert_eq!(host.log.last().unwrap(), "ungrab");
}

#[test]
fn owner_appearing_on_retry_or_verification_refuses() {
    for owners in [&[0, 0, 99][..], &[0, 99][..]] {
        let mut host = FakeHost::new(owners);
        assert_eq!(
            acquire_with(&mut host, 10).unwrap().0,
            Acquisition::Duplicate
        );
        assert_eq!(
            host.log.iter().filter(|s| s.starts_with("claim:")).count(),
            1
        );
        assert!(!host.grabbed);
    }
}

#[test]
fn transaction_errors_always_attempt_release_and_never_retry() {
    for failure in ["grab", "lookup", "claim", "verification"] {
        for release_failure in [false, true] {
            let mut host = FakeHost::new(&[0, 10]);
            host.failure = Some(failure);
            host.release_failure = release_failure;
            if failure == "lookup" {
                host.owners[0] = Err("lookup");
            }
            if failure == "verification" {
                host.owners[1] = Err("verification");
            }
            let error = acquire_with(&mut host, 10).unwrap_err();
            assert_eq!(error.to_string(), failure, "original failure has priority");
            assert_eq!(host.log.last().unwrap(), "ungrab");
            assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 1);
        }
    }
}

#[test]
fn release_failure_blocks_success_duplicate_and_stale_retry() {
    for owners in [&[0, 10][..], &[99][..], &[0, 0][..]] {
        let mut host = FakeHost::new(owners);
        host.release_failure = true;
        assert_eq!(
            acquire_with(&mut host, 10).unwrap_err().to_string(),
            "release"
        );
        assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 1);
    }
}

#[test]
fn zero_or_failed_timestamp_never_enters_transaction() {
    for zero in [false, true] {
        let mut host = FakeHost::new(&[]);
        if zero {
            host.timestamps[0] = 0;
        } else {
            host.failure = Some("timestamp");
        }
        assert!(acquire_with(&mut host, 10).is_err());
        assert_eq!(host.log, ["timestamp"]);
    }
}

#[test]
fn overall_budget_caps_retry_probe_and_prevents_late_claim() {
    let mut host = FakeHost::new(&[0, 0]);
    let start = host.now;
    host.probe_cost = Duration::from_millis(900);
    host.transaction_cost = Duration::from_millis(400);
    assert!(acquire_with(&mut host, 10)
        .unwrap_err()
        .to_string()
        .contains("deadline"));
    assert_eq!(
        host.deadlines,
        [start + PROBE_BUDGET, start + ACQUISITION_BUDGET]
    );
    assert_eq!(host.log.iter().filter(|s| *s == "grab").count(), 1);
    let mut host = FakeHost::new(&[0, 0]);
    host.transaction_cost = ACQUISITION_BUDGET;
    assert!(acquire_with(&mut host, 10).is_err());
    assert_eq!(host.log.iter().filter(|s| *s == "timestamp").count(), 1);
}

#[test]
fn control_construction_is_input_only_without_focus_or_pointer_events() {
    let screen = Screen {
        root: 55,
        root_visual: 66,
        ..Screen::default()
    };
    let request = control_request(10, &screen);
    assert_eq!(
        (
            request.depth,
            request.border_width,
            request.width,
            request.height
        ),
        (0, 0, 1, 1)
    );
    assert_eq!(
        (request.parent, request.visual, request.class),
        (55, 66, WindowClass::INPUT_ONLY)
    );
    assert_eq!(
        request.value_list.event_mask,
        Some(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY)
    );
    // Creating a window leaves it unmapped; the endpoint owns no map operation.
    let endpoint = endpoint();
    assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Starting);
    assert_eq!(endpoint.descriptor.body_xid, None);
    assert!(!endpoint.owns_selection);
}

#[test]
fn descriptor_publication_requires_ownership_body_and_checked_write() {
    let mut endpoint = endpoint();
    let body = OwnerDescriptor {
        body_xid: Some(88),
        ..endpoint.descriptor
    };
    assert!(endpoint
        .publish_with(body, |_| panic!("not acquired"))
        .is_err());
    endpoint.owns_selection = true;
    let ready_without_body = OwnerDescriptor {
        lifecycle: Lifecycle::Ready,
        ..endpoint.descriptor
    };
    assert!(endpoint
        .publish_with(ready_without_body, |_| panic!("missing body"))
        .is_err());
    endpoint
        .publish_with(body, |descriptor| {
            assert_eq!(descriptor.lifecycle, Lifecycle::Starting);
            assert_eq!(descriptor.encode().unwrap()[6], 88);
            Ok(())
        })
        .unwrap();
    let ready = OwnerDescriptor {
        lifecycle: Lifecycle::Ready,
        ..body
    };
    assert!(endpoint
        .publish_with(ready, |_| Err("write failed".into()))
        .is_err());
    assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Starting);
    endpoint.publish_with(ready, |_| Ok(())).unwrap();
    endpoint
        .publish_with(ready, |_| panic!("Ready must be idempotent"))
        .unwrap();
}

#[test]
fn stale_and_fabricated_selection_clear_requery_actual_owner() {
    let mut endpoint = endpoint();
    endpoint.owns_selection = true;
    for synthetic in [false, true] {
        let queries = Cell::new(0);
        assert!(!endpoint
            .handle_event_with(&clear(synthetic), || {
                queries.set(1);
                Ok(10)
            })
            .unwrap());
        assert_eq!(queries.get(), 1);
        assert!(endpoint.owns_selection);
    }
    assert!(endpoint
        .handle_event_with(&clear(false), || Err("lookup".into()))
        .is_err());
}

#[test]
fn actual_loss_is_terminal_and_closing_cannot_touch_replacement() {
    for owner in [0, 99] {
        let mut endpoint = endpoint();
        endpoint.owns_selection = true;
        assert!(endpoint
            .handle_event_with(&clear(false), || Ok(owner))
            .unwrap());
        assert!(!endpoint.owns_selection);
        assert!(!endpoint
            .handle_event_with(&clear(false), || panic!("no reacquisition"))
            .unwrap());
        endpoint
            .closing_with(|| panic!("lost"), |_| panic!("do not publish"))
            .unwrap();
    }
}

#[test]
fn destruction_is_native_only_and_satisfies_release_obligation() {
    let mut endpoint = endpoint();
    endpoint.owns_selection = true;
    let mut synthetic = destroy();
    if let Event::DestroyNotify(ev) = &mut synthetic {
        ev.response_type |= 0x80;
    }
    assert!(!endpoint.handle_event_with(&synthetic, || panic!()).unwrap());
    assert!(endpoint.exists);
    assert!(endpoint.handle_event_with(&destroy(), || panic!()).unwrap());
    assert!(!endpoint.exists);
    assert!(!endpoint.owns_selection);
    endpoint
        .resource
        .release_with(|| panic!("already externally destroyed"))
        .unwrap();
    endpoint.closing_with(|| panic!(), |_| panic!()).unwrap();
}

#[test]
fn closing_rechecks_pending_loss_and_preserves_failed_write_truth() {
    let mut endpoint = endpoint();
    endpoint.owns_selection = true;
    endpoint
        .closing_with(|| Ok(99), |_| panic!("replacement owner"))
        .unwrap();
    assert!(!endpoint.owns_selection);
    let mut endpoint = self::endpoint();
    endpoint.owns_selection = true;
    assert!(endpoint
        .closing_with(|| Ok(10), |_| Err("write".into()))
        .is_err());
    assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Starting);
    endpoint
        .closing_with(
            || Ok(10),
            |descriptor| {
                assert_eq!(descriptor.lifecycle, Lifecycle::Closing);
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn unacknowledged_control_destruction_is_not_repeated_or_reported_successful() {
    let endpoint = endpoint();
    assert!(endpoint
        .resource
        .release_with(|| Err("destroy failed".into()))
        .is_err());
    assert!(endpoint
        .resource
        .release_with(|| panic!("unknown prior result"))
        .is_err());
}
#[test]
fn delay_ignores_stale_clear_then_exits_on_verified_loss() {
    let mut endpoint = endpoint();
    endpoint.owns_selection = true;
    let start = Instant::now();
    let mut events = VecDeque::from([(clear(true), 1), (clear(false), 2)]);
    let mut owners = VecDeque::from([10, 99]);
    let error = wait_events_with::<()>(
        start + Duration::from_secs(10),
        || start,
        || Ok(events.pop_front()),
        |_| panic!("loss already buffered"),
        |event, _| {
            if endpoint.handle_event_with(&event, || Ok(owners.pop_front().unwrap()))? {
                return Err("endpoint lost".into());
            }
            Ok(None)
        },
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "endpoint lost");
    assert!(owners.is_empty());
    assert!(!endpoint.owns_selection);
    assert_eq!(endpoint.descriptor.body_xid, None);
}

#[test]
fn unrelated_lifecycle_events_do_not_query_or_end_ownership() {
    let mut endpoint = endpoint();
    endpoint.owns_selection = true;
    let Event::SelectionClear(valid) = clear(false) else {
        unreachable!()
    };
    for event in [
        Event::SelectionClear(SelectionClearEvent { owner: 11, ..valid }),
        Event::SelectionClear(SelectionClearEvent {
            selection: 31,
            ..valid
        }),
        Event::DestroyNotify(DestroyNotifyEvent {
            response_type: 17,
            sequence: 1,
            event: 10,
            window: 11,
        }),
    ] {
        assert!(!endpoint.is_lifecycle_event(&event));
        assert!(!endpoint
            .handle_event_with(&event, || panic!("unrelated owner query"))
            .unwrap());
        assert!(endpoint.owns_selection);
    }
}
