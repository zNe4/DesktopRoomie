use super::*;
use std::collections::VecDeque;
use x11rb::protocol::xproto::{DestroyNotifyEvent, SelectionClearEvent};

const ROOT: u32 = 55;
const CONTROL: u32 = 10;
const REPLY_WINDOW: u32 = 90;
const ID: u64 = 0x1234_5678_9abc_def0;
fn atoms() -> TransportAtoms {
    TransportAtoms {
        request: 40,
        reply: 41,
        caller: 42,
    }
}
fn owner() -> OwnerDescriptor {
    OwnerDescriptor {
        screen_num: 0,
        control_xid: CONTROL,
        epoch: 77,
        lifecycle: Lifecycle::Ready,
        body_xid: Some(88),
    }
}
fn request(command: Command) -> Request {
    Request {
        command,
        request_id: ID,
        reply_xid: REPLY_WINDOW,
        epoch: 77,
    }
}
fn marker(command: Command) -> CallerMarker {
    CallerMarker {
        destination_desktop: if matches!(command, Command::BringHere | Command::Recover) {
            7
        } else {
            control::NO_DESTINATION
        },
        control_xid: CONTROL,
        epoch: 77,
        request_id: ID,
        command,
    }
}
fn evidence(words: &[u32]) -> EndpointEvidence {
    EndpointEvidence {
        root: ROOT,
        parent: ROOT,
        class: WindowClass::INPUT_ONLY,
        map_state: MapState::UNMAPPED,
        property: GetPropertyReply {
            format: 32,
            type_: AtomEnum::CARDINAL.into(),
            value_len: words.len() as u32,
            value: words.iter().flat_map(|word| word.to_ne_bytes()).collect(),
            ..GetPropertyReply::default()
        },
    }
}
fn cardinal(word: u32) -> GetPropertyReply {
    GetPropertyReply {
        format: 32,
        type_: AtomEnum::CARDINAL.into(),
        length: 1,
        value_len: 1,
        value: word.to_ne_bytes().to_vec(),
        ..Default::default()
    }
}
fn response() -> TerminalResponse {
    TerminalResponse {
        status: Status::Failed,
        request_id: ID,
        control_xid: CONTROL,
        detail: control::preflight_detail(Reason::Unsupported, Stage::Preflight),
    }
}
fn terminal() -> Event {
    Event::ClientMessage(reply_event(atoms(), REPLY_WINDOW, response()).unwrap())
}
fn destroyed(synthetic: bool) -> Event {
    Event::DestroyNotify(DestroyNotifyEvent {
        response_type: 17 | if synthetic { 0x80 } else { 0 },
        sequence: 0,
        event: CONTROL,
        window: CONTROL,
    })
}
struct Fake {
    now: Instant,
    owner: u32,
    owners: VecDeque<u32>,
    evidence: VecDeque<Option<EndpointEvidence>>,
    events: VecDeque<Event>,
    sent: Vec<ClientMessageEvent>,
    marker: Option<CallerMarker>,
    log: Vec<&'static str>,
    waits: Vec<Instant>,
    cost: Option<(&'static str, Duration)>,
    step_costs: Vec<(&'static str, usize, Duration)>,
    id_elapsed: std::rc::Rc<std::cell::Cell<Duration>>,
    endless_events: bool,
    watch_ok: bool,
    send_ok: bool,
    failure: Option<&'static str>,
    caller_pair: (u32, u32),
    discovery_pair: Option<(u32, u32)>,
    caller_snapshots:
        VecDeque<std::result::Result<(GetPropertyReply, GetPropertyReply), DestinationError>>,
}
impl Fake {
    fn new() -> Self {
        Self {
            now: Instant::now(),
            owner: CONTROL,
            owners: VecDeque::new(),
            evidence: VecDeque::new(),
            events: VecDeque::new(),
            sent: vec![],
            marker: None,
            log: vec![],
            waits: vec![],
            cost: None,
            step_costs: vec![],
            id_elapsed: Default::default(),
            endless_events: false,
            watch_ok: true,
            send_ok: true,
            failure: None,
            caller_pair: (9, 7),
            discovery_pair: None,
            caller_snapshots: VecDeque::new(),
        }
    }
    fn call(&mut self, name: &'static str) -> Result<()> {
        self.log.push(name);
        if let Some((target, cost)) = self.cost {
            if target == name {
                self.now += cost;
            }
        }
        let occurrence = self.log.iter().filter(|&&step| step == name).count();
        for &(step, nth, elapsed) in &self.step_costs {
            if step == name && nth == occurrence {
                self.now += elapsed;
            }
        }
        if self.failure == Some(name) {
            Err(name.into())
        } else {
            Ok(())
        }
    }
    fn route(
        &mut self,
        active: &mut Option<ActiveOperation>,
        lifecycle: Lifecycle,
        busy: bool,
        event: &ClientMessageEvent,
    ) -> Result<bool> {
        route_request_with(
            self,
            OwnerDescriptor {
                lifecycle,
                ..owner()
            },
            ROOT,
            atoms(),
            Some(99),
            busy,
            active,
            event,
        )
    }
    fn call_command(&mut self) -> Result<CallerOutcome> {
        caller_with(self, ROOT, 0, Command::Show, || Ok(ID))
    }
    fn read_caller(&mut self) -> std::result::Result<CallerDesktop, DestinationError> {
        super::super::monitors::destination::capture_caller_with(|| {
            self.call("caller count")
                .map_err(|e| DestinationError::Unverifiable(e.to_string()))?;
            self.call("caller current")
                .map_err(|e| DestinationError::Unverifiable(e.to_string()))?;
            self.caller_snapshots
                .pop_front()
                .unwrap_or_else(|| Ok((cardinal(self.caller_pair.0), cardinal(self.caller_pair.1))))
        })
    }
}
impl PeerHost for Fake {
    fn now(&self) -> Instant {
        self.now + self.id_elapsed.get()
    }
    fn owner(&mut self) -> Result<u32> {
        self.call("owner")?;
        if let Some(pair) = self.discovery_pair {
            self.caller_pair = pair;
        }
        Ok(self.owners.pop_front().unwrap_or(self.owner))
    }
    fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>> {
        self.call("evidence")?;
        if let Some(evidence) = self.evidence.pop_front() {
            return Ok(evidence);
        }
        Ok(Some(if window == CONTROL {
            evidence(&owner().encode().unwrap())
        } else {
            assert_eq!(property, atoms().caller);
            evidence(&marker(Command::Show).encode().unwrap())
        }))
    }
    fn send(&mut self, event: ClientMessageEvent) -> Result<bool> {
        self.sent.push(event);
        self.call("send")?;
        Ok(self.send_ok)
    }
}
impl CallerHost for Fake {
    fn capture_desktop(&mut self) -> std::result::Result<CallerDesktop, DestinationError> {
        self.call("capture")
            .map_err(|e| DestinationError::Unverifiable(e.to_string()))?;
        self.read_caller()
    }
    fn revalidate_desktop(
        &mut self,
        captured: CallerDesktop,
    ) -> std::result::Result<(), DestinationError> {
        self.call("revalidate")
            .map_err(|e| DestinationError::Unverifiable(e.to_string()))?;
        captured.revalidate(self.read_caller()?)
    }
    fn atoms(&mut self, _: u32) -> Result<(u32, TransportAtoms)> {
        self.call("atoms")?;
        Ok((30, atoms()))
    }
    fn watch(&mut self, _: u32) -> Result<bool> {
        self.call("watch")?;
        Ok(self.watch_ok)
    }
    fn create_reply(&mut self, marker: CallerMarker) -> Result<u32> {
        self.call("create reply")?;
        assert!(
            self.marker.replace(marker).is_none(),
            "one immutable marker only"
        );
        self.call("marker write")?;
        Ok(REPLY_WINDOW)
    }
    fn next(&mut self) -> Result<Option<Event>> {
        self.call("next")?;
        Ok(self.events.pop_front().or_else(|| {
            self.endless_events.then(|| {
                let mut event = reply_event(atoms(), REPLY_WINDOW, response()).unwrap();
                event.type_ += 1;
                Event::ClientMessage(event)
            })
        }))
    }
    fn wait(&mut self, deadline: Instant) -> Result<()> {
        self.call("wait")?;
        self.waits.push(deadline);
        self.now = deadline - self.id_elapsed.get();
        Ok(())
    }
    fn cleanup(&mut self) -> Result<()> {
        self.call("cleanup")
    }
}

#[test]
fn g_shutdown_at_every_progression_snapshot_replies_once_and_cleans_pending_layer() {
    for (name, op, mut layers) in crate::show_tests::g3_hardening::lifecycle_fixtures() {
        for peer_failure in [0, 1, 2] {
            let mut h = Fake::new();
            if peer_failure == 1 {
                h.send_ok = false;
            } else if peer_failure == 2 {
                h.failure = Some("send");
            }
            let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
            endpoint.transport = atoms();
            endpoint.descriptor = owner();
            endpoint.owns_selection = true;
            endpoint.active = Some(op);
            endpoint.closing_with(|| Ok(CONTROL), |_| Ok(())).unwrap();
            let result = endpoint.finish_remote_with(&mut h);
            assert_eq!(result.is_err(), peer_failure == 2, "{name}");
            if peer_failure == 2 {
                assert_eq!(
                    crate::finish_probe_with(Err("original G native error".into()), || result)
                        .unwrap_err()
                        .to_string(),
                    "original G native error"
                );
            }
            endpoint.finish_remote_with(&mut h).unwrap();
            assert!(endpoint.active.is_none());
            assert_eq!(h.sent.len(), 1);
            let response = TerminalResponse::decode(&h.sent[0].data.as_data32()).unwrap();
            assert_eq!(
                response.status,
                if op.bring_top.restoration_confirmed {
                    Status::Partial
                } else {
                    Status::Failed
                },
                "{name}"
            );
            assert_eq!(
                (response.detail.reason, response.detail.stage),
                (Reason::Closing, Stage::Shutdown)
            );
            assert_eq!(
                response.detail.progress,
                control::Progress {
                    host_mutation_dispatched: op.detail.progress.host_mutation_dispatched,
                    ..control::Progress::default()
                }
            );
            assert_eq!(response.request_id, op.request.request_id);
            assert_eq!(h.sent[0].type_, atoms().reply);
            assert_eq!(h.log.iter().filter(|s| **s == "send").count(), 1);
        }
        let pending = layers.pending().is_some();
        assert_eq!(
            crate::abort_layer(&mut layers, "shutdown").is_some(),
            pending
        );
        assert!(crate::abort_layer(&mut layers, "shutdown again").is_none());
        assert!(layers.pending().is_none());
    }
}
#[test]
fn g_selection_or_endpoint_loss_across_phases_never_continues_under_replacement() {
    use x11rb::protocol::xproto::SelectionClearEvent;
    for (name, op, mut layers) in crate::show_tests::g3_hardening::lifecycle_fixtures() {
        for destroyed_endpoint in [false, true] {
            let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
            endpoint.transport = atoms();
            endpoint.descriptor = owner();
            endpoint.owns_selection = true;
            endpoint.active = Some(op);
            let event = if destroyed_endpoint {
                destroyed(false)
            } else {
                Event::SelectionClear(SelectionClearEvent {
                    owner: CONTROL,
                    selection: 50,
                    ..Default::default()
                })
            };
            assert!(
                endpoint
                    .handle_event_with(&event, || Ok(CONTROL + 1))
                    .unwrap(),
                "{name}"
            );
            assert!(!endpoint.owns_selection);
            assert_eq!(
                endpoint
                    .handle_event_with(&event, || panic!(
                        "no reacquisition or second ownership query"
                    ))
                    .unwrap(),
                destroyed_endpoint
            );
            let mut h = Fake::new();
            endpoint.finish_remote_with(&mut h).unwrap();
            endpoint.finish_remote_with(&mut h).unwrap();
            let reply = TerminalResponse::decode(&h.sent[0].data.as_data32()).unwrap();
            assert_eq!(h.sent.len(), 1);
            assert_eq!(
                (reply.detail.reason, reply.detail.stage),
                (Reason::OwnershipLost, Stage::Shutdown)
            );
            assert_eq!(
                reply.status,
                if op.bring_top.restoration_confirmed {
                    Status::Partial
                } else {
                    Status::Failed
                }
            );
            assert!(!reply.detail.progress.not_minimized_confirmed);
            assert!(!reply.detail.progress.above_confirmed);
            endpoint
                .closing_with(
                    || panic!("lost owner must not query/reclaim"),
                    |_| panic!("must not publish to replacement"),
                )
                .unwrap();
            let _ = crate::abort_layer(&mut layers, "lost owner");
            assert!(layers.pending().is_none());
        }
    }
}
#[test]
fn g_caller_disappearance_and_reply_loss_do_not_undo_host_progress() {
    for (_, op, _) in crate::show_tests::g3_hardening::lifecycle_fixtures()
        .into_iter()
        .filter(|(name, _, _)| *name == "layer-confirmed-before-reply")
    {
        let mut h = Fake::new();
        h.send_ok = false;
        let response = TerminalResponse {
            status: if op.bring_top.layer_checked {
                Status::Success
            } else {
                Status::Partial
            },
            request_id: op.request.request_id,
            control_xid: CONTROL,
            detail: op.detail,
        };
        let mut active = Some(op);
        finish_active(&mut h, atoms(), &mut active, response).unwrap();
        assert!(active.is_none());
        assert_eq!(h.sent.len(), 1);
        assert!(op.bring_top.restoration_confirmed);
        assert!(op.bring_top.layer_dispatched);
    }
    let mut h = Fake::new();
    h.evidence.push_back(None);
    let mut active = None;
    h.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::BringTop)).unwrap(),
    )
    .unwrap();
    assert!(active.is_none());
    assert!(h.sent.is_empty());
}
#[test]
fn g_caller_death_reply_order_replacement_and_traffic_keep_five_second_budget() {
    for events in [
        vec![destroyed(false)],
        vec![destroyed(false), terminal()],
        vec![terminal(), destroyed(false)],
    ] {
        let expected_terminal = events.len() == 2;
        let mut h = Fake::new();
        h.events = events.into();
        let outcome = caller_with(&mut h, ROOT, 0, Command::BringTop, || Ok(ID)).unwrap();
        assert_eq!(
            matches!(outcome, CallerOutcome::Terminal(_)),
            expected_terminal
        );
        if !expected_terminal {
            assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
        }
        assert_eq!(h.sent.len(), 1);
        assert!(h.waits.is_empty());
    }
    let mut h = Fake::new();
    h.cost = Some(("next", Duration::from_millis(10)));
    h.endless_events = true;
    let t0 = h.now;
    assert_eq!(
        caller_with(&mut h, ROOT, 0, Command::BringTop, || Ok(ID))
            .unwrap()
            .exit_code(),
        ExitCode::OutcomeUnknown
    );
    assert_eq!(h.now, t0 + Duration::from_millis(5000));
    assert_eq!(h.sent.len(), 1);
    assert!(h.waits.is_empty());
    for replacement in [0, CONTROL + 1] {
        let mut h = Fake::new();
        h.owners = VecDeque::from([CONTROL, CONTROL, replacement]);
        if replacement != 0 {
            h.evidence = VecDeque::from([
                Some(evidence(&owner().encode().unwrap())),
                Some(evidence(
                    &OwnerDescriptor {
                        control_xid: replacement,
                        ..owner()
                    }
                    .encode()
                    .unwrap(),
                )),
            ]);
        }
        assert!(!matches!(
            caller_with(&mut h, ROOT, 0, Command::BringTop, || Ok(ID)).unwrap(),
            CallerOutcome::Terminal(_)
        ));
        assert!(h.sent.is_empty());
    }
    let mut h = Fake::new();
    h.send_ok = false;
    assert_eq!(
        caller_with(&mut h, ROOT, 0, Command::BringTop, || Ok(ID))
            .unwrap()
            .exit_code(),
        ExitCode::OutcomeUnknown
    );
    assert_eq!(h.sent.len(), 1);
}
#[test]
fn g_stale_reply_ids_and_epoch_change_never_redirect_or_resend() {
    let mut h = Fake::new();
    let mut stale = response();
    stale.request_id = ID + 1;
    h.events.push_back(Event::ClientMessage(
        reply_event(atoms(), REPLY_WINDOW, stale).unwrap(),
    ));
    h.events
        .push_back(Event::ClientMessage(ClientMessageEvent::new(
            32,
            REPLY_WINDOW,
            atoms().reply + 1,
            [0; 5],
        )));
    h.events.push_back(terminal());
    h.cost = Some(("next", Duration::from_millis(10)));
    assert_eq!(
        caller_with(&mut h, ROOT, 0, Command::BringTop, || Ok(ID)).unwrap(),
        CallerOutcome::Terminal(response())
    );
    assert_eq!(h.sent.len(), 1);
    assert_eq!(h.log.iter().filter(|s| **s == "next").count(), 3);
    let mut h = Fake::new();
    h.evidence = VecDeque::from([
        Some(evidence(&owner().encode().unwrap())),
        Some(evidence(
            &OwnerDescriptor {
                epoch: 78,
                ..owner()
            }
            .encode()
            .unwrap(),
        )),
    ]);
    assert_eq!(
        caller_with(&mut h, ROOT, 0, Command::BringTop, || Ok(ID)).unwrap(),
        CallerOutcome::Protocol
    );
    assert!(h.sent.is_empty());
    let mut h = Fake::new();
    let mut active = None;
    let mut req = request(Command::BringTop);
    req.epoch += 1;
    h.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, req).unwrap(),
    )
    .unwrap();
    assert!(active.is_none());
    assert!(h.sent.is_empty());
}
#[test]
fn caller_reply_returned_at_or_after_fixed_deadline_is_unknown_without_resend() {
    for command in [
        Command::Hide,
        Command::Show,
        Command::BringTop,
        Command::BringHere,
        Command::Recover,
    ] {
        for delta in [-1_i64, 0, 1] {
            let mut h = Fake::new();
            let budget = caller_budget(command);
            let cost = if delta < 0 {
                budget - Duration::from_nanos(1)
            } else {
                budget + Duration::from_nanos(delta as u64)
            };
            h.cost = Some(("next", cost));
            h.events.push_back(terminal());
            let outcome = caller_with(&mut h, ROOT, 0, command, || Ok(ID)).unwrap();
            assert_eq!(
                matches!(outcome, CallerOutcome::Terminal(_)),
                delta < 0,
                "command={command:?} delta={delta}"
            );
            if delta >= 0 {
                assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
            }
            assert_eq!(h.sent.len(), 1);
        }
    }
}
#[test]
fn command_specific_caller_budget_starts_before_discovery_and_never_resends() {
    for (command, millis) in [
        (Command::Hide, 4000),
        (Command::Show, 4000),
        (Command::BringTop, 5000),
    ] {
        assert_eq!(caller_budget(command), Duration::from_millis(millis));
        let mut h = Fake::new();
        let t0 = h.now;
        h.cost = Some(("atoms", Duration::from_millis(700)));
        let outcome = caller_with(&mut h, ROOT, 0, command, || Ok(ID)).unwrap();
        assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
        assert_eq!(h.waits, [t0 + Duration::from_millis(millis)]);
        assert_eq!(h.sent.len(), 1);
        assert_eq!(
            Request::decode(&h.sent[0].data.as_data32())
                .unwrap()
                .command,
            command
        );
        let mut h = Fake::new();
        h.cost = Some(("atoms", Duration::from_millis(millis)));
        assert_eq!(
            caller_with(&mut h, ROOT, 0, command, || Ok(ID))
                .unwrap()
                .exit_code(),
            ExitCode::OutcomeUnknown
        );
        assert!(h.sent.is_empty());
    }
}

#[test]
fn caller_timeout_does_not_cancel_admitted_show_or_make_vanished_reply_fatal() {
    struct CoupledShow {
        transport: Fake,
        active: Option<ActiveOperation>,
        show: crate::show_tests::CallerTimeoutShow,
        reply_exists: bool,
        admitted_at: Option<Instant>,
        completed: Option<(Instant, control::OperationResult)>,
    }

    impl CoupledShow {
        fn service_owner(&mut self) -> Result<()> {
            let Some(operation) = self.active.as_mut() else {
                return Ok(());
            };
            let original_deadline = operation.deadline;
            let result = self.show.service_at(operation, self.transport.now)?;
            assert_eq!(operation.deadline, original_deadline);
            if let Some(result) = result {
                self.show.assert_completed(result);
                assert!(self.transport.now < original_deadline);
                let response = TerminalResponse {
                    status: result.status(),
                    detail: result.detail(),
                    request_id: operation.request.request_id,
                    control_xid: CONTROL,
                };
                self.completed = Some((self.transport.now, result));
                let mut active = self.active.take();
                let sent = finish_active(self, atoms(), &mut active, response);
                assert!(active.is_none());
                sent?;
            }
            Ok(())
        }
    }

    impl PeerHost for CoupledShow {
        fn now(&self) -> Instant {
            self.transport.now
        }
        fn owner(&mut self) -> Result<u32> {
            self.transport.owner()
        }
        fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>> {
            if window == REPLY_WINDOW && !self.reply_exists {
                return Ok(None);
            }
            self.transport.evidence(window, property)
        }
        fn send(&mut self, event: ClientMessageEvent) -> Result<bool> {
            self.transport.send(event)?;
            if event.type_ == atoms().request {
                assert!(self.active.is_none(), "one request, no resend/cancellation");
                assert!(self.reply_exists);
                assert_eq!(self.transport.marker, Some(marker(Command::Show)));
                self.transport
                    .route(&mut self.active, Lifecycle::Ready, false, &event)?;
                assert_eq!(self.active.unwrap().request, request(Command::Show));
                assert!(self.admitted_at.replace(self.transport.now).is_none());
                self.service_owner()?;
                assert!(
                    self.completed.is_none(),
                    "Map acknowledgement is not completion"
                );
                return Ok(true);
            }
            assert_eq!((event.type_, event.window), (atoms().reply, REPLY_WINDOW));
            assert!(!self.reply_exists);
            let (_, result) = self.completed.expect("host completion precedes reply loss");
            assert_eq!(result.status(), Status::Success);
            // Exercise the production typed peer-error boundary, not a
            // generic send failure or a manually fabricated success result.
            Ok(peer_result::<()>(
                Err(ReplyError::X11Error(x11rb::x11_utils::X11Error {
                    error_kind: ErrorKind::Window,
                    error_code: 3,
                    sequence: 1,
                    bad_value: REPLY_WINDOW,
                    minor_opcode: 0,
                    major_opcode: 25,
                    extension_name: None,
                    request_name: None,
                })),
                REPLY_WINDOW,
            )?
            .is_some())
        }
    }

    impl CallerHost for CoupledShow {
        fn capture_desktop(&mut self) -> std::result::Result<CallerDesktop, DestinationError> {
            panic!("Show must not capture caller workspace")
        }
        fn revalidate_desktop(
            &mut self,
            _: CallerDesktop,
        ) -> std::result::Result<(), DestinationError> {
            panic!("Show must not revalidate caller workspace")
        }
        fn atoms(&mut self, screen_num: u32) -> Result<(u32, TransportAtoms)> {
            self.transport.atoms(screen_num)
        }
        fn watch(&mut self, owner: u32) -> Result<bool> {
            self.transport.watch(owner)
        }
        fn create_reply(&mut self, marker: CallerMarker) -> Result<u32> {
            let reply = self.transport.create_reply(marker)?;
            self.reply_exists = true;
            Ok(reply)
        }
        fn next(&mut self) -> Result<Option<Event>> {
            self.transport.next()
        }
        fn wait(&mut self, deadline: Instant) -> Result<()> {
            self.show.assert_pending();
            self.transport.wait(deadline)?;
            self.service_owner()?;
            assert!(self.active.is_some());
            Ok(())
        }
        fn cleanup(&mut self) -> Result<()> {
            self.transport.cleanup()?;
            self.reply_exists = false;
            Ok(())
        }
    }

    let mut transport = Fake::new();
    let t0 = transport.now;
    transport.cost = Some(("atoms", Duration::from_millis(3000)));
    let mut coupled = CoupledShow {
        transport,
        active: None,
        show: crate::show_tests::CallerTimeoutShow::new(t0),
        reply_exists: false,
        admitted_at: None,
        completed: None,
    };
    let outcome = caller_with(&mut coupled, ROOT, 0, Command::Show, || Ok(ID)).unwrap();
    assert_eq!(outcome, CallerOutcome::Unknown("caller deadline expired"));
    assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
    assert_eq!(outcome.exit_code() as u32, 8);
    let caller_deadline = t0 + Duration::from_millis(4000);
    assert_eq!(coupled.transport.waits, [caller_deadline]);
    assert_eq!(coupled.transport.now, caller_deadline);
    assert!(!coupled.reply_exists);
    assert_eq!(coupled.admitted_at, Some(t0 + Duration::from_millis(3000)));
    let pending = coupled
        .active
        .expect("caller timeout must not cancel admitted work");
    let owner_deadline = t0 + Duration::from_millis(5500);
    assert_eq!(pending.deadline, owner_deadline);
    assert!(caller_deadline < owner_deadline);
    assert_eq!(pending.show.phase, control::ShowPhase::RestoreRequested);
    assert!(coupled.completed.is_none());
    assert_eq!(coupled.transport.sent.len(), 1);
    coupled.show.assert_pending();

    coupled.transport.now = t0 + Duration::from_millis(4200);
    coupled.service_owner().unwrap();
    assert_eq!(coupled.active, Some(pending));
    coupled.show.assert_pending();
    coupled.transport.now = t0 + Duration::from_millis(4500);
    coupled
        .show
        .observe_restored(coupled.active.as_mut().unwrap());
    coupled.service_owner().unwrap(); // expected reply BadWindow is nonfatal
    assert!(coupled.active.is_none());
    let (completed_at, result) = coupled.completed.unwrap();
    assert_eq!(completed_at, t0 + Duration::from_millis(4500));
    assert!(caller_deadline < completed_at && completed_at < owner_deadline);
    coupled.show.assert_completed(result);
    assert_eq!(coupled.transport.sent.len(), 2); // one request and one terminal attempt
    assert_eq!(
        Request::decode(&coupled.transport.sent[0].data.as_data32()).unwrap(),
        pending.request
    );
    let terminal = TerminalResponse::decode(&coupled.transport.sent[1].data.as_data32()).unwrap();
    assert_eq!(terminal.status, Status::Success);
    assert_eq!(terminal.detail, result.detail());
    assert_eq!((terminal.control_xid, terminal.request_id), (CONTROL, ID));

    let completed_trace = coupled.show.trace().to_vec();
    coupled.transport.now = owner_deadline + Duration::from_millis(100);
    coupled.service_owner().unwrap();
    assert!(coupled.active.is_none());
    assert_eq!(coupled.show.trace(), completed_trace);
    assert_eq!(coupled.transport.sent.len(), 2);
    assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
}

#[test]
fn bring_top_duplicates_busy_and_single_terminal_use_production_transport() {
    let mut h = Fake::new();
    let mut active = None;
    let req = request(Command::BringTop);
    for _ in 0..2 {
        h.evidence
            .push_back(Some(evidence(&marker(Command::BringTop).encode().unwrap())));
        h.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, req).unwrap(),
        )
        .unwrap();
    }
    let original = active;
    assert!(h.sent.is_empty());
    for command in [Command::Hide, Command::Show, Command::BringTop] {
        let marker = CallerMarker {
            command,
            request_id: ID + 1,
            ..marker(command)
        };
        let other = Request {
            command,
            request_id: ID + 1,
            ..req
        };
        h.evidence
            .push_back(Some(evidence(&marker.encode().unwrap())));
        h.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, other).unwrap(),
        )
        .unwrap();
        let terminal = TerminalResponse::decode(&h.sent.last().unwrap().data.as_data32()).unwrap();
        assert_eq!(terminal.status, Status::Busy);
        assert!(!terminal.detail.progress.host_mutation_dispatched);
        assert_eq!(active, original);
    }
    finish_active(&mut h, atoms(), &mut active, response()).unwrap();
    assert!(active.is_none());
    assert_eq!(h.sent.len(), 4);
}
#[test]
fn bring_top_local_layer_pending_is_busy_without_admission() {
    let mut h = Fake::new();
    h.evidence
        .push_back(Some(evidence(&marker(Command::BringTop).encode().unwrap())));
    let mut active = None;
    h.route(
        &mut active,
        Lifecycle::Ready,
        true,
        &request_event(atoms(), CONTROL, request(Command::BringTop)).unwrap(),
    )
    .unwrap();
    assert!(active.is_none());
    let response = TerminalResponse::decode(&h.sent[0].data.as_data32()).unwrap();
    assert_eq!(response.status, Status::Busy);
    assert!(!response.detail.progress.host_mutation_dispatched);
}
#[test]
fn guarded_here_shutdown_at_every_service_phase_replies_once_with_history_and_dispatch() {
    for (phase, operation) in crate::here_service_tests::lifecycle_snapshots() {
        for owned in [false, true] {
            for fail_reply in [false, true] {
                let mut host = Fake::new();
                host.failure = fail_reply.then_some("send");
                let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
                endpoint.transport = atoms();
                endpoint.descriptor = owner();
                endpoint.owns_selection = owned;
                endpoint.active = Some(operation);
                let history = operation.placement.proof.history;
                let dispatched = operation.detail.progress.host_mutation_dispatched;
                assert_eq!(
                    endpoint.finish_remote_with(&mut host).is_err(),
                    fail_reply,
                    "{phase}"
                );
                assert!(endpoint.active().is_none());
                endpoint.finish_remote_with(&mut host).unwrap();
                assert_eq!(host.sent.len(), 1);
                let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
                assert_eq!(
                    reply.status,
                    if history.relocation_confirmed {
                        Status::Partial
                    } else {
                        Status::Failed
                    }
                );
                assert_eq!(reply.detail.stage, Stage::Shutdown);
                assert_eq!(
                    reply.detail.reason,
                    if owned {
                        Reason::Closing
                    } else {
                        Reason::OwnershipLost
                    }
                );
                assert_eq!(reply.detail.placement.history, history);
                assert_eq!(
                    reply.detail.placement.current,
                    control::PlacementConfirmation::default()
                );
                assert_eq!(reply.detail.progress.host_mutation_dispatched, dispatched);
                assert!(!reply.detail.progress.input_ready);
                assert_eq!(host.log, ["send"]);
            }
        }
    }
}

#[test]
fn recover_shutdown_preserves_every_history_partition_and_clears_current_proof() {
    for owned in [false, true] {
        for relocated in [false, true] {
            for restored in [false, true] {
                for phase in [control::RecoverPhase::Here, control::RecoverPhase::Top] {
                    let mut host = Fake::new();
                    let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
                    endpoint.transport = atoms();
                    endpoint.descriptor = owner();
                    endpoint.owns_selection = owned;
                    control::admit(
                        &mut endpoint.active,
                        request(Command::Recover),
                        7,
                        Lifecycle::Ready,
                        false,
                        host.now,
                    );
                    let op = endpoint.active.as_mut().unwrap();
                    op.recover_phase = phase;
                    op.placement.baseline = Some(control::PlacementBaseline {
                        desktop: if relocated { 0 } else { 7 },
                        minimized: restored,
                        layer: control::LayerObservation::Below,
                    });
                    op.placement.proof.history = control::RecoveryHistory {
                        relocation_confirmed: relocated,
                        restoration_confirmed: restored,
                    };
                    op.detail.progress = control::Progress {
                        host_mutation_dispatched: true,
                        not_minimized_confirmed: true,
                        above_confirmed: true,
                        input_ready: true,
                        placement_deferred: false,
                    };
                    op.placement.proof.current.destination = true;
                    endpoint.finish_remote_with(&mut host).unwrap();
                    assert!(endpoint.active().is_none());
                    let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
                    assert_eq!(
                        reply.status,
                        if relocated || restored {
                            Status::Partial
                        } else {
                            Status::Failed
                        }
                    );
                    assert_eq!(
                        reply.detail.reason,
                        if owned {
                            Reason::Closing
                        } else {
                            Reason::OwnershipLost
                        }
                    );
                    assert_eq!(reply.detail.stage, Stage::Shutdown);
                    assert_eq!(reply.request_id, ID);
                    assert_eq!(
                        reply.detail.placement.history,
                        control::RecoveryHistory {
                            relocation_confirmed: relocated,
                            restoration_confirmed: restored,
                        }
                    );
                    assert_eq!(
                        reply.detail.placement.current,
                        control::PlacementConfirmation::default()
                    );
                    assert_eq!(
                        reply.detail.progress,
                        control::Progress {
                            host_mutation_dispatched: true,
                            ..Default::default()
                        }
                    );
                    endpoint.finish_remote_with(&mut host).unwrap();
                    assert_eq!(host.sent.len(), 1);
                }
            }
        }
    }
}

fn recover_lifecycle_endpoint(operation: ActiveOperation) -> ControlWindow {
    let mut endpoint = ControlWindow::candidate(0, CONTROL, 77, 50, 51, 52);
    endpoint.root = ROOT;
    endpoint.transport = atoms();
    endpoint.descriptor = owner();
    endpoint.owns_selection = true;
    // Transfer a snapshot of the production-driven slot, without reconstructing
    // any phase, proof, consumed attempt or deadline in this transport fixture.
    endpoint.active = Some(operation);
    endpoint
}

fn recover_selection_clear(synthetic: bool) -> Event {
    Event::SelectionClear(SelectionClearEvent {
        response_type: 29 | if synthetic { 0x80 } else { 0 },
        owner: CONTROL,
        selection: 50,
        ..Default::default()
    })
}

fn assert_recover_lifecycle_terminal(
    endpoint: &mut ControlWindow,
    phase: usize,
    operation: ActiveOperation,
    reason: Reason,
    peer_failure: usize,
) {
    let owned = endpoint.owns_selection;
    assert_eq!(owned, reason == Reason::Closing);
    let descriptor = endpoint.descriptor;
    let mut queries = 0;
    let mut writes = 0;
    endpoint
        .closing_with(
            || {
                assert!(owned, "lost owner must not query or reacquire");
                queries += 1;
                Ok(CONTROL)
            },
            |published| {
                assert!(owned, "must not publish under a replacement owner");
                writes += 1;
                assert_eq!(
                    published,
                    OwnerDescriptor {
                        lifecycle: Lifecycle::Closing,
                        ..descriptor
                    }
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!((queries, writes), (usize::from(owned), usize::from(owned)));
    // Lifecycle handling/Closing cannot replenish attempts, change identity,
    // advance the operation or renew any of its phase/total deadlines.
    assert_eq!(endpoint.active(), Some(&operation), "snapshot {phase}");
    assert_eq!(endpoint.owns_selection, owned);

    let mut peer = Fake::new();
    peer.owner = CONTROL + 1;
    peer.send_ok = peer_failure != 1;
    peer.failure = (peer_failure == 2).then_some("send");
    assert_eq!(
        endpoint.finish_remote_with(&mut peer).is_err(),
        peer_failure == 2,
        "snapshot {phase}"
    );
    assert!(endpoint.active().is_none(), "clear even on a failed send");
    assert_eq!(peer.sent.len(), 1);
    let event = &peer.sent[0];
    assert_eq!(event.window, operation.request.reply_xid);
    assert_eq!(event.type_, atoms().reply);
    let reply = TerminalResponse::decode(&event.data.as_data32()).unwrap();
    assert_eq!(reply.control_xid, CONTROL);
    assert_eq!(reply.request_id, operation.request.request_id);
    assert!(ReplyCorrelation {
        control_xid: CONTROL,
        epoch: operation.request.epoch,
        request_id: operation.request.request_id,
        reply_xid: operation.request.reply_xid,
    }
    .matches(descriptor, event.window, reply));
    let history = operation.placement.proof.history;
    assert_eq!(history.relocation_confirmed, phase >= 2);
    assert_eq!(history.restoration_confirmed, phase >= 4);
    assert_eq!(reply.detail.placement.history, history);
    assert_eq!(
        reply.status,
        if history.relocation_confirmed || history.restoration_confirmed {
            Status::Partial
        } else {
            Status::Failed
        }
    );
    assert_eq!(
        (reply.detail.reason, reply.detail.stage),
        (reason, Stage::Shutdown)
    );
    assert_eq!(
        reply.detail.placement.current,
        control::PlacementConfirmation::default()
    );
    assert_eq!(
        reply.detail.progress,
        control::Progress {
            host_mutation_dispatched: operation.detail.progress.host_mutation_dispatched,
            ..Default::default()
        }
    );
    assert_eq!(
        (
            reply.detail.visibility,
            reply.detail.layer,
            reply.detail.workspace
        ),
        (
            control::Visibility::Unknown,
            control::LayerObservation::Unknown,
            control::Workspace::Unknown
        )
    );
    if !owned {
        assert!(!endpoint
            .handle_event_with(&recover_selection_clear(false), || panic!(
                "loss stays terminal"
            ))
            .unwrap());
        assert!(endpoint
            .handle_event_with(&destroyed(false), || panic!(
                "native destruction needs no query"
            ))
            .unwrap());
        endpoint
            .closing_with(
                || panic!("no reacquisition"),
                |_| panic!("no publication after loss"),
            )
            .unwrap();
    }
    endpoint.finish_remote_with(&mut peer).unwrap();
    assert!(!endpoint
        .service_here_with_peer(&mut peer, |_, _, _, _| panic!(
            "no operation after loss/shutdown"
        ))
        .unwrap());
    assert_eq!(peer.sent.len(), 1, "no second terminal attempt");
    assert_eq!(
        peer.log,
        ["send"],
        "no discovery, read or mutation after loss/shutdown"
    );
}

#[test]
fn recover_selection_clear_confirms_loss_at_every_service_snapshot() {
    for (phase, operation) in crate::here_service_tests::recover_tests::lifecycle_snapshots() {
        for replacement in [0, CONTROL + 1] {
            for synthetic in [false, true] {
                for peer_failure in 0..3 {
                    let mut endpoint = recover_lifecycle_endpoint(operation);
                    let event = recover_selection_clear(synthetic);
                    let mut queries = 0;
                    assert!(endpoint
                        .handle_event_with(&event, || {
                            queries += 1;
                            Ok(replacement)
                        })
                        .unwrap());
                    assert_eq!(queries, 1, "SelectionClear requires fresh confirmation");
                    assert!(!endpoint.owns_selection);
                    assert!(
                        endpoint.exists,
                        "selection loss is not endpoint destruction"
                    );
                    for repeated in [
                        recover_selection_clear(false),
                        recover_selection_clear(true),
                    ] {
                        assert!(!endpoint
                            .handle_event_with(&repeated, || panic!(
                                "no query or reacquisition after loss"
                            ))
                            .unwrap());
                    }
                    assert_recover_lifecycle_terminal(
                        &mut endpoint,
                        phase,
                        operation,
                        Reason::OwnershipLost,
                        peer_failure,
                    );
                }
            }
        }
    }
}

#[test]
fn recover_selection_clear_with_unchanged_owner_preserves_every_service_snapshot() {
    for (phase, operation) in crate::here_service_tests::recover_tests::lifecycle_snapshots() {
        for synthetic in [false, true] {
            let mut endpoint = recover_lifecycle_endpoint(operation);
            let descriptor = endpoint.descriptor;
            let mut queries = 0;
            assert!(!endpoint
                .handle_event_with(&recover_selection_clear(synthetic), || {
                    queries += 1;
                    Ok(CONTROL)
                })
                .unwrap());
            assert_eq!(queries, 1);
            assert!(endpoint.owns_selection);
            assert!(endpoint.exists);
            assert_eq!(endpoint.descriptor, descriptor);
            assert_eq!(endpoint.active(), Some(&operation));
            // A later clean shutdown must still be Closing, never OwnershipLost.
            assert_recover_lifecycle_terminal(&mut endpoint, phase, operation, Reason::Closing, 0);
        }
    }
}

#[test]
fn recover_native_endpoint_destroy_is_terminal_at_every_service_snapshot() {
    for (phase, operation) in crate::here_service_tests::recover_tests::lifecycle_snapshots() {
        for peer_failure in 0..3 {
            let mut endpoint = recover_lifecycle_endpoint(operation);
            let event = destroyed(false);
            assert!(endpoint
                .handle_event_with(&event, || panic!(
                    "native destruction needs no ownership query"
                ))
                .unwrap());
            assert!(!endpoint.exists);
            assert!(!endpoint.owns_selection);
            endpoint
                .resource
                .release_with(|| panic!("externally destroyed endpoint"))
                .unwrap();
            assert!(endpoint
                .handle_event_with(&event, || panic!("repeated destroy needs no query"))
                .unwrap());
            assert!(!endpoint
                .handle_event_with(&recover_selection_clear(false), || panic!(
                    "no reacquisition"
                ))
                .unwrap());
            assert!(!endpoint
                .handle_event_with(&destroyed(true), || panic!("synthetic destroy"))
                .unwrap());
            assert_recover_lifecycle_terminal(
                &mut endpoint,
                phase,
                operation,
                Reason::OwnershipLost,
                peer_failure,
            );
        }
    }
}

#[test]
fn recover_synthetic_destroy_and_unrelated_lifecycle_events_preserve_every_snapshot() {
    for (phase, operation) in crate::here_service_tests::recover_tests::lifecycle_snapshots() {
        let mut endpoint = recover_lifecycle_endpoint(operation);
        for synthetic in [false, true] {
            for (clear_owner, selection) in [(CONTROL + 1, 50), (CONTROL, 51)] {
                let event = Event::SelectionClear(SelectionClearEvent {
                    response_type: 29 | if synthetic { 0x80 } else { 0 },
                    owner: clear_owner,
                    selection,
                    ..Default::default()
                });
                assert!(!endpoint
                    .handle_event_with(&event, || panic!("unrelated clear"))
                    .unwrap());
                assert_eq!(endpoint.active(), Some(&operation));
            }
            let event = Event::DestroyNotify(DestroyNotifyEvent {
                response_type: 17 | if synthetic { 0x80 } else { 0 },
                window: CONTROL + 1,
                ..Default::default()
            });
            assert!(!endpoint
                .handle_event_with(&event, || panic!("unrelated destroy"))
                .unwrap());
        }
        assert!(!endpoint
            .handle_event_with(&destroyed(true), || panic!("fabricated destroy"))
            .unwrap());
        assert!(endpoint.exists);
        assert!(endpoint.owns_selection);
        assert_eq!(endpoint.descriptor, owner());
        assert_eq!(endpoint.active(), Some(&operation));
        assert_recover_lifecycle_terminal(&mut endpoint, phase, operation, Reason::Closing, 0);
    }
}

#[test]
fn recover_closing_lookup_replacement_is_proven_loss_without_publication() {
    for (phase, operation) in crate::here_service_tests::recover_tests::lifecycle_snapshots() {
        let mut endpoint = recover_lifecycle_endpoint(operation);
        let descriptor = endpoint.descriptor;
        let mut queries = 0;
        endpoint
            .closing_with(
                || {
                    queries += 1;
                    Ok(CONTROL + 1)
                },
                |_| panic!("must not publish to the replacement owner"),
            )
            .unwrap();
        assert_eq!(queries, 1);
        assert!(!endpoint.owns_selection);
        assert_eq!(endpoint.descriptor, descriptor);
        assert_eq!(endpoint.active(), Some(&operation));
        // Reuse F-T1a's history, correlation, no-reacquisition and one-terminal
        // assertions. A successful mismatch takes ordinary OwnershipLost.
        assert_recover_lifecycle_terminal(
            &mut endpoint,
            phase,
            operation,
            Reason::OwnershipLost,
            0,
        );
    }
}

#[test]
fn bring_top_shutdown_retains_history_and_clears_stale_current_confirmations() {
    for restored in [false, true] {
        let mut h = Fake::new();
        let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
        endpoint.transport = atoms();
        endpoint.descriptor = owner();
        control::admit(
            &mut endpoint.active,
            request(Command::BringTop),
            crate::control::NO_DESTINATION,
            Lifecycle::Ready,
            false,
            h.now,
        );
        let op = endpoint.active_mut().unwrap();
        op.bring_top.minimized_at_start = restored;
        op.bring_top.restoration_confirmed = restored;
        op.detail.progress = control::Progress {
            not_minimized_confirmed: true,
            above_confirmed: true,
            input_ready: true,
            host_mutation_dispatched: true,
            ..control::Progress::default()
        };
        endpoint.finish_remote_with(&mut h).unwrap();
        endpoint.finish_remote_with(&mut h).unwrap();
        assert_eq!(h.sent.len(), 1);
        let response = TerminalResponse::decode(&h.sent[0].data.as_data32()).unwrap();
        assert_eq!(
            response.status,
            if restored {
                Status::Partial
            } else {
                Status::Failed
            }
        );
        assert!(!response.detail.progress.not_minimized_confirmed);
        assert!(!response.detail.progress.above_confirmed);
        assert!(!response.detail.progress.input_ready);
        assert!(response.detail.progress.host_mutation_dispatched);
    }
}
#[test]
fn bring_top_diagnostics_qualify_location_history_and_current_layer() {
    let mut response = response();
    response.status = Status::Success;
    response.detail = control::preflight_detail(Reason::None, Stage::Layer);
    response.detail.progress.not_minimized_confirmed = true;
    response.detail.progress.above_confirmed = true;
    for elsewhere in [false, true] {
        response.detail.progress.placement_deferred = elsewhere;
        for dispatched in [false, true] {
            response.detail.progress.host_mutation_dispatched = dispatched;
            let text = CallerOutcome::Terminal(response).diagnostic(Command::BringTop);
            assert_eq!(text.contains("another workspace"), elsewhere);
            assert_eq!(
                text.contains("no restoration/layer request needed"),
                !dispatched
            );
            assert!(!text.contains("visible here"));
        }
    }
    response.status = Status::Partial;
    response.detail.reason = Reason::ConfirmationTimeout;
    response.detail.layer = control::LayerObservation::Normal;
    response.detail.progress.above_confirmed = false;
    assert!(CallerOutcome::Terminal(response)
        .diagnostic(Command::BringTop)
        .contains("restoration confirmed on another workspace"));
    response.detail.progress.not_minimized_confirmed = false;
    assert!(CallerOutcome::Terminal(response)
        .diagnostic(Command::BringTop)
        .contains("confirmed earlier"));
    response.status = Status::Failed;
    let text = CallerOutcome::Terminal(response).diagnostic(Command::BringTop);
    assert!(text.contains("current effective layer: Normal"));
    assert!(!text.contains("restoration confirmed"));
}
#[test]
fn discovery_lifecycle_and_absence_never_create_or_send() {
    for lifecycle in [Lifecycle::Starting, Lifecycle::Closing] {
        let mut host = Fake::new();
        host.evidence.push_back(Some(evidence(
            &OwnerDescriptor {
                lifecycle,
                ..owner()
            }
            .encode()
            .unwrap(),
        )));
        assert_eq!(
            host.call_command().unwrap(),
            CallerOutcome::Lifecycle(lifecycle)
        );
        assert!(host.sent.is_empty());
        assert!(host.marker.is_none());
        assert_eq!(host.log.last(), Some(&"cleanup"));
    }
    let mut host = Fake::new();
    host.owner = 0;
    assert_eq!(host.call_command().unwrap(), CallerOutcome::NoInstance);
    assert!(host.marker.is_none());
    assert!(host.sent.is_empty());
    assert_eq!(CallerOutcome::NoInstance.exit_code(), ExitCode::NoInstance);
}
#[test]
fn discovery_rejects_every_malformed_descriptor_and_native_endpoint() {
    let valid = owner().encode().unwrap();
    let mut variants = vec![];
    for (index, word) in [(0, 0), (1, 1), (2, 1), (3, 11), (4, 0), (5, 99), (6, 0)] {
        let mut words = valid;
        words[index] = word;
        variants.push(Some(evidence(&words)));
    }
    variants.push(None);
    for case in 0..10 {
        let mut e = evidence(&valid);
        match case {
            0 => e.root += 1,
            1 => e.parent += 1,
            2 => e.class = WindowClass::INPUT_OUTPUT,
            3 => e.map_state = MapState::VIEWABLE,
            4 => e.property.type_ = AtomEnum::STRING.into(),
            5 => e.property.format = 8,
            6 => e.property.value_len = 6,
            7 => e.property.bytes_after = 4,
            8 => e.property.value.push(0),
            9 => e.property.value.clear(),
            _ => unreachable!(),
        }
        variants.push(Some(e));
    }
    for e in variants {
        let mut host = Fake::new();
        host.evidence.push_back(e);
        assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
        assert!(host.sent.is_empty());
        assert!(host.marker.is_none());
    }
    for replacement in [0, 11] {
        let mut host = Fake::new();
        host.owners = VecDeque::from([CONTROL, replacement]);
        assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
        assert!(host.sent.is_empty());
    }
}
#[test]
fn ready_caller_sends_exactly_once_and_accepts_real_correlated_terminal() {
    let mut host = Fake::new();
    host.events.push_back(terminal());
    assert_eq!(
        host.call_command().unwrap(),
        CallerOutcome::Terminal(response())
    );
    assert_eq!(host.marker, Some(marker(Command::Show)));
    assert_eq!(host.sent.len(), 1);
    let event = host.sent[0];
    assert_eq!(
        (event.window, event.type_, event.format),
        (CONTROL, atoms().request, 32)
    );
    assert_eq!(
        event.data.as_data32(),
        [
            (2 << 16) | 2,
            ID as u32,
            (ID >> 32) as u32,
            REPLY_WINDOW,
            77
        ]
    );
    assert_eq!(host.log.iter().filter(|&&x| x == "create reply").count(), 1);
    assert_eq!(host.log.last(), Some(&"cleanup"));
    assert_eq!(
        CallerOutcome::Terminal(response()).exit_code(),
        ExitCode::OperationFailed
    );
}
#[test]
fn caller_window_is_exact_unmapped_input_only_without_input_subscriptions() {
    let screen = Screen {
        root: ROOT,
        root_visual: 66,
        ..Screen::default()
    };
    let r = caller_request(REPLY_WINDOW, &screen);
    assert_eq!((r.wid, r.parent, r.visual), (REPLY_WINDOW, ROOT, 66));
    assert_eq!((r.depth, r.border_width, r.width, r.height), (0, 0, 1, 1));
    assert_eq!(r.class, WindowClass::INPUT_ONLY);
    assert_eq!(r.value_list.event_mask, None);
    assert_eq!(
        marker(Command::Show).encode().unwrap(),
        [
            control::MAGIC,
            2,
            CONTROL,
            77,
            ID as u32,
            (ID >> 32) as u32,
            2,
            control::NO_DESTINATION,
        ]
    );
    // CreateWindow has no mapped flag; neither CallerHost nor PeerHost offers MapWindow,
    // selection acquisition, alpha visual, body, renderer, focus, or keyboard work.
}
#[test]
fn random_ids_regenerate_zero_and_use_all_64_bits_with_io_errors_preserved() {
    let mut bytes = Vec::from(0u64.to_ne_bytes());
    bytes.extend(ID.to_ne_bytes());
    bytes.extend((ID + 1).to_ne_bytes());
    let mut bytes = bytes.as_slice();
    assert_eq!(fresh_request_id(&mut bytes).unwrap(), ID);
    assert_eq!(fresh_request_id(&mut bytes).unwrap(), ID + 1);
    assert!(fresh_request_id(&mut bytes).is_err());
}
#[test]
fn caller_cleanup_runs_on_setup_send_and_wait_failure_and_retains_original_error() {
    for failure in [
        "atoms",
        "owner",
        "evidence",
        "watch",
        "create reply",
        "send",
        "next",
        "wait",
        "cleanup",
    ] {
        let mut host = Fake::new();
        host.failure = Some(failure);
        let error = host.call_command().unwrap_err();
        assert_eq!(error.to_string(), failure);
        assert_eq!(host.log.last(), Some(&"cleanup"));
        assert!(host.sent.len() <= 1);
    }
}
#[test]
fn request_envelope_garbage_and_stale_identity_cannot_admit_or_reply() {
    let good = request_event(atoms(), CONTROL, request(Command::Show)).unwrap();
    for case in 0..6 {
        let mut event = good;
        match case {
            0 => event.window += 1,
            1 => event.type_ += 1,
            2 => event.format = 8,
            3 => {
                let mut w = event.data.as_data32();
                w[4] += 1;
                event.data = w.into();
            }
            4 => {
                let mut w = event.data.as_data32();
                w[1] = 0;
                w[2] = 0;
                event.data = w.into();
            }
            5 => {
                let mut w = event.data.as_data32();
                w[3] = 0;
                event.data = w.into();
            }
            _ => unreachable!(),
        }
        let mut host = Fake::new();
        let mut active = None;
        host.route(&mut active, Lifecycle::Ready, false, &event)
            .unwrap();
        assert!(active.is_none());
        assert!(host.sent.is_empty());
    }
}
#[test]
fn reply_route_requires_exact_native_evidence_and_immutable_identity() {
    let req = request(Command::Show);
    let words = req.encode().unwrap();
    let valid = marker(Command::Show).encode().unwrap();
    let mut variants = vec![None];
    for (index, word) in [
        (0, 0),
        (1, 1),
        (2, CONTROL + 1),
        (3, 78),
        (4, 0),
        (5, 0),
        (6, 1),
    ] {
        let mut w = valid;
        w[index] = word;
        variants.push(Some(evidence(&w)));
    }
    for case in 0..11 {
        let mut e = evidence(&valid);
        match case {
            0 => e.root += 1,
            1 => e.parent += 1,
            2 => e.class = WindowClass::INPUT_OUTPUT,
            3 => e.map_state = MapState::UNVIEWABLE,
            4 => e.property.type_ = 0,
            5 => e.property.type_ = AtomEnum::STRING.into(),
            6 => e.property.format = 16,
            7 => e.property.value_len = 7,
            8 => e.property.bytes_after = 4,
            9 => e.property.value.truncate(24),
            10 => e.property.value.extend([0; 4]),
            _ => unreachable!(),
        }
        variants.push(Some(e));
    }
    for e in variants {
        let mut host = Fake::new();
        host.evidence.push_back(e);
        let mut active = None;
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, req).unwrap(),
        )
        .unwrap();
        assert!(active.is_none());
        assert!(host.sent.is_empty());
    }
    for forbidden in [0, ROOT, CONTROL, 88, 99] {
        assert!(
            validated_route(&evidence(&valid), ROOT, owner(), Some(99), forbidden, words).is_none()
        );
    }
    assert_eq!(
        validated_route(
            &evidence(&valid),
            ROOT,
            owner(),
            Some(99),
            REPLY_WINDOW,
            words
        ),
        Some((req, control::NO_DESTINATION, None))
    );
}
#[test]
fn incompatible_headers_get_protocol_error_only_via_valid_v2_route() {
    for (header, reason) in [
        ((1 << 16) | 2, Reason::WrongVersion),
        ((2 << 16) | 99, Reason::UnknownCommand),
    ] {
        for safe in [true, false] {
            let mut host = Fake::new();
            if !safe {
                host.evidence.push_back(None);
            }
            let mut event = request_event(atoms(), CONTROL, request(Command::Show)).unwrap();
            let mut words = event.data.as_data32();
            words[0] = header;
            event.data = words.into();
            let mut active = None;
            host.route(&mut active, Lifecycle::Ready, false, &event)
                .unwrap();
            assert!(active.is_none());
            assert_eq!(host.sent.len(), usize::from(safe));
            if safe {
                let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
                assert_eq!(
                    (reply.status, reply.detail.reason),
                    (Status::ProtocolError, reason)
                );
            }
        }
    }
}
#[test]
fn bring_top_is_asynchronously_admitted_without_an_unsupported_reply() {
    let command = Command::BringTop;
    let mut host = Fake::new();
    host.evidence
        .push_back(Some(evidence(&marker(command).encode().unwrap())));
    let event = request_event(atoms(), CONTROL, request(command)).unwrap();
    let mut active = None;
    host.route(&mut active, Lifecycle::Ready, false, &event)
        .unwrap();
    let op = active.unwrap();
    assert_eq!(op.request.command, command);
    assert_eq!(op.stage, Stage::Show);
    assert_eq!(op.deadline, host.now + Duration::from_millis(3500));
    assert_eq!(
        op.bring_top.visibility_cutoff,
        host.now + Duration::from_millis(2500)
    );
    assert!(host.sent.is_empty());
    finish_active(&mut host, atoms(), &mut active, response()).unwrap();
    assert!(active.is_none());
    assert_eq!(host.sent.len(), 1);
    assert!(finish_active(&mut host, atoms(), &mut active, response()).is_err());
    assert_eq!(host.sent.len(), 1);
}

#[test]
fn hide_is_deferred_with_one_terminal_duplicate_suppression_and_competitor_busy() {
    let mut host = Fake::new();
    let mut active = None;
    let req = request(Command::Hide);
    for _ in 0..2 {
        host.evidence
            .push_back(Some(evidence(&marker(Command::Hide).encode().unwrap())));
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, req).unwrap(),
        )
        .unwrap();
    }
    assert!(host.sent.is_empty());
    let original = active.unwrap();
    assert_eq!(original.request, req);
    assert_eq!(original.stage, Stage::Hide);
    assert_eq!(original.deadline, host.now + Duration::from_millis(1000));
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
    )
    .unwrap();
    assert_eq!(active, Some(original));
    assert_eq!(
        TerminalResponse::decode(&host.sent[0].data.as_data32())
            .unwrap()
            .status,
        Status::Busy
    );
    finish_active(&mut host, atoms(), &mut active, response()).unwrap();
    assert!(active.is_none());
    assert_eq!(host.sent.len(), 2);
    assert!(finish_active(&mut host, atoms(), &mut active, response()).is_err());
    assert_eq!(host.sent.len(), 2);
}
#[test]
fn hide_during_starting_closing_or_local_layer_operation_has_no_admission() {
    for (lifecycle, busy, status) in [
        (Lifecycle::Starting, false, Status::Starting),
        (Lifecycle::Closing, false, Status::Closing),
        (Lifecycle::Ready, true, Status::Busy),
    ] {
        let mut host = Fake::new();
        host.evidence
            .push_back(Some(evidence(&marker(Command::Hide).encode().unwrap())));
        let mut active = None;
        host.route(
            &mut active,
            lifecycle,
            busy,
            &request_event(atoms(), CONTROL, request(Command::Hide)).unwrap(),
        )
        .unwrap();
        assert!(active.is_none());
        assert_eq!(host.sent.len(), 1);
        let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
        assert_eq!(reply.status, status);
        assert_eq!(reply.status.exit_code(), ExitCode::Busy);
        assert!(!reply.detail.progress.host_mutation_dispatched);
    }
}
#[test]
fn hide_shutdown_at_each_phase_preserves_progress_and_ready_owner_until_closing() {
    let fixtures = crate::show_tests::h_c2::hide_phases();
    assert_eq!(fixtures.len(), 6);
    for (operation, fixture) in fixtures {
        assert_shutdown_snapshot(operation, fixture, false);
    }
}

fn assert_shutdown_snapshot(
    operation: ActiveOperation,
    mut fixture: crate::show_tests::h_c2::Fixture,
    reply_failure: bool,
) {
    let mut host = Fake::new();
    if reply_failure {
        host.failure = Some("send");
    }
    let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
    endpoint.transport = atoms();
    endpoint.descriptor = OwnerDescriptor {
        body_xid: Some(fixture.body()),
        ..owner()
    };
    endpoint.owns_selection = true;
    endpoint.active = Some(operation);
    let original = fixture.take_original_error();
    let original_message = original.as_ref().map(ToString::to_string);
    let state = fixture.state();
    let trace = fixture.trace().to_vec();
    let ready = fixture.input_ready();
    assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Ready);
    let body = endpoint.descriptor.body_xid;
    endpoint.closing_with(|| Ok(CONTROL), |_| Ok(())).unwrap();
    assert_eq!(endpoint.descriptor.lifecycle, Lifecycle::Closing);
    assert_eq!(endpoint.descriptor.body_xid, body);
    let finished = endpoint.finish_remote_with(&mut host);
    assert_eq!(finished.is_err(), reply_failure);
    assert!(endpoint.active().is_none());
    endpoint.finish_remote_with(&mut host).unwrap();
    assert_eq!(host.sent.len(), 1);
    let response = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
    assert_eq!(response.status, Status::Closing);
    assert_eq!(
        (response.detail.reason, response.detail.stage),
        (Reason::Closing, Stage::Shutdown)
    );
    assert_eq!(response.detail.progress, operation.detail.progress);
    assert_eq!(response.detail.visibility, operation.detail.visibility);
    assert_eq!(response.detail.workspace, operation.detail.workspace);
    assert_eq!(response.request_id, operation.request.request_id);
    assert_eq!(response.control_xid, CONTROL);
    assert_eq!(
        (host.sent[0].type_, host.sent[0].window),
        (atoms().reply, operation.request.reply_xid)
    );
    assert_eq!(fixture.state(), state); // no rollback, including InputRestored

    fixture.notify_and_service(endpoint.active_mut());
    fixture.late_notifications(endpoint.active_mut());
    endpoint.finish_remote_with(&mut host).unwrap();
    assert_eq!(fixture.trace(), trace); // no map/HIDDEN/configure/layer/enable work
    assert_eq!(fixture.state(), state);
    assert_eq!(fixture.input_ready(), ready);
    assert_eq!(host.sent.len(), 1);

    // Use the ordered production cleanup coordinator after remote clearing.
    // A failed reply cannot prevent body/control/flush cleanup or supersede
    // the original geometry failure in the new placement-pending snapshot.
    let order = std::cell::RefCell::new(vec![]);
    let cleanup = crate::owner_cleanup_with(
        &mut endpoint,
        |endpoint| {
            assert!(endpoint.active().is_none());
            order.borrow_mut().push("remote-cleared");
            finished
        },
        || {
            order.borrow_mut().push("body");
            fixture.cleanup()
        },
        |endpoint| {
            endpoint.resource.release_with(|| {
                order.borrow_mut().push("control");
                Ok(())
            })
        },
        || {
            order.borrow_mut().push("flush");
            Ok(())
        },
    );
    assert_eq!(
        *order.borrow(),
        ["remote-cleared", "body", "control", "flush"]
    );
    if let Some(original) = original {
        assert_eq!(
            crate::finish_probe_with(Err(original), || cleanup)
                .unwrap_err()
                .to_string(),
            original_message.unwrap()
        );
    } else {
        cleanup.unwrap();
    }
    assert_eq!(fixture.trace(), trace);
    assert!(endpoint.active().is_none());
    assert_eq!(host.sent.len(), 1);
}

fn assert_post_dispatch_hide_loss(destroyed: bool) {
    for elsewhere in [false, true] {
        let (operation, result, mut fixture) =
            crate::show_tests::h_c2::hide_loss_after_dispatch(destroyed, elsewhere);
        let mut host = Fake::new();
        let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
        endpoint.transport = atoms();
        endpoint.descriptor = owner();
        endpoint.active = Some(operation);
        let state = fixture.state();
        let trace = fixture.trace().to_vec();
        let terminal = TerminalResponse {
            status: result.status(),
            detail: result.detail(),
            request_id: operation.request.request_id,
            control_xid: CONTROL,
        };
        finish_active(&mut host, atoms(), &mut endpoint.active, terminal).unwrap();
        assert!(endpoint.active().is_none());
        assert_eq!(
            TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap(),
            terminal
        );
        fixture.notify_and_service(endpoint.active_mut());
        fixture.late_notifications(endpoint.active_mut());
        endpoint.finish_remote_with(&mut host).unwrap();
        assert_eq!(fixture.trace(), trace);
        assert_eq!(fixture.state(), state);
        assert_eq!(host.sent.len(), 1);
        assert!(!fixture.input_ready());
        fixture.cleanup().unwrap();
        assert_eq!(fixture.trace(), trace);
    }
}

#[test]
fn hide_body_destroyed_after_dispatch_fails_once_without_retry_or_input_restoration() {
    assert_post_dispatch_hide_loss(true);
}

#[test]
fn hide_body_withdrawn_after_dispatch_never_confuses_unmapped_with_minimized() {
    assert_post_dispatch_hide_loss(false);
}
#[test]
fn hide_owner_loss_during_validation_is_fatal_without_admission_or_reply() {
    let mut host = Fake::new();
    host.owners = VecDeque::from([CONTROL, 999]);
    host.evidence
        .push_back(Some(evidence(&marker(Command::Hide).encode().unwrap())));
    let mut active = None;
    assert!(host
        .route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(Command::Hide)).unwrap()
        )
        .is_err());
    assert!(active.is_none());
    assert!(host.sent.is_empty());
}
#[test]
fn hide_noop_terminal_leaves_same_ready_endpoint_available_for_show() {
    let mut host = Fake::new();
    let mut endpoint = ControlWindow::candidate(0, CONTROL, 11, 50, 51, 52);
    endpoint.transport = atoms();
    endpoint.descriptor = owner();
    let descriptor = endpoint.descriptor;
    host.evidence
        .push_back(Some(evidence(&marker(Command::Hide).encode().unwrap())));
    host.route(
        &mut endpoint.active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::Hide)).unwrap(),
    )
    .unwrap();
    let mut terminal = response();
    terminal.status = Status::Success;
    terminal.detail = control::preflight_detail(Reason::None, Stage::Hide);
    terminal.detail.visibility = control::Visibility::Minimized;
    terminal.detail.workspace = control::Workspace::Current;
    assert_eq!(
        CallerOutcome::Terminal(terminal).diagnostic(Command::Hide),
        "hide: confirmed minimized; existing instance remains running"
    );
    finish_active(&mut host, atoms(), &mut endpoint.active, terminal).unwrap();
    assert_eq!(endpoint.descriptor, descriptor);
    host.route(
        &mut endpoint.active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
    )
    .unwrap();
    assert_eq!(endpoint.active().unwrap().request.command, Command::Show);
    assert_eq!(host.sent.len(), 1);
    assert!(
        !TerminalResponse::decode(&host.sent[0].data.as_data32())
            .unwrap()
            .detail
            .progress
            .host_mutation_dispatched
    );
}

#[test]
fn valid_show_is_pending_without_ack_and_gets_one_correlated_terminal() {
    let mut host = Fake::new();
    let mut active = None;
    let req = request(Command::Show);
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, req).unwrap(),
    )
    .unwrap();
    assert!(host.sent.is_empty());
    let operation = active.unwrap();
    assert_eq!(operation.request, req);
    assert_eq!(operation.stage, Stage::Show);
    assert_eq!(operation.deadline, host.now + Duration::from_millis(2500));
    assert_eq!(operation.detail.reason, Reason::None);
    finish_active(&mut host, atoms(), &mut active, response()).unwrap();
    assert!(active.is_none());
    let reply = host.sent[0];
    let terminal = TerminalResponse::decode(&reply.data.as_data32()).unwrap();
    assert_eq!(reply.window, req.reply_xid);
    assert_eq!(terminal.request_id, req.request_id);
    assert_eq!(terminal.control_xid, CONTROL);
    assert!(finish_active(&mut host, atoms(), &mut active, response()).is_err());
    assert_eq!(host.sent.len(), 1);
}

#[test]
fn shutdown_at_each_show_phase_replies_once_with_progress_then_clears_record() {
    let fixtures = crate::show_tests::h_c2::show_phases();
    assert_eq!(fixtures.len(), 8);
    for (operation, fixture) in fixtures {
        assert_shutdown_snapshot(operation, fixture, false);
    }
}

#[test]
fn shutdown_during_show_placement_and_correction_retains_work_without_restarting() {
    let fixtures = crate::show_tests::h_c2::geometry_snapshots();
    assert_eq!(fixtures.len(), 3);
    for (name, operation, fixture) in fixtures {
        assert_shutdown_snapshot(operation, fixture, name == "placement-pending");
    }
}

#[test]
fn caller_gone_before_show_admission_creates_no_operation_or_reply() {
    let mut host = Fake::new();
    host.evidence.push_back(None);
    let mut active = None;
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
    )
    .unwrap();
    assert!(active.is_none());
    assert!(host.sent.is_empty());
}

#[test]
fn show_diagnostics_distinguish_location_dispatch_timeout_and_drift() {
    for (elsewhere, dispatched, expected) in [
        (false, true, "restored here"),
        (false, false, "already not minimized here"),
        (true, true, "restored on another workspace"),
        (true, false, "already not minimized on another workspace"),
    ] {
        let mut terminal = response();
        terminal.status = Status::Success;
        terminal.detail.progress.host_mutation_dispatched = dispatched;
        terminal.detail.progress.placement_deferred = elsewhere;
        let diagnostic = CallerOutcome::Terminal(terminal).diagnostic(Command::Show);
        assert!(diagnostic.contains(expected));
        if elsewhere {
            assert!(diagnostic.contains("not necessarily visible here"));
        }
    }
    for (reason, expected) in [
        (Reason::ConfirmationTimeout, "timed out"),
        (Reason::LayoutChanged, "workspace/layout"),
        (Reason::LayerChanged, "layer changed"),
        (Reason::Unverifiable, "Unverifiable"),
    ] {
        let mut terminal = response();
        terminal.detail.reason = reason;
        assert!(CallerOutcome::Terminal(terminal)
            .diagnostic(Command::Show)
            .contains(expected));
    }
}
#[test]
fn starting_closing_and_local_layer_busy_reply_without_admission() {
    for (lifecycle, busy, status, reason, stage) in [
        (
            Lifecycle::Starting,
            false,
            Status::Starting,
            Reason::Starting,
            Stage::Preflight,
        ),
        (
            Lifecycle::Closing,
            false,
            Status::Closing,
            Reason::Closing,
            Stage::Shutdown,
        ),
        (
            Lifecycle::Ready,
            true,
            Status::Busy,
            Reason::Busy,
            Stage::Preflight,
        ),
    ] {
        let mut host = Fake::new();
        let before = host.now;
        let mut active = None;
        host.route(
            &mut active,
            lifecycle,
            busy,
            &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
        )
        .unwrap();
        let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
        assert_eq!(
            (reply.status, reply.detail.reason, reply.detail.stage),
            (status, reason, stage)
        );
        assert!(active.is_none());
        assert_eq!(host.now, before);
        assert_eq!(reply.detail.progress, control::Progress::default());
    }
}
#[test]
fn active_duplicate_is_ignored_competitor_busy_and_completed_replay_is_new() {
    let mut host = Fake::new();
    let req = request(Command::Show);
    let mut active = None;
    assert_eq!(
        control::admit(
            &mut active,
            req,
            crate::control::NO_DESTINATION,
            Lifecycle::Ready,
            false,
            host.now
        ),
        Admission::Admitted
    );
    let original = active;
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, req).unwrap(),
    )
    .unwrap();
    assert_eq!(active, original);
    assert!(host.sent.is_empty());
    let other = Request {
        request_id: ID + 1,
        ..req
    };
    host.evidence.push_back(Some(evidence(
        &CallerMarker {
            request_id: ID + 1,
            ..marker(Command::Show)
        }
        .encode()
        .unwrap(),
    )));
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, other).unwrap(),
    )
    .unwrap();
    assert_eq!(active, original);
    assert_eq!(host.sent.len(), 1);
    assert_eq!(
        TerminalResponse::decode(&host.sent[0].data.as_data32())
            .unwrap()
            .status,
        Status::Busy
    );
    finish_active(&mut host, atoms(), &mut active, response()).unwrap();
    assert!(active.is_none());
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, req).unwrap(),
    )
    .unwrap();
    assert_eq!(host.sent.len(), 2);
    assert_eq!(active.unwrap().request, req); // replay is a new pending admission
    finish_active(&mut host, atoms(), &mut active, response()).unwrap();
    assert_eq!(host.sent.len(), 3);
}
#[test]
fn admission_deadline_is_fixed_at_admission_and_not_extended_by_duplicates() {
    for (command, millis) in [
        (Command::Hide, 1000),
        (Command::Show, 2500),
        (Command::BringTop, 3500),
    ] {
        let now = Instant::now();
        let req = request(command);
        let mut active = None;
        assert_eq!(
            control::admit(
                &mut active,
                req,
                crate::control::NO_DESTINATION,
                Lifecycle::Ready,
                false,
                now
            ),
            Admission::Admitted
        );
        assert_eq!(
            active.unwrap().deadline,
            now + Duration::from_millis(millis)
        );
        assert_eq!(active.unwrap().stage, Stage::Preflight);
        for elapsed in [500, 2499, 2500, 5000] {
            assert_eq!(
                control::admit(
                    &mut active,
                    req,
                    crate::control::NO_DESTINATION,
                    Lifecycle::Ready,
                    false,
                    now + Duration::from_millis(elapsed)
                ),
                Admission::Duplicate
            );
            assert_eq!(
                active.unwrap().deadline,
                now + Duration::from_millis(millis)
            );
        }
    }
}
#[test]
fn ownership_loss_before_or_during_validation_never_admits() {
    for owners in [[0, 0], [CONTROL, 0], [CONTROL, 11]] {
        let mut host = Fake::new();
        host.owners = VecDeque::from(owners);
        let mut active = None;
        assert!(host
            .route(
                &mut active,
                Lifecycle::Ready,
                false,
                &request_event(atoms(), CONTROL, request(Command::Show)).unwrap()
            )
            .is_err());
        assert!(active.is_none());
        assert!(host.sent.is_empty());
    }
}
#[test]
fn vanished_caller_after_admission_is_nonfatal_but_other_send_failure_is_fatal() {
    for failure in [false, true] {
        let mut host = Fake::new();
        host.send_ok = false;
        if failure {
            host.failure = Some("send");
        }
        let mut active = None;
        let result = host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
        );
        assert!(result.is_ok()); // Show admission never sends a terminal reply
        assert!(active.is_some());
        let completion = finish_active(&mut host, atoms(), &mut active, response());
        assert_eq!(completion.is_err(), failure);
        assert!(active.is_none());
        assert_eq!(host.sent.len(), 1);
        assert_eq!(
            TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap(),
            response()
        );
    }
}
#[test]
fn only_typed_badwindow_for_the_expected_peer_is_recoverable() {
    for (kind, bad_value, recoverable) in [
        (ErrorKind::Window, REPLY_WINDOW, true),
        (ErrorKind::Window, CONTROL, false),
        (ErrorKind::Atom, REPLY_WINDOW, false),
    ] {
        let error = x11rb::x11_utils::X11Error {
            error_kind: kind,
            error_code: 3,
            sequence: 1,
            bad_value,
            minor_opcode: 0,
            major_opcode: 25,
            extension_name: None,
            request_name: None,
        };
        let result = peer_result::<()>(Err(ReplyError::X11Error(error)), REPLY_WINDOW);
        if recoverable {
            assert_eq!(result.unwrap(), None);
        } else {
            assert!(result.is_err());
        }
    }
    assert!(peer_result::<()>(
        Err(ReplyError::ConnectionError(
            x11rb::errors::ConnectionError::UnknownError
        )),
        REPLY_WINDOW
    )
    .is_err());
}
#[test]
fn correlation_ignores_all_unrelated_envelopes_and_stops_at_first_valid_reply() {
    let good = reply_event(atoms(), REPLY_WINDOW, response()).unwrap();
    let mut host = Fake::new();
    for case in 0..7 {
        let mut event = good;
        match case {
            0 => event.window += 1,
            1 => event.type_ += 1,
            2 => event.format = 8,
            3 => {
                let mut w = event.data.as_data32();
                w[0] = (1 << 16) | 1;
                event.data = w.into();
            }
            4 => {
                let mut w = event.data.as_data32();
                w[1] += 1;
                event.data = w.into();
            }
            5 => {
                let mut w = event.data.as_data32();
                w[3] += 1;
                event.data = w.into();
            }
            6 => {
                let mut w = event.data.as_data32();
                w[4] |= 1 << 31;
                event.data = w.into();
            }
            _ => unreachable!(),
        }
        host.events.push_back(Event::ClientMessage(event));
    }
    host.events.extend([terminal(), terminal()]);
    assert_eq!(
        host.call_command().unwrap(),
        CallerOutcome::Terminal(response())
    );
    assert_eq!(host.events.len(), 1);
    assert_eq!(host.sent.len(), 1);
    let correlation = ReplyCorrelation {
        control_xid: CONTROL,
        epoch: 78,
        request_id: ID,
        reply_xid: REPLY_WINDOW,
    };
    assert!(correlated_reply(&terminal(), atoms(), owner(), correlation).is_none());
}
#[test]
fn one_four_second_budget_includes_discovery_setup_send_and_unrelated_traffic() {
    for step in ["atoms", "owner", "evidence", "watch", "create reply"] {
        let mut host = Fake::new();
        host.cost = Some((step, Duration::from_secs(4)));
        assert!(matches!(
            host.call_command().unwrap(),
            CallerOutcome::Unknown(_)
        ));
        assert!(host.sent.is_empty(), "no send after budget used by {step}");
    }
    let mut host = Fake::new();
    let start = host.now;
    host.cost = Some(("create reply", Duration::from_secs(3)));
    assert!(matches!(
        host.call_command().unwrap(),
        CallerOutcome::Unknown(_)
    ));
    assert_eq!(host.waits, [start + Duration::from_secs(4)]);
    assert_eq!(host.sent.len(), 1);
    let mut host = Fake::new();
    let start = host.now;
    host.cost = Some(("next", Duration::from_millis(10)));
    host.endless_events = true;
    assert!(matches!(
        host.call_command().unwrap(),
        CallerOutcome::Unknown(_)
    ));
    assert_eq!(host.now, start + Duration::from_secs(4));
    assert_eq!(host.sent.len(), 1);
    assert!(host.waits.is_empty());
}
#[test]
fn owner_disappearance_before_send_never_sends_to_stale_or_replacement_owner() {
    let mut host = Fake::new();
    host.watch_ok = false;
    assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
    assert!(host.sent.is_empty());
    let mut host = Fake::new();
    host.owners = VecDeque::from([CONTROL, CONTROL, 0]);
    assert_eq!(host.call_command().unwrap(), CallerOutcome::NoInstance);
    assert!(host.sent.is_empty());
    let mut host = Fake::new();
    host.evidence = VecDeque::from([Some(evidence(&owner().encode().unwrap())), None]);
    assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
    assert!(host.sent.is_empty());
    let mut host = Fake::new();
    host.evidence = VecDeque::from([
        Some(evidence(&owner().encode().unwrap())),
        Some(evidence(
            &OwnerDescriptor {
                epoch: 78,
                ..owner()
            }
            .encode()
            .unwrap(),
        )),
    ]);
    assert_eq!(host.call_command().unwrap(), CallerOutcome::Protocol);
    assert!(host.sent.is_empty());
}
#[test]
fn owner_death_drains_buffered_terminal_first_without_resend() {
    for events in [
        vec![terminal(), destroyed(false)],
        vec![destroyed(false), terminal()],
        vec![destroyed(true), terminal()],
    ] {
        let mut host = Fake::new();
        host.events = events.into();
        assert_eq!(
            host.call_command().unwrap(),
            CallerOutcome::Terminal(response())
        );
        assert_eq!(host.sent.len(), 1);
    }
    let mut host = Fake::new();
    host.events.push_back(destroyed(false));
    assert_eq!(
        host.call_command().unwrap(),
        CallerOutcome::Unknown("owner disappeared before terminal response")
    );
    assert!(host.waits.is_empty());
    assert_eq!(host.sent.len(), 1);
    let mut host = Fake::new();
    host.send_ok = false;
    assert!(matches!(
        host.call_command().unwrap(),
        CallerOutcome::Unknown(_)
    ));
    assert_eq!(host.sent.len(), 1);
}
#[test]
fn shutdown_completes_active_once_and_clears_even_on_reply_failure() {
    for fails in [false, true] {
        let mut host = Fake::new();
        if fails {
            host.failure = Some("send");
        }
        let mut active = None;
        control::admit(
            &mut active,
            request(Command::Show),
            crate::control::NO_DESTINATION,
            Lifecycle::Ready,
            false,
            host.now,
        );
        let closing = TerminalResponse {
            status: Status::Closing,
            detail: control::preflight_detail(Reason::Closing, Stage::Shutdown),
            ..response()
        };
        assert_eq!(
            finish_active(&mut host, atoms(), &mut active, closing).is_err(),
            fails
        );
        assert!(active.is_none());
        assert_eq!(host.sent.len(), 1);
    }
}
#[test]
fn direct_native_request_and_reply_use_no_propagation_or_event_mask() {
    for event in [
        request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
        reply_event(atoms(), REPLY_WINDOW, response()).unwrap(),
    ] {
        let native = direct_message(event);
        assert!(!native.propagate);
        assert_eq!(native.destination, event.window);
        assert_eq!(native.event_mask, EventMask::NO_EVENT);
        let bytes: [u8; 32] = event.into();
        assert_eq!(*native.event, bytes);
    }
}

#[test]
fn direct_starting_requests_do_not_extend_the_startup_wait() {
    use std::cell::{Cell, RefCell};
    let host = RefCell::new(Fake::new());
    let start = host.borrow().now;
    let now = Cell::new(start);
    let deadline = start + Duration::from_secs(2);
    let event = request_event(atoms(), CONTROL, request(Command::Show)).unwrap();
    let mut active = None;
    let result = wait_events_with::<()>(
        deadline,
        || now.get(),
        || {
            now.set(now.get() + Duration::from_millis(100));
            Ok(Some((Event::ClientMessage(event), 1)))
        },
        |_| panic!("continuous event traffic"),
        |event, _| {
            let Event::ClientMessage(event) = event else {
                unreachable!()
            };
            host.borrow_mut()
                .route(&mut active, Lifecycle::Starting, false, &event)?;
            Ok(None)
        },
    )
    .unwrap();
    assert_eq!(result, None);
    assert_eq!(now.get(), deadline);
    assert!(active.is_none());
    assert_eq!(host.borrow().sent.len(), 20);
    for reply in &host.borrow().sent {
        assert_eq!(
            TerminalResponse::decode(&reply.data.as_data32())
                .unwrap()
                .status,
            Status::Starting
        );
    }
}
#[test]
fn closing_boundary_drains_only_one_bounded_batch_without_admitting() {
    let event =
        Event::ClientMessage(request_event(atoms(), CONTROL, request(Command::Show)).unwrap());
    let mut queue = VecDeque::from(vec![event; 100]);
    let mut host = Fake::new();
    let mut active = None;
    closing_events_with(
        || Ok(queue.pop_front()),
        |event| {
            let Event::ClientMessage(event) = event else {
                unreachable!()
            };
            host.route(&mut active, Lifecycle::Closing, false, &event)?;
            Ok(false)
        },
    )
    .unwrap();
    assert_eq!(queue.len(), 36);
    assert!(active.is_none());
    assert_eq!(host.sent.len(), 64);
    for reply in host.sent {
        let reply = TerminalResponse::decode(&reply.data.as_data32()).unwrap();
        assert_eq!(reply.status, Status::Closing);
        assert_eq!(reply.detail.stage, Stage::Shutdown);
    }
}
#[test]
fn public_recover_admission_retains_the_sole_placement_slot_without_reply() {
    let command = Command::Recover;
    let mut host = Fake::new();
    host.evidence
        .push_back(Some(evidence(&marker(command).encode().unwrap())));
    let mut active = None;
    host.route(
        &mut active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(command)).unwrap(),
    )
    .unwrap();
    let operation = active.unwrap();
    assert_eq!(operation.request, request(command));
    assert_eq!(operation.placement.destination_desktop, 7);
    assert_eq!(operation.stage, Stage::Placement);
    assert_eq!(operation.recover_phase, control::RecoverPhase::Here);
    assert!(operation.placement.observation_due());
    assert_eq!(operation.deadline, host.now + Duration::from_millis(7000));
    assert!(host.sent.is_empty());
    assert_eq!(host.log, ["owner", "evidence", "owner"]);
}

#[test]
fn placement_transport_preserves_lifecycle_and_busy_admission_policy() {
    for command in [Command::BringHere, Command::Recover] {
        for (lifecycle, busy, status, reason, stage) in [
            (
                Lifecycle::Starting,
                false,
                Status::Starting,
                Reason::Starting,
                Stage::Preflight,
            ),
            (
                Lifecycle::Closing,
                false,
                Status::Closing,
                Reason::Closing,
                Stage::Shutdown,
            ),
            (
                Lifecycle::Ready,
                true,
                Status::Busy,
                Reason::Busy,
                Stage::Preflight,
            ),
        ] {
            let mut host = Fake::new();
            let mut active = None;
            host.evidence
                .push_back(Some(evidence(&marker(command).encode().unwrap())));
            host.route(
                &mut active,
                lifecycle,
                busy,
                &request_event(atoms(), CONTROL, request(command)).unwrap(),
            )
            .unwrap();
            assert!(active.is_none());
            assert_eq!(host.sent.len(), 1);
            let reply = TerminalResponse::decode(&host.sent[0].data.as_data32()).unwrap();
            assert_eq!(
                (reply.status, reply.detail.reason, reply.detail.stage),
                (status, reason, stage)
            );
        }
        let mut host = Fake::new();
        let mut active = None;
        control::admit(
            &mut active,
            request(Command::Show),
            control::NO_DESTINATION,
            Lifecycle::Ready,
            false,
            host.now,
        );
        let original = active;
        host.evidence
            .push_back(Some(evidence(&marker(command).encode().unwrap())));
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(command)).unwrap(),
        )
        .unwrap();
        assert_eq!(active, original);
        assert_eq!(
            TerminalResponse::decode(&host.sent[0].data.as_data32())
                .unwrap()
                .status,
            Status::Busy
        );
    }
}

#[test]
fn placement_marker_destination_is_frozen_even_when_a_duplicate_property_changes() {
    for command in [Command::BringHere, Command::Recover] {
        let mut host = Fake::new();
        let mut active = None;
        control::admit(
            &mut active,
            request(command),
            7,
            Lifecycle::Ready,
            false,
            host.now,
        );
        let original = active;
        let changed = CallerMarker {
            destination_desktop: 8,
            ..marker(command)
        };
        host.evidence
            .push_back(Some(evidence(&changed.encode().unwrap())));
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(command)).unwrap(),
        )
        .unwrap();
        assert_eq!(active, original);
        assert_eq!(
            TerminalResponse::decode(&host.sent[0].data.as_data32())
                .unwrap()
                .status,
            Status::Busy
        );
        assert_eq!(active.unwrap().placement.destination_desktop, 7);
    }
}

#[test]
fn v1_and_invalid_destination_markers_are_never_admitted_or_translated() {
    for command in [Command::Show, Command::BringHere, Command::Recover] {
        let valid = marker(command).encode().unwrap();
        let mut wrong_version = valid;
        wrong_version[1] = 1;
        let mut wrong_destination = valid;
        wrong_destination[7] = if command == Command::Show {
            7
        } else {
            control::NO_DESTINATION
        };
        let mut trailing = valid.to_vec();
        trailing.push(0);
        for words in [
            valid[..7].to_vec(),
            wrong_version.to_vec(),
            wrong_destination.to_vec(),
            trailing,
        ] {
            let mut host = Fake::new();
            let mut active = None;
            host.evidence.push_back(Some(evidence(&words)));
            host.route(
                &mut active,
                Lifecycle::Ready,
                false,
                &request_event(atoms(), CONTROL, request(command)).unwrap(),
            )
            .unwrap();
            assert!(active.is_none());
            assert!(host.sent.is_empty());
        }
    }
}

#[test]
fn matching_v2_callers_retain_all_three_existing_command_routes_and_budgets() {
    for command in [Command::Hide, Command::Show, Command::BringTop] {
        let mut host = Fake::new();
        host.events.push_back(terminal());
        assert_eq!(
            caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap(),
            CallerOutcome::Terminal(response())
        );
        assert_eq!(host.marker, Some(marker(command)));
        assert_eq!(host.marker.unwrap().encode().unwrap().len(), 8);
        assert_eq!(host.sent.len(), 1);
        assert_eq!(
            Request::decode(&host.sent[0].data.as_data32()).unwrap(),
            request(command)
        );
        let mut host = Fake::new();
        let mut active = None;
        host.evidence
            .push_back(Some(evidence(&marker(command).encode().unwrap())));
        host.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(command)).unwrap(),
        )
        .unwrap();
        assert_eq!(active.unwrap().request, request(command));
        assert!(host.sent.is_empty());
    }
    for (command, millis) in [
        (Command::Hide, 4000),
        (Command::Show, 4000),
        (Command::BringTop, 5000),
        (Command::BringHere, 5000),
        (Command::Recover, 9000),
    ] {
        assert_eq!(caller_budget(command), Duration::from_millis(millis));
    }
}

/// Test facade over actual transport admission, production command dispatch,
/// reply correlation and shutdown. No native connection or body constructor.
pub(crate) struct HereOwner {
    endpoint: ControlWindow,
    peer: Fake,
}
impl HereOwner {
    pub(crate) fn descriptor(&self) -> OwnerDescriptor {
        self.endpoint.descriptor
    }
    pub(crate) fn closing_with(
        &mut self,
        owner: impl FnOnce() -> Result<u32>,
        write: impl FnOnce(OwnerDescriptor) -> Result<()>,
    ) -> Result<()> {
        self.endpoint.closing_with(owner, write)
    }
    pub(crate) fn finish_remote(&mut self) -> Result<()> {
        self.endpoint.finish_remote_with(&mut self.peer)
    }
    /// Internal pre-admission only: never calls the public request router.
    pub(crate) fn recover(now: Instant, destination: u32) -> Self {
        let mut endpoint = ControlWindow::candidate(0, CONTROL, 77, 50, 51, 52);
        endpoint.descriptor = owner();
        endpoint.root = ROOT;
        endpoint.transport = atoms();
        endpoint.owns_selection = true;
        assert_eq!(
            control::admit(
                &mut endpoint.active,
                request(Command::Recover),
                destination,
                Lifecycle::Ready,
                false,
                now,
            ),
            Admission::Admitted
        );
        Self {
            endpoint,
            peer: Fake::new(),
        }
    }
    pub(crate) fn new(now: Instant, destination: u32) -> Self {
        Self::placement(now, Command::BringHere, destination)
    }
    /// Admit either placement command through the production transport router.
    pub(crate) fn placement(now: Instant, command: Command, destination: u32) -> Self {
        let mut endpoint = ControlWindow::candidate(0, CONTROL, 77, 50, 51, 52);
        endpoint.descriptor = owner();
        endpoint.root = ROOT;
        endpoint.transport = atoms();
        endpoint.owns_selection = true;
        let mut peer = Fake::new();
        peer.now = now;
        let mut result = Self { endpoint, peer };
        result.request_placement(command, ID, destination, false);
        assert!(result.peer.sent.is_empty());
        let active = result.active().unwrap();
        assert_eq!(active.request, request(command));
        assert_eq!(active.placement.destination_desktop, destination);
        assert_eq!(active.stage, Stage::Placement);
        assert!(active.placement.observation_due());
        assert_eq!(result.peer.log, ["owner", "evidence", "owner"]);
        result
    }
    pub(crate) fn request(&mut self, id: u64, destination: u32, busy: bool) {
        self.request_placement(Command::BringHere, id, destination, busy);
    }
    pub(crate) fn request_placement(
        &mut self,
        command: Command,
        id: u64,
        destination: u32,
        busy: bool,
    ) {
        assert!(matches!(command, Command::BringHere | Command::Recover));
        let marker = CallerMarker {
            request_id: id,
            destination_desktop: destination,
            ..marker(command)
        };
        self.peer
            .evidence
            .push_back(Some(evidence(&marker.encode().unwrap())));
        route_request_with(
            &mut self.peer,
            self.endpoint.descriptor,
            self.endpoint.root,
            self.endpoint.transport,
            Some(99),
            busy,
            &mut self.endpoint.active,
            &request_event(
                atoms(),
                CONTROL,
                Request {
                    request_id: id,
                    ..request(command)
                },
            )
            .unwrap(),
        )
        .unwrap();
    }
    pub(crate) fn active(&self) -> Option<&ActiveOperation> {
        self.endpoint.active()
    }
    pub(crate) fn active_mut(&mut self) -> Option<&mut ActiveOperation> {
        self.endpoint.active_mut()
    }
    pub(crate) fn service(
        &mut self,
        service: impl FnOnce(
            &mut Option<ActiveOperation>,
            u32,
            u32,
            &mut dyn FnMut(control::OperationResult) -> Result<()>,
        ) -> Result<()>,
    ) -> Result<bool> {
        self.endpoint
            .service_here_with_peer(&mut self.peer, service)
    }
    pub(crate) fn replies(&self) -> Vec<TerminalResponse> {
        self.peer
            .sent
            .iter()
            .map(|event| {
                assert_eq!((event.window, event.type_), (REPLY_WINDOW, atoms().reply));
                let reply = TerminalResponse::decode(&event.data.as_data32()).unwrap();
                assert_eq!(reply.control_xid, CONTROL);
                reply
            })
            .collect()
    }
    pub(crate) fn fail_reply(&mut self, fatal: bool) {
        if fatal {
            self.peer.failure = Some("send");
        } else {
            self.peer.send_ok = false;
        }
    }
    pub(crate) fn request_ownership_failure(
        &mut self,
        during_validation: bool,
        query_error: bool,
    ) -> HostError {
        if query_error {
            self.peer.failure = Some("owner");
        } else {
            self.peer.owners.extend(if during_validation {
                vec![CONTROL, CONTROL + 1]
            } else {
                vec![CONTROL + 1]
            });
        }
        self.peer
            .evidence
            .push_back(Some(evidence(&marker(Command::Show).encode().unwrap())));
        let error = self
            .peer
            .route(
                &mut self.endpoint.active,
                Lifecycle::Ready,
                false,
                &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
            )
            .unwrap_err();
        self.peer.failure = None;
        error
    }
    pub(crate) fn shutdown(&mut self, owned: bool) {
        self.endpoint.owns_selection = owned;
        self.endpoint.finish_remote_with(&mut self.peer).unwrap();
    }
    pub(crate) fn advance_peer(&mut self, elapsed: Duration) {
        self.peer.now += elapsed;
    }
    pub(crate) fn original_id(&self) -> u64 {
        ID
    }
}

#[test]
fn late_event_turnover_terminal_correlation_and_completed_replay_keep_existing_protocol() {
    // Actual admitted slots and production service/finalization produce these
    // replies. Body observations use the established public-roundtrip fixtures.
    for predecessor_command in [Command::BringHere, Command::Recover] {
        let finish = |endpoint: &mut HereOwner, command| {
            endpoint
                .service(|active, _, _, reply| match command {
                    Command::BringHere => crate::here_service_tests::public_roundtrip_service(
                        active,
                        false,
                        false,
                        control::LayerObservation::Above,
                        reply,
                    ),
                    Command::Recover => {
                        crate::here_service_tests::recover_tests::public_roundtrip_service(
                            active,
                            false,
                            false,
                            control::LayerObservation::Above,
                            crate::here_service_tests::recover_tests::PublicFault::None,
                            reply,
                        )
                    }
                    _ => unreachable!(),
                })
                .unwrap();
            assert!(endpoint.active().is_none());
        };
        let mut endpoint = HereOwner::placement(Instant::now(), predecessor_command, 2);
        finish(&mut endpoint, predecessor_command);
        let old = endpoint.replies()[0];
        assert_eq!(old.status, Status::Success);
        let old_event = Event::ClientMessage(reply_event(atoms(), REPLY_WINDOW, old).unwrap());
        endpoint.advance_peer(Duration::from_millis(100));
        let successor_command = if predecessor_command == Command::BringHere {
            Command::Recover
        } else {
            Command::BringHere
        };
        endpoint.request_placement(successor_command, ID + 1, 2, false);
        let successor = *endpoint.active().unwrap();
        let correlation = ReplyCorrelation {
            control_xid: CONTROL,
            epoch: 77,
            request_id: ID + 1,
            reply_xid: REPLY_WINDOW,
        };
        assert!(
            correlated_reply(&old_event, atoms(), endpoint.descriptor(), correlation).is_none()
        );
        // An active identical request is still a duplicate, with no new reply,
        // deadline, correction allowance or changed frozen destination.
        endpoint.request_placement(successor_command, ID + 1, 2, false);
        assert_eq!(endpoint.active(), Some(&successor));
        assert_eq!(endpoint.replies(), [old]);
        finish(&mut endpoint, successor_command);
        let fresh = endpoint.replies()[1];
        assert_eq!(fresh.request_id, ID + 1);
        let good = reply_event(atoms(), REPLY_WINDOW, fresh).unwrap();
        assert_eq!(
            correlated_reply(
                &Event::ClientMessage(good),
                atoms(),
                endpoint.descriptor(),
                correlation
            ),
            Some(fresh)
        );
        for field in 0..4 {
            let mut event = good;
            let mut descriptor = endpoint.descriptor();
            match field {
                0 => event.window += 1,
                1 => event.type_ += 1,
                2 => {
                    let mut words = event.data.as_data32();
                    words[3] += 1;
                    event.data = words.into();
                }
                _ => descriptor.epoch += 1,
            }
            assert!(correlated_reply(
                &Event::ClientMessage(event),
                atoms(),
                descriptor,
                correlation
            )
            .is_none());
        }
        // No completed-request replay cache: an identical valid request after
        // completion creates another terminal obligation under this contract.
        endpoint.advance_peer(Duration::from_millis(100));
        endpoint.request_placement(successor_command, ID + 1, 2, false);
        let replay = endpoint.active().unwrap();
        assert_eq!(replay.request, successor.request);
        assert!(replay.deadline > successor.deadline);
        assert_eq!(replay.placement, control::PlacementProgress::new(2));
        assert_eq!(endpoint.replies(), [old, fresh]);
        finish(&mut endpoint, successor_command);
        assert_eq!(endpoint.replies(), [old, fresh, fresh]);
        assert!(!endpoint
            .service(|_, _, _, _| panic!("no predecessor or successor obligation remains"))
            .unwrap());
    }
}

use crate::here_service_tests::recover_tests::PublicFault;

struct PublicPlacementCaller {
    caller: Fake,
    peer: Fake,
    endpoint: ControlWindow,
    remote: bool,
    minimized: bool,
    layer: control::LayerObservation,
    fault: PublicFault,
    drop_reply: bool,
    fail_reply: bool,
    reply_error: bool,
}
impl PeerHost for PublicPlacementCaller {
    fn now(&self) -> Instant {
        self.caller.now()
    }
    fn owner(&mut self) -> Result<u32> {
        self.caller.owner()
    }
    fn evidence(&mut self, window: u32, property: u32) -> Result<Option<EndpointEvidence>> {
        self.caller.evidence(window, property)
    }
    fn send(&mut self, event: ClientMessageEvent) -> Result<bool> {
        self.caller.send(event)?;
        let marker = self.caller.marker.unwrap();
        assert_eq!(marker.destination_desktop, 2);
        let command = Request::decode(&event.data.as_data32()).unwrap().command;
        assert_eq!(marker.command, command);
        assert_eq!(marker.encode().unwrap().len(), 8);
        self.peer
            .evidence
            .push_back(Some(evidence(&marker.encode().unwrap())));
        self.peer
            .route(&mut self.endpoint.active, Lifecycle::Ready, false, &event)?;
        let active = self.endpoint.active.as_ref().unwrap();
        assert_eq!(active.request.command, command);
        assert_eq!(active.stage, Stage::Placement);
        assert_eq!(active.placement.destination_desktop, 2);
        assert!(
            self.peer.sent.is_empty(),
            "admission must not reply Unsupported"
        );
        if self.fail_reply {
            self.peer.send_ok = false;
        }
        if self.reply_error {
            self.peer.failure = Some("send");
        }
        let serviced =
            self.endpoint
                .service_here_with_peer(&mut self.peer, |active, _, _, reply| {
                    if command == Command::Recover {
                        crate::here_service_tests::recover_tests::public_roundtrip_service(
                            active,
                            self.remote,
                            self.minimized,
                            self.layer,
                            self.fault,
                            reply,
                        )
                    } else {
                        crate::here_service_tests::public_roundtrip_service(
                            active,
                            self.remote,
                            self.minimized,
                            self.layer,
                            reply,
                        )
                    }
                });
        if self.reply_error {
            assert_eq!(serviced.unwrap_err().to_string(), "send");
        } else {
            assert!(serviced?);
        }
        assert!(self.endpoint.active.is_none());
        assert_eq!(self.peer.sent.len(), 1);
        if !self.drop_reply && !self.fail_reply && !self.reply_error {
            self.caller
                .events
                .push_back(Event::ClientMessage(self.peer.sent[0]));
        }
        Ok(true)
    }
}
impl CallerHost for PublicPlacementCaller {
    fn capture_desktop(&mut self) -> std::result::Result<CallerDesktop, DestinationError> {
        self.caller.capture_desktop()
    }
    fn revalidate_desktop(
        &mut self,
        captured: CallerDesktop,
    ) -> std::result::Result<(), DestinationError> {
        self.caller.revalidate_desktop(captured)
    }
    fn atoms(&mut self, screen: u32) -> Result<(u32, TransportAtoms)> {
        self.caller.atoms(screen)
    }
    fn watch(&mut self, owner: u32) -> Result<bool> {
        self.caller.watch(owner)
    }
    fn create_reply(&mut self, marker: CallerMarker) -> Result<u32> {
        self.caller.create_reply(marker)
    }
    fn next(&mut self) -> Result<Option<Event>> {
        self.caller.next()
    }
    fn wait(&mut self, deadline: Instant) -> Result<()> {
        self.caller.wait(deadline)
    }
    fn cleanup(&mut self) -> Result<()> {
        self.caller.cleanup()
    }
}
impl PublicPlacementCaller {
    fn new(remote: bool, minimized: bool, layer: control::LayerObservation) -> Self {
        let mut caller = Fake::new();
        caller.caller_pair = (5, 2);
        let mut endpoint = ControlWindow::candidate(0, CONTROL, 77, 50, 51, 52);
        endpoint.descriptor = owner();
        endpoint.transport = atoms();
        endpoint.root = ROOT;
        let mut peer = Fake::new();
        peer.now = caller.now;
        Self {
            caller,
            peer,
            endpoint,
            remote,
            minimized,
            layer,
            fault: PublicFault::None,
            drop_reply: false,
            fail_reply: false,
            reply_error: false,
        }
    }
    fn invoke(&mut self, flag: &str) -> CallerOutcome {
        let crate::CliMode::Control(command) = crate::parse_args([flag]).unwrap() else {
            panic!("caller role")
        };
        caller_with(self, ROOT, 0, command, || Ok(ID)).unwrap()
    }
    fn assert_single_roundtrip(&self, command: Command) {
        assert_eq!(self.caller.marker.unwrap().command, command);
        assert_eq!(self.caller.marker.unwrap().destination_desktop, 2);
        assert_eq!(self.caller.sent.len(), 1);
        assert_eq!(
            Request::decode(&self.caller.sent[0].data.as_data32()).unwrap(),
            request(command)
        );
        assert_eq!(
            self.caller
                .log
                .iter()
                .filter(|&&s| s == "create reply")
                .count(),
            1
        );
        assert_eq!(
            self.caller
                .log
                .iter()
                .filter(|&&s| s == "marker write")
                .count(),
            1
        );
        assert_eq!(self.peer.sent.len(), 1);
        let reply = self.peer.sent[0];
        assert_eq!(reply.window, REPLY_WINDOW);
        let response = TerminalResponse::decode(&reply.data.as_data32()).unwrap();
        assert_eq!(response.request_id, ID);
        assert_eq!(response.control_xid, CONTROL);
        assert_ne!(response.detail.reason, Reason::Unsupported);
        assert_eq!(self.caller.log.last(), Some(&"cleanup"));
        assert!(self.endpoint.active.is_none());
    }
}

#[test]
fn public_placement_roundtrip_uses_actual_cli_caller_admission_owner_and_correlated_reply() {
    for (flag, command) in [
        ("--bring-here", Command::BringHere),
        ("--recover", Command::Recover),
    ] {
        for remote in [false, true] {
            for minimized in [false, true] {
                for layer in [
                    control::LayerObservation::Normal,
                    control::LayerObservation::Above,
                    control::LayerObservation::Below,
                ] {
                    let mut host = PublicPlacementCaller::new(remote, minimized, layer);
                    let outcome = host.invoke(flag);
                    let CallerOutcome::Terminal(reply) = &outcome else {
                        panic!("{outcome:?}")
                    };
                    host.assert_single_roundtrip(command);
                    assert_eq!(outcome.exit_code() as i32, 0);
                    assert_eq!(reply.status, Status::Success);
                    assert_eq!(reply.detail.placement.history.relocation_confirmed, remote);
                    if command == Command::Recover {
                        assert_eq!(reply.detail.stage, Stage::Layer);
                        assert_eq!(reply.detail.layer, control::LayerObservation::Above);
                        assert_eq!(
                            reply.detail.visibility,
                            control::Visibility::NotMinimizedHere
                        );
                        assert!(reply.detail.progress.input_ready);
                        assert_eq!(
                            reply.detail.placement.current.geometry,
                            control::GeometryProof::Verified
                        );
                        assert!(!reply.detail.progress.placement_deferred);
                        assert_eq!(
                            reply.detail.placement.history.restoration_confirmed,
                            minimized
                        );
                        let text = outcome.diagnostic(command);
                        for meaning in [
                            "same existing body",
                            "frozen workspace",
                            "not minimized and Above",
                            "placement verified, input ready",
                        ] {
                            assert!(text.contains(meaning), "{text}");
                        }
                        if !remote && !minimized && layer == control::LayerObservation::Above {
                            assert!(!reply.detail.progress.host_mutation_dispatched);
                            assert!(!text.contains("confirmed earlier"));
                        }
                    } else {
                        assert_eq!(reply.detail.layer, layer);
                        assert_eq!(reply.detail.progress.placement_deferred, minimized);
                    }
                }
            }
        }
    }
}

#[test]
fn public_recover_failures_preserve_history_and_terminal_correlation() {
    for (fault, remote, minimized, status, reason, relocation, restoration) in [
        (
            PublicFault::Layout,
            false,
            false,
            Status::Failed,
            Reason::LayoutChanged,
            false,
            false,
        ),
        (
            PublicFault::AfterRelocation,
            true,
            true,
            Status::Partial,
            Reason::LayoutChanged,
            true,
            false,
        ),
        (
            PublicFault::AfterRestoration,
            false,
            true,
            Status::Partial,
            Reason::LayoutChanged,
            false,
            true,
        ),
        (
            PublicFault::AfterBoth,
            true,
            true,
            Status::Partial,
            Reason::LayoutChanged,
            true,
            true,
        ),
        (
            PublicFault::OwnerLost,
            true,
            true,
            Status::Failed,
            Reason::OwnershipLost,
            false,
            false,
        ),
        (
            PublicFault::Native,
            true,
            true,
            Status::Failed,
            Reason::X11Failure,
            false,
            false,
        ),
        (
            PublicFault::NativeDispatch,
            true,
            true,
            Status::Failed,
            Reason::X11Failure,
            false,
            false,
        ),
    ] {
        let mut host =
            PublicPlacementCaller::new(remote, minimized, control::LayerObservation::Below);
        host.fault = fault;
        let outcome = host.invoke("--recover");
        host.assert_single_roundtrip(Command::Recover);
        let CallerOutcome::Terminal(reply) = &outcome else {
            panic!("{fault:?}: {outcome:?}")
        };
        assert_eq!(reply.status, status, "{fault:?}");
        assert_eq!(reply.detail.reason, reason, "{fault:?}");
        assert_eq!(
            reply.detail.placement.history,
            control::RecoveryHistory {
                relocation_confirmed: relocation,
                restoration_confirmed: restoration
            }
        );
        assert_eq!(
            outcome.exit_code() as i32,
            if status == Status::Partial { 9 } else { 7 }
        );
        let text = outcome.diagnostic(Command::Recover);
        assert!(
            text.contains(if status == Status::Partial {
                "not currently confirmed"
            } else {
                "complete Recover was not confirmed"
            }),
            "{text}"
        );
        assert!(!text.contains("input ready"));
        if fault == PublicFault::NativeDispatch {
            assert!(reply.detail.progress.host_mutation_dispatched);
            assert!(text.contains("may have reached the WM and later state may still change"));
        } else if fault == PublicFault::Layout {
            assert!(!reply.detail.progress.host_mutation_dispatched);
        }
    }
}

#[test]
fn public_recover_lost_terminal_or_disappeared_caller_is_bounded_unknown_without_resend() {
    for loss in 0..3 {
        let mut host = PublicPlacementCaller::new(true, true, control::LayerObservation::Below);
        host.drop_reply = loss == 0;
        host.fail_reply = loss == 1;
        host.reply_error = loss == 2;
        let started = host.caller.now;
        let outcome = host.invoke("--recover");
        host.assert_single_roundtrip(Command::Recover);
        assert_eq!(outcome.exit_code() as i32, 8);
        assert!(outcome
            .diagnostic(Command::Recover)
            .contains("outcome unknown"));
        assert_eq!(host.caller.waits, [started + Duration::from_millis(9000)]);
        // Owner success is deliberately unavailable to this caller.
        assert_eq!(
            TerminalResponse::decode(&host.peer.sent[0].data.as_data32())
                .unwrap()
                .status,
            Status::Success
        );
    }
}

#[test]
fn recover_fixed_nine_second_deadline_covers_every_synchronous_boundary_exactly() {
    let budget = Duration::from_millis(9000);
    for (step, occurrence) in [
        ("capture", 1),
        ("caller count", 1),
        ("caller current", 1),
        ("atoms", 1),
        ("owner", 1),
        ("evidence", 1),
        ("owner", 2),
        ("watch", 1),
        ("create reply", 1),
        ("marker write", 1),
        ("owner", 3),
        ("evidence", 2),
        ("owner", 4),
        ("revalidate", 1),
        ("caller count", 3),
        ("caller current", 3),
        ("caller count", 4),
        ("caller current", 4),
        ("send", 1),
        ("next", 1),
    ] {
        for delta in [-1_i64, 0, 1] {
            let mut h = Fake::new();
            let start = h.now;
            let elapsed = if delta < 0 {
                budget - Duration::from_nanos(1)
            } else {
                budget + Duration::from_nanos(delta as u64)
            };
            h.step_costs.push((step, occurrence, elapsed));
            h.events.push_back(terminal());
            let outcome = caller_with(&mut h, ROOT, 0, Command::Recover, || Ok(ID)).unwrap();
            let dispatched = delta < 0 || matches!(step, "send" | "next");
            assert_eq!(
                h.sent.len(),
                usize::from(dispatched),
                "{step}/{occurrence}/{delta}"
            );
            assert_eq!(
                matches!(outcome, CallerOutcome::Terminal(_)),
                delta < 0,
                "{step}/{occurrence}/{delta}"
            );
            if delta >= 0 {
                assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
                assert_eq!(
                    outcome
                        .diagnostic(Command::Recover)
                        .contains("request not sent"),
                    !dispatched
                );
            }
            assert_eq!(h.now, start + elapsed);
            assert!(h.log.iter().filter(|&&s| s == "capture").count() == 1);
            assert!(h.log.iter().filter(|&&s| s == "marker write").count() <= 1);
            assert_eq!(h.log.last(), Some(&"cleanup"));
        }
    }
    for elapsed in [
        budget - Duration::from_nanos(1),
        budget,
        budget + Duration::from_nanos(1),
    ] {
        let mut h = Fake::new();
        let clock = h.id_elapsed.clone();
        h.events.push_back(terminal());
        let outcome = caller_with(&mut h, ROOT, 0, Command::Recover, || {
            clock.set(elapsed);
            Ok(ID)
        })
        .unwrap();
        assert_eq!(
            matches!(outcome, CallerOutcome::Terminal(_)),
            elapsed < budget
        );
        assert_eq!(h.sent.len(), usize::from(elapsed < budget));
        assert_eq!(h.marker.is_some(), elapsed < budget);
    }
    // Several separate calls consume one budget; revalidation cannot renew it.
    let mut h = Fake::new();
    let start = h.now;
    h.step_costs = vec![
        ("capture", 1, Duration::from_millis(2000)),
        ("marker write", 1, Duration::from_millis(2000)),
        ("revalidate", 1, Duration::from_millis(2000)),
    ];
    let outcome = caller_with(&mut h, ROOT, 0, Command::Recover, || Ok(ID)).unwrap();
    assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
    assert_eq!(h.sent.len(), 1);
    assert_eq!(h.waits, [start + budget]);
    assert_eq!(h.now, start + budget);
    // Waiting itself and an endless stream of unrelated replies stay bounded.
    for step in ["wait", "next"] {
        let mut h = Fake::new();
        let start = h.now;
        h.cost = Some((
            step,
            if step == "wait" {
                budget
            } else {
                Duration::from_millis(10)
            },
        ));
        h.endless_events = step == "next";
        let outcome = caller_with(&mut h, ROOT, 0, Command::Recover, || Ok(ID)).unwrap();
        assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown);
        assert_eq!(h.now, start + budget);
        assert_eq!(h.sent.len(), 1);
    }
}

#[test]
fn recover_reply_correlation_ignores_wrong_envelopes_and_stale_owner_identity() {
    let good = reply_event(atoms(), REPLY_WINDOW, response()).unwrap();
    let mut h = Fake::new();
    for case in 0..7 {
        let mut event = good;
        let mut words = event.data.as_data32();
        match case {
            0 => event.window += 1,
            1 => event.type_ += 1,
            2 => event.format = 8,
            3 => words[0] ^= 1 << 16,
            4 => words[1] ^= 1,
            5 => words[3] += 1,
            _ => words[4] |= 1 << 31,
        }
        event.data = words.into();
        h.events.push_back(Event::ClientMessage(event));
    }
    h.events.extend([terminal(), terminal()]);
    assert_eq!(
        caller_with(&mut h, ROOT, 0, Command::Recover, || Ok(ID)).unwrap(),
        CallerOutcome::Terminal(response())
    );
    assert_eq!(
        h.events.len(),
        1,
        "accept only the first exact terminal reply"
    );
    assert_eq!(h.sent.len(), 1);
    let correlation = ReplyCorrelation {
        control_xid: CONTROL,
        epoch: 77,
        request_id: ID,
        reply_xid: REPLY_WINDOW,
    };
    // Epoch is deliberately absent from the unchanged terminal wire. It is
    // checked against the captured descriptor, never invented from a reply.
    for stale_owner in [
        OwnerDescriptor {
            epoch: 78,
            ..owner()
        },
        OwnerDescriptor {
            control_xid: CONTROL + 1,
            ..owner()
        },
    ] {
        assert!(correlated_reply(&terminal(), atoms(), stale_owner, correlation).is_none());
    }
}

#[test]
fn recover_public_duplicates_competitors_and_local_layer_busy_preserve_the_active_slot() {
    let commands = [
        Command::Hide,
        Command::Show,
        Command::BringTop,
        Command::BringHere,
        Command::Recover,
    ];
    let mut h = Fake::new();
    let mut endpoint = ControlWindow::candidate(0, CONTROL, 77, 50, 51, 52);
    endpoint.descriptor = owner();
    endpoint.root = ROOT;
    endpoint.transport = atoms();
    let frozen = CallerMarker {
        destination_desktop: 2,
        ..marker(Command::Recover)
    };
    h.evidence
        .push_back(Some(evidence(&frozen.encode().unwrap())));
    h.route(
        &mut endpoint.active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::Recover)).unwrap(),
    )
    .unwrap();
    let original = endpoint.active;
    h.evidence
        .push_back(Some(evidence(&frozen.encode().unwrap())));
    h.now += Duration::from_secs(1);
    h.route(
        &mut endpoint.active,
        Lifecycle::Ready,
        true,
        &request_event(atoms(), CONTROL, request(Command::Recover)).unwrap(),
    )
    .unwrap();
    assert_eq!(endpoint.active, original);
    assert!(
        h.sent.is_empty(),
        "duplicate retains the existing terminal obligation"
    );
    for (i, command) in commands.into_iter().enumerate() {
        let competing = Request {
            request_id: ID + 1 + i as u64,
            ..request(command)
        };
        h.evidence.push_back(Some(evidence(
            &CallerMarker {
                request_id: competing.request_id,
                ..marker(command)
            }
            .encode()
            .unwrap(),
        )));
        h.route(
            &mut endpoint.active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, competing).unwrap(),
        )
        .unwrap();
        assert_eq!(
            endpoint.active, original,
            "state/deadline/history preserved"
        );
        let reply = TerminalResponse::decode(&h.sent.last().unwrap().data.as_data32()).unwrap();
        assert_eq!(
            (reply.status, reply.detail.reason, reply.request_id),
            (Status::Busy, Reason::Busy, competing.request_id)
        );
    }
    // Complete the actual Recover through the existing owner, then admit anew.
    assert!(endpoint
        .service_here_with_peer(&mut h, |active, _, _, reply| {
            crate::here_service_tests::recover_tests::public_roundtrip_service(
                active,
                false,
                false,
                control::LayerObservation::Above,
                PublicFault::None,
                reply,
            )
        })
        .unwrap());
    assert!(endpoint.active.is_none());
    for command in commands {
        let mut active = None;
        h.evidence
            .push_back(Some(evidence(&marker(command).encode().unwrap())));
        h.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(command)).unwrap(),
        )
        .unwrap();
        let original = active;
        let competing = Request {
            request_id: ID + 100,
            ..request(Command::Recover)
        };
        h.evidence.push_back(Some(evidence(
            &CallerMarker {
                request_id: competing.request_id,
                ..marker(Command::Recover)
            }
            .encode()
            .unwrap(),
        )));
        h.route(
            &mut active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, competing).unwrap(),
        )
        .unwrap();
        assert_eq!(active, original);
        assert_eq!(
            TerminalResponse::decode(&h.sent.last().unwrap().data.as_data32())
                .unwrap()
                .status,
            Status::Busy
        );
    }
    h.evidence
        .push_back(Some(evidence(&marker(Command::Recover).encode().unwrap())));
    h.route(
        &mut endpoint.active,
        Lifecycle::Ready,
        true,
        &request_event(atoms(), CONTROL, request(Command::Recover)).unwrap(),
    )
    .unwrap();
    assert!(endpoint.active.is_none());
    assert_eq!(
        TerminalResponse::decode(&h.sent.last().unwrap().data.as_data32())
            .unwrap()
            .status,
        Status::Busy
    );
    h.evidence
        .push_back(Some(evidence(&marker(Command::Show).encode().unwrap())));
    let before = h.sent.len();
    h.route(
        &mut endpoint.active,
        Lifecycle::Ready,
        false,
        &request_event(atoms(), CONTROL, request(Command::Show)).unwrap(),
    )
    .unwrap();
    assert_eq!(endpoint.active.unwrap().request.command, Command::Show);
    assert_eq!(
        h.sent.len(),
        before,
        "new command admitted after Recover terminal"
    );
}

#[test]
fn recover_diagnostics_distinguish_every_history_partition_and_reason_at_the_actual_stage() {
    for (fault, remote, minimized, expected) in [
        (
            PublicFault::Layout,
            false,
            false,
            "neither relocation nor restoration was confirmed as history",
        ),
        (
            PublicFault::AfterRelocation,
            true,
            true,
            "relocation was confirmed earlier; restoration was not confirmed as history",
        ),
        (
            PublicFault::AfterRestoration,
            false,
            true,
            "restoration was confirmed earlier; relocation was not confirmed as history",
        ),
        (
            PublicFault::AfterBoth,
            true,
            true,
            "relocation and restoration were both confirmed earlier",
        ),
    ] {
        let mut h = PublicPlacementCaller::new(remote, minimized, control::LayerObservation::Below);
        h.fault = fault;
        let CallerOutcome::Terminal(mut response) = h.invoke("--recover") else {
            panic!("terminal")
        };
        for (reason, meaning) in [
            (Reason::ConfirmationTimeout, "confirmation timed out"),
            (Reason::LayoutChanged, "frozen workspace/layout changed"),
            (Reason::LayerChanged, "effective layer changed incompatibly"),
            (Reason::OwnershipLost, "owner selection lost"),
            (Reason::X11Failure, "native X11 failure"),
            (Reason::Closing, "owner is closing"),
            (Reason::BodyDestroyed, "existing body was destroyed"),
            (Reason::Withdrawn, "existing body was withdrawn"),
            (Reason::Unverifiable, "required state could not be verified"),
            (Reason::Unsupported, "required WM support unavailable"),
            (
                Reason::Busy,
                "another remote or local layer/placement operation is active",
            ),
        ] {
            response.detail.reason = reason;
            let outcome = CallerOutcome::Terminal(response);
            let text = outcome.diagnostic(Command::Recover);
            assert!(text.contains(expected), "{text}");
            assert!(text.contains(meaning), "{text}");
            assert!(
                text.contains(&format!("({:?})", response.detail.stage)),
                "{text}"
            );
            assert!(!text.contains("input ready"));
            assert_eq!(
                outcome.exit_code() as i32,
                if fault == PublicFault::Layout { 7 } else { 9 }
            );
        }
    }
    let mut busy = response();
    busy.status = Status::Busy;
    busy.detail.reason = Reason::Busy;
    assert!(CallerOutcome::Terminal(busy)
        .diagnostic(Command::Recover)
        .contains("Busy — Recover was not completed"));
}

#[test]
fn placement_marker_builder_rejects_legacy_commands_without_changing_the_v2_codec() {
    let mut h = Fake::new();
    let captured = h.capture_desktop().unwrap();
    for command in [Command::Hide, Command::Show, Command::BringTop] {
        assert!(placement_caller_marker(command, captured, owner(), ID).is_err());
    }
}

#[test]
fn production_placement_dispatch_excludes_all_legacy_commands() {
    for command in [Command::Hide, Command::Show, Command::BringTop] {
        let mut peer = Fake::new();
        let mut endpoint = ControlWindow::candidate(0, CONTROL, 77, 50, 51, 52);
        endpoint.descriptor = owner();
        endpoint.root = ROOT;
        endpoint.transport = atoms();
        peer.evidence
            .push_back(Some(evidence(&marker(command).encode().unwrap())));
        peer.route(
            &mut endpoint.active,
            Lifecycle::Ready,
            false,
            &request_event(atoms(), CONTROL, request(command)).unwrap(),
        )
        .unwrap();
        let active = endpoint.active;
        assert!(!endpoint
            .service_here_with_peer(&mut peer, |_, _, _, _| panic!(
                "non-Here must never compose Here"
            ))
            .unwrap());
        assert_eq!(endpoint.active, active);
        assert!(active.is_some());
        assert!(peer.sent.is_empty());
    }
}

#[test]
fn public_placement_admission_rejects_selection_loss_before_and_after_marker_validation() {
    for command in [Command::BringHere, Command::Recover] {
        for owners in [vec![CONTROL + 1], vec![CONTROL, CONTROL + 1]] {
            let mut host = Fake::new();
            host.owners = owners.into();
            host.evidence
                .push_back(Some(evidence(&marker(command).encode().unwrap())));
            let mut active = None;
            assert!(host
                .route(
                    &mut active,
                    Lifecycle::Ready,
                    false,
                    &request_event(atoms(), CONTROL, request(command)).unwrap()
                )
                .is_err());
            assert!(active.is_none());
            assert!(host.sent.is_empty());
        }
    }
}

#[test]
fn public_placement_capture_marker_rediscovery_revalidation_send_order_is_exact() {
    for command in [Command::BringHere, Command::Recover] {
        let mut host = Fake::new();
        host.events.push_back(terminal());
        let outcome = caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap();
        assert_eq!(outcome, CallerOutcome::Terminal(response()));
        assert_eq!(host.marker, Some(marker(command)));
        assert_eq!(host.sent.len(), 1);
        assert_eq!(
            Request::decode(&host.sent[0].data.as_data32()).unwrap(),
            request(command)
        );
        assert_eq!(
            host.log,
            [
                "capture",
                "caller count",
                "caller current",
                "caller count",
                "caller current",
                "atoms",
                "owner",
                "evidence",
                "owner",
                "watch",
                "create reply",
                "marker write",
                "owner",
                "evidence",
                "owner",
                "revalidate",
                "caller count",
                "caller current",
                "caller count",
                "caller current",
                "send",
                "next",
                "cleanup",
            ]
        );
    }
}

#[test]
fn public_placement_capture_failure_or_invalid_two_read_evidence_never_discovers_or_sends() {
    for command in [Command::BringHere, Command::Recover] {
        for case in 0..13 {
            let mut host = Fake::new();
            match case {
                0 => host.failure = Some("capture"),
                1 => host.failure = Some("caller current"),
                2 => host.caller_pair = (0, 0),
                3 => host.caller_pair = (9, 9),
                4 => host.caller_pair = (9, u32::MAX),
                5 => host
                    .caller_snapshots
                    .push_back(Ok((GetPropertyReply::default(), cardinal(7)))),
                6 => host
                    .caller_snapshots
                    .push_back(Ok((cardinal(9), cardinal(6)))),
                7 => host
                    .caller_snapshots
                    .push_back(Ok((cardinal(9), GetPropertyReply::default()))),
                8 => {
                    let mut count = cardinal(9);
                    count.format = 8;
                    host.caller_snapshots.push_back(Ok((count, cardinal(7))));
                }
                9 => {
                    let mut current = cardinal(7);
                    current.type_ = AtomEnum::STRING.into();
                    host.caller_snapshots.push_back(Ok((cardinal(9), current)));
                }
                10 => host.failure = Some("caller count"),
                11 => host.caller_snapshots.extend([
                    Ok((cardinal(9), cardinal(7))),
                    Ok((cardinal(10), cardinal(7))),
                ]),
                _ => host
                    .caller_snapshots
                    .push_back(Err(DestinationError::Unverifiable(
                        "native capture failure".into(),
                    ))),
            }
            let outcome = caller_with(&mut host, ROOT, 0, command, || {
                panic!("no ID before capture")
            })
            .unwrap();
            assert_eq!(outcome.exit_code(), ExitCode::Protocol);
            assert!(outcome.diagnostic(command).contains("capture failed"));
            assert!(outcome.diagnostic(command).contains("request not sent"));
            assert!(!host.log.contains(&"owner"));
            assert!(host.marker.is_none());
            assert!(host.sent.is_empty());
            assert_eq!(host.log.last(), Some(&"cleanup"));
        }
    }
}

#[test]
fn public_placement_discovery_drift_never_retargets_rewrites_retries_or_sends() {
    for command in [Command::BringHere, Command::Recover] {
        for (count, current) in [(10, 7), (9, 6), (9, u32::MAX), (0, 0)] {
            let mut host = Fake::new();
            host.discovery_pair = Some((count, current));
            let outcome = caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap();
            assert_eq!(outcome.exit_code(), ExitCode::Protocol);
            assert!(outcome
                .diagnostic(command)
                .contains("frozen caller workspace changed"));
            assert_eq!(host.marker, Some(marker(command)));
            assert_eq!(host.log.iter().filter(|&&s| s == "create reply").count(), 1);
            assert_eq!(host.log.iter().filter(|&&s| s == "capture").count(), 1);
            assert_eq!(host.log.iter().filter(|&&s| s == "revalidate").count(), 1);
            assert!(host.sent.is_empty());
            assert_eq!(host.log.last(), Some(&"cleanup"));
        }
        let mut host = Fake::new();
        host.failure = Some("revalidate");
        assert_eq!(
            caller_with(&mut host, ROOT, 0, command, || Ok(ID))
                .unwrap()
                .exit_code(),
            ExitCode::Protocol
        );
        assert_eq!(host.marker, Some(marker(command)));
        assert_eq!(host.log.iter().filter(|&&s| s == "create reply").count(), 1);
        assert!(host.sent.is_empty());
        assert_eq!(host.log.last(), Some(&"cleanup"));
    }
}

#[test]
fn public_placement_capture_and_revalidation_share_original_command_deadline() {
    for command in [Command::BringHere, Command::Recover] {
        for step in [
            "capture",
            "atoms",
            "owner",
            "evidence",
            "watch",
            "create reply",
            "revalidate",
            "caller count",
            "caller current",
        ] {
            let mut host = Fake::new();
            host.cost = Some((step, caller_budget(command)));
            let outcome = caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap();
            assert_eq!(outcome.exit_code(), ExitCode::OutcomeUnknown, "{step}");
            assert!(outcome.diagnostic(command).contains("request not sent"));
            assert!(host.sent.is_empty(), "{step}");
            assert_eq!(host.log.last(), Some(&"cleanup"));
        }
        for step in ["capture", "create reply", "revalidate"] {
            let mut host = Fake::new();
            let start = host.now;
            host.cost = Some((step, caller_budget(command) / 3));
            assert_eq!(
                caller_with(&mut host, ROOT, 0, command, || Ok(ID))
                    .unwrap()
                    .exit_code(),
                ExitCode::OutcomeUnknown
            );
            assert_eq!(host.waits, [start + caller_budget(command)]);
            assert_eq!(host.now, start + caller_budget(command));
            assert_eq!(host.sent.len(), 1);
        }
    }
}

#[test]
fn public_placement_cleanup_covers_each_setup_send_and_wait_failure_boundary() {
    for command in [Command::BringHere, Command::Recover] {
        for step in [
            "atoms",
            "owner",
            "evidence",
            "watch",
            "create reply",
            "send",
            "next",
            "wait",
        ] {
            let mut host = Fake::new();
            host.failure = Some(step);
            assert_eq!(
                caller_with(&mut host, ROOT, 0, command, || Ok(ID))
                    .unwrap_err()
                    .to_string(),
                step
            );
            assert_eq!(host.log.last(), Some(&"cleanup"));
            assert_eq!(host.log.iter().filter(|&&s| s == "cleanup").count(), 1);
            assert!(host.sent.len() <= 1);
        }
        let mut host = Fake::new();
        assert!(caller_with(&mut host, ROOT, 0, command, || Err("id failed".into())).is_err());
        assert_eq!(host.log.last(), Some(&"cleanup"));
        assert!(host.sent.is_empty());
    }
}

#[test]
fn public_placement_owner_replacement_epoch_or_lifecycle_change_prevents_dispatch() {
    for command in [Command::BringHere, Command::Recover] {
        for case in 0..4 {
            let mut host = Fake::new();
            let changed = OwnerDescriptor {
                control_xid: if case == 0 { CONTROL + 1 } else { CONTROL },
                epoch: if case == 1 { 78 } else { 77 },
                lifecycle: match case {
                    2 => Lifecycle::Starting,
                    3 => Lifecycle::Closing,
                    _ => Lifecycle::Ready,
                },
                ..owner()
            };
            host.owners =
                VecDeque::from([CONTROL, CONTROL, changed.control_xid, changed.control_xid]);
            host.evidence = VecDeque::from([
                Some(evidence(&owner().encode().unwrap())),
                Some(evidence(&changed.encode().unwrap())),
            ]);
            let outcome = caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap();
            assert!(!matches!(outcome, CallerOutcome::Terminal(_)));
            assert_eq!(host.marker, Some(marker(command)));
            assert!(!host.log.contains(&"revalidate"));
            assert!(host.sent.is_empty());
            assert_eq!(host.log.last(), Some(&"cleanup"));
        }
    }
}

#[test]
fn public_placement_replies_keep_full_correlation_and_owner_loss_is_bounded_unknown() {
    for command in [Command::BringHere, Command::Recover] {
        let mut host = Fake::new();
        for case in 0..4 {
            let mut reply = reply_event(atoms(), REPLY_WINDOW, response()).unwrap();
            let mut words = reply.data.as_data32();
            match case {
                0 => words[1] ^= 1,
                1 => words[3] += 1,
                2 => reply.window += 1,
                _ => reply.type_ += 1,
            }
            reply.data = words.into();
            host.events.push_back(Event::ClientMessage(reply));
        }
        host.events.push_back(terminal());
        assert_eq!(
            caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap(),
            CallerOutcome::Terminal(response())
        );
        assert_eq!(host.log.iter().filter(|&&s| s == "next").count(), 5);
        assert_eq!(host.sent.len(), 1);
        for events in [vec![destroyed(false)], vec![]] {
            let mut host = Fake::new();
            let start = host.now;
            host.events = events.into();
            assert_eq!(
                caller_with(&mut host, ROOT, 0, command, || Ok(ID))
                    .unwrap()
                    .exit_code(),
                ExitCode::OutcomeUnknown
            );
            assert!(host.now <= start + caller_budget(command));
            assert_eq!(host.sent.len(), 1);
            assert_eq!(host.log.last(), Some(&"cleanup"));
        }
    }
}

#[test]
fn public_here_legacy_callers_do_not_read_here_identity() {
    for command in [Command::Hide, Command::Show, Command::BringTop] {
        let mut host = Fake::new();
        host.failure = Some("capture");
        host.caller_pair = (0, u32::MAX);
        host.events.push_back(terminal());
        assert_eq!(
            caller_with(&mut host, ROOT, 0, command, || Ok(ID)).unwrap(),
            CallerOutcome::Terminal(response())
        );
        assert_eq!(
            host.marker.unwrap().destination_desktop,
            control::NO_DESTINATION
        );
        assert!(!host.log.iter().any(|s| [
            "capture",
            "revalidate",
            "caller count",
            "caller current"
        ]
        .contains(s)));
    }
}

#[test]
fn prepared_placement_marker_freezes_only_the_validated_destination_in_existing_v2_shape() {
    for command in [Command::BringHere, Command::Recover] {
        use super::super::monitors::destination::capture_caller_with;
        let cardinal = |word: u32| GetPropertyReply {
            format: 32,
            type_: AtomEnum::CARDINAL.into(),
            length: 1,
            value_len: 1,
            value: word.to_ne_bytes().to_vec(),
            ..Default::default()
        };
        let mut reads = 0;
        let captured = capture_caller_with(|| {
            reads += 1;
            Ok((cardinal(9_u32), cardinal(7_u32)))
        })
        .unwrap();
        assert_eq!(reads, 2);
        let marker = placement_caller_marker(command, captured, owner(), ID).unwrap();
        assert_eq!(marker, self::marker(command));
        let words = marker.encode().unwrap();
        assert_eq!(words.len(), 8);
        assert_eq!(words[1], control::VERSION);
        assert_eq!(words[7], 7);
        assert_eq!(CallerMarker::decode(&words).unwrap(), marker);
        let mut host = Fake::new();
        host.create_reply(marker).unwrap();
        assert_eq!(host.log, ["create reply", "marker write"]);
        assert!(host.sent.is_empty(), "preparation does not dispatch");
    }
}

#[test]
fn pre_send_capture_failure_or_drift_cannot_rewrite_the_marker_or_choose_another_target() {
    for command in [Command::BringHere, Command::Recover] {
        use super::super::monitors::destination::{capture_caller_with, DestinationError};
        let cardinal = |word: u32| GetPropertyReply {
            format: 32,
            type_: AtomEnum::CARDINAL.into(),
            length: 1,
            value_len: 1,
            value: word.to_ne_bytes().to_vec(),
            ..Default::default()
        };
        let captured = capture_caller_with(|| Ok((cardinal(9), cardinal(7)))).unwrap();
        // Retain the D2 seam-level validation cases beside public routing tests.
        for case in 0..9 {
            let mut host = Fake::new();
            let original = placement_caller_marker(command, captured, owner(), ID).unwrap();
            host.create_reply(original).unwrap();
            let mut reads = 0;
            let fresh = capture_caller_with(|| {
                reads += 1;
                let mut count = cardinal(9);
                let mut current = cardinal(7);
                match case {
                    0 => count = cardinal(10),
                    1 => current = cardinal(6),
                    2 => current = cardinal(u32::MAX),
                    3 => current = cardinal(9),
                    4 => current = GetPropertyReply::default(),
                    5 => count.format = 8,
                    6 => return Err(DestinationError::Unverifiable("read failed".into())),
                    7 if reads == 2 => current = cardinal(6),
                    _ => {}
                }
                Ok((count, current))
            });
            let validated = fresh.and_then(|fresh| captured.revalidate(fresh));
            assert_eq!(validated.is_ok(), case == 8);
            assert_eq!(reads, if case == 6 { 1 } else { 2 });
            assert_eq!(host.marker, Some(original));
            assert_eq!(host.log, ["create reply", "marker write"]);
            assert!(host.sent.is_empty());
            assert_eq!(captured.frozen_destination(), 7);
            assert_eq!(
                placement_caller_marker(command, captured, owner(), ID).unwrap(),
                original
            );
        }
    }
}
