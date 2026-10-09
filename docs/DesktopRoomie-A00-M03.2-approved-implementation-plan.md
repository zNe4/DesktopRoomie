> **Design status (2026-10-09): APPROVED FOR STAGED IMPLEMENTATION PLANNING.** The independently reviewed revised Codex read-only proposal below is the approved technical design baseline for A00-M03.2. **This is not authorization to edit executable code, commit an implementation, perform host-acceptance on behalf of the owner, or begin M03.3.** Each implementation stage needs a separately approved, bounded assignment with explicit Git publication authority.
>
> **Provenance:** Codex prepared this plan against `m03` HEAD `b744272a8d3a60f6a88406c1197dd206550dbcbe`, with no repo changes during planning. ChatGPT independently reviewed both passes and approved the revised design; user authorized publication of this design in the documentation. Its recorded read-only baseline and original concluding 'approval still required' statements are **historical planning text**, superseded solely in status by this header. Technical assumptions remain subject to deterministic checks and the real Openbox acceptance gates in the plan.
>
> **Stage decomposition:** Section 16 is Codex's proposed ordered sequence, not the final allocation of bounded implementation stages. Stage contracts and checkpoints will be decided separately. Keep M03.2 Hide/Show/Bring Top independent from M03.3 Bring Here/combined Recover.

---

# DesktopRoomie A00-M03.2 — Revised read-only implementation plan

This replaces the previous proposal. M03.2 owns managed Hide, Show, independent same-instance control, and Bring Top on the body’s existing workspace.

## 1. Repository baseline and decision provenance

| Item | Verified result |
|---|---|
| Repository | /home/zzzne4/Desktop/Utils/ProjectVanilla |
| Branch | m03 |
| HEAD | b744272a8d3a60f6a88406c1197dd206550dbcbe |
| Working tree | Clean |
| Accepted executable revision | f8332ec99e876db1d809cd718dc76dff9fd38e27 |
| Repository changes in either planning pass | None |
| Branch creation, commits, pushes | None |

The original inspection covered the required documentation, relevant Rust source/tests, x11rb 0.14.0, and local X11 documentation. This revision rechecked the baseline, startup ordering, layer observation, and local ICCCM selection-acquisition rules.

The following choices were explicitly supplied through the owner’s answers to two clarification rounds earlier in this conversation:

1. Plain duplicate launch refuses.
2. Control commands do not start an instance when none exists.
3. Use a brief server-grab ownership transaction.
4. Preserve the accepted responsive-X-server limitation.

Those choices remain authoritative. The corrected timestamp mechanism, visibility predicates, and simplified protocol below are proposed implementation decisions subject to independent review.

No executable or Cargo checks were run for this revision.

## 2. Current-state map

The existing architecture remains the foundation:

- ProbeRuntime owns interaction, pointer obligations, movement/correction state, body availability, popup resources, and the layer controller.
- BodyAvailability describes interaction readiness; it does not distinguish minimization from off-workspace state.
- Pointer cancellation invalidates actions and movement before checked release. Failed release retains an obligation.
- The existing layer controller implements bounded absolute Above/Normal/Below transitions without rollback or reassertion loops.
- Layer orchestration currently prints several distinct outcomes while returning Ok(()); remote control requires structured outcomes.
- The current layer reader requires Viewable. A narrowly scoped off-workspace eligibility path is necessary.
- Lifecycle/layout interruptions currently cancel applicable interaction and abort layer operations.
- The loop processes bounded batches, checks buffered events after synchronous requests, and waits against the earliest active deadline.
- Startup currently permits waiting for MapNotify without a dedicated startup-map timeout. Preserve that behavior.
- Logical rejection of body input does not prevent X11 automatic capture. Preserve the proposed native input fence.

No second runtime framework, generic host abstraction, or broad main.rs refactor is proposed.

## 3. Architectural decisions

| Area | Revised decision |
|---|---|
| Instance scope | One cooperative owner per X server/screen, discovered through _DESKTOPROOMIE_INSTANCE_S{screen_num} |
| Endpoint | Dedicated never-mapped InputOnly control window |
| Acquisition | Genuine PropertyNotify timestamp, then brief guarded lookup/claim/verification |
| Timestamp rejection | One retry with a newly obtained timestamp, outside the server grab; then explicit failure |
| Existing owner | Never replace, clear, kill, or take over automatically |
| Startup delay | Acquire ownership and publish Starting before delaying visible appearance |
| Startup timeout | No new startup-map timeout |
| Transport | One request ClientMessage and one terminal response ClientMessage |
| Caller property | Immutable participation marker only |
| Concurrency | One admitted remote operation; no queue |
| Retransmission | None |
| Timeout cancellation | None; admitted work may finish after caller timeout |
| Visibility | Combined ICCCM, map-state, HIDDEN, and workspace evidence |
| Show | Ensure not minimized, preserving workspace and effective layer |
| Bring Top | Ensure not minimized, then reuse the existing Above operation |
| Off-workspace placement | No caller-workarea correction; defer input readiness |
| Host integration | CLI works independently; optional owner-managed Openbox bindings |
| Extra commands | No remote layer/status/Quit, Bring Here, or Recover |
| Taskbar/pager | Observe current behavior; no skip-state policy change |

All operation deadlines retain the accepted limitation: a responsive X server is assumed. Synchronous connection, reply, check, and flush calls do not acquire a new hard wall-clock timeout.

## 4. Single-instance lifecycle and timestamps

### Control-window construction

After parsing and connecting, prepare all acquisition resources before grabbing the server:

- intern control atoms;
- generate the control XID and a nonzero instance epoch;
- create a root-child InputOnly window, depth zero, border zero, size 1×1, with the screen’s root visual;
- select PropertyChange and StructureNotify;
- create its timestamp property and complete Starting descriptor;
- never map it, render into it, or select keyboard/button input.

Help requires no connection. Diagnose creates no owner. Control mode bypasses body, renderer, alpha-visual discovery, and layout subscriptions.

### Obtaining a genuine server timestamp

Add private atom _DESKTOPROOMIE_TIMESTAMP.

1. Initialize that property on the control window as CARDINAL/32.
2. Issue a checked zero-length append to it.
3. Record that ChangeProperty request’s sequence.
4. Drain events and wait through socket poll for the corresponding native PropertyNotify:
   - matching control window and timestamp atom;
   - NewValue state;
   - matching request sequence;
   - not synthetic;
   - nonzero event timestamp.
5. Use the event timestamp directly. Do not derive a timestamp from a local clock.

Ignore older timestamp-property notifications. Handle connection/error/control-destruction events normally.

Use a one-second maximum wait for each timestamp probe, capped by a two-second overall acquisition-attempt budget. These limits bound startup arbitration, not body mapping. They retain the responsive-server limitation.

A zero timestamp cannot be passed through because it denotes CurrentTime on the request wire. Treat it as an unusable sample and continue only within the remaining probe budget.

### Guarded transaction

With a fresh timestamp available:

GrabServer
→ GetSelectionOwner
→ if owner is None:
     SetSelectionOwner(control, selection, real_timestamp)
     GetSelectionOwner to verify control owns it
→ always UngrabServer

Check protocol errors and ownership separately. Where practical, use the required replies to establish processing order without unnecessary extra round trips.

The grabbed section contains only ownership arbitration and its checks. No timestamp acquisition, sleeps, WM-dependent requests, body creation, rendering, logging work, or unrelated initialization belongs inside it.

Always attempt checked UngrabServer, including on lookup, claim, or verification failure. Failure to confirm server release prevents all further startup work and leads to connection teardown, preserving the original error.

### Stale timestamp despite an unowned selection

A recent owner can acquire and relinquish the selection after timestamp sampling. The selection can therefore be unowned while retaining a last-change time newer than the sample. SetSelectionOwner may be processed without transferring ownership.

Policy:

- Ownership verification catches this; checked request processing is insufficient.
- If verification returns None without a protocol error, release the server and obtain a new timestamp.
- Permit one complete retry within the overall acquisition budget.
- If an owner is observed on either attempt, refuse duplicate startup.
- If the second claim remains unconfirmed, fail explicitly.
- Never fall back to CurrentTime.

This accommodates a normal short ownership race without unbounded contention or takeover behavior. Store the successfully used acquisition timestamp locally for diagnostics/lifecycle interpretation.

### Delay and initialization

After acquisition:

1. Keep the descriptor at Starting.
2. Apply --delay before body initialization/mapping.
3. During the delay, use a small event-driven startup wait that handles control-window loss and returns Starting to any directly received valid control request.
4. Initialize and map the body through the existing path.
5. Publish the body XID while still Starting.
6. Publish Ready after existing map/placement/input validation.

Clients reading a Starting descriptor may immediately return Starting without sending a command.

This preserves the focus-testing purpose of --delay: visible appearance is delayed. Creating an unmapped InputOnly window and owning a selection does not request focus or display a body.

Remove the proposed one-second initial map/placement timeout. Ownership correctness does not require it. Starting can remain pending; clients have bounded results, duplicates cannot create another body, and existing duration/shutdown behavior remains available. Preserve the existing duration-start semantics.

### Cleanup and loss

- Publish Closing before rejecting further operations and beginning shutdown.
- Best-effort terminate any active remote operation with its partial observations.
- Abort layer work, cancel interaction, and attempt pointer/popup/renderer/body cleanup.
- Destroy the control window last using explicit resource ownership.
- Do not call SetSelectionOwner(None): destruction clears this window’s ownership without risking a replacement owner’s selection.
- On actual selection loss or control destruction, stop accepting work and shut down; never reacquire automatically.
- Re-query ownership for a purported SelectionClear before treating a fabricated/stale notification as real loss.
- Connection closure/destruction clears selection ownership through X itself. Unacknowledged cleanup remains unacknowledged.

## 5. Minimal wire protocol

### Resources and atoms

Private atoms:

- _DESKTOPROOMIE_INSTANCE_S{screen_num} — selection.
- _DESKTOPROOMIE_INSTANCE — owner descriptor.
- _DESKTOPROOMIE_TIMESTAMP — acquisition timestamp probe.
- _DESKTOPROOMIE_REQUEST — request message.
- _DESKTOPROOMIE_REPLY — terminal response.
- _DESKTOPROOMIE_CALLER — immutable caller participation marker.

Version: 1. Magic: 0x44524D31.

Generate a nonzero 32-bit owner epoch and nonzero 64-bit request ID using /dev/urandom through the standard library. No new dependency or authentication claim.

Owner descriptor, CARDINAL/32, exactly seven words:

[magic, version, screen_num, control_xid, epoch, lifecycle, body_xid]

Lifecycle: Starting 1, Ready 2, Closing 3.

Each command invocation creates one never-mapped InputOnly reply window on the same root. Its immutable marker is CARDINAL/32, exactly seven words:

[magic, version, control_xid, epoch,
 request_id_low, request_id_high, command]

The owner never changes this property. There are no tickets, Accepted/Final states, persisted results, or replay caches.

### Request

Commands: Hide 1, Show 2, BringTop 3.

SendEvent(false, control_xid, NO_EVENT,
  ClientMessage(
    window = control_xid,
    type = _DESKTOPROOMIE_REQUEST,
    format = 32,
    data = [
      (version << 16) | command,
      request_id_low,
      request_id_high,
      reply_xid,
      epoch
    ]))

A caller sends at most one request. Missing, Starting, Closing, or incompatible owners can be reported during discovery without sending one.

### Terminal response

SendEvent(false, reply_xid, NO_EVENT,
  ClientMessage(
    window = reply_xid,
    type = _DESKTOPROOMIE_REPLY,
    format = 32,
    data = [
      (version << 16) | status,
      request_id_low,
      request_id_high,
      control_xid,
      detail
    ]))

Status values:

| Value | Terminal meaning |
|---|---|
| 0 | Success |
| 1 | Failed |
| 2 | Partial |
| 3 | Busy |
| 4 | Starting |
| 5 | Closing |
| 6 | ProtocolError |

The five data words fit the ClientMessage’s 20-byte payload.

detail retains compact structured diagnostics:

| Bits | Meaning |
|---|---|
| 0–7 | Reason |
| 8–10 | Visibility classification |
| 11–13 | Layer |
| 14–15 | Workspace relation |
| 16 | Not-minimized state confirmed |
| 17 | Above confirmed |
| 18 | Placement validation deferred |
| 19 | A host mutation was dispatched |
| 20 | Local geometry/input readiness confirmed |
| 21–23 | Stage |
| 24–31 | Reserved, zero |

Reason values:

0 None                 1 MalformedRequest       2 WrongVersion
3 UnknownCommand       4 Starting               5 Busy
6 Closing              7 OwnershipLost          8 BodyDestroyed
9 Withdrawn           10 Unverifiable          11 Unsupported
12 ConfirmationTimeout
13 LayoutChanged      14 LayerChanged          15 ReleaseFailed
16 X11Failure         17 StaleInstance

Visibility:

0 Unknown
1 Minimized
2 NotMinimizedHere
3 NotMinimizedElsewhere
4 Transitional
5 Withdrawn
6 Destroyed

Layer: unknown 0, Normal 1, Above 2, Below 3, Conflict 4.

Workspace relation: unknown 0, current 1, other 2, all-desktops 3.

Stage: preflight 0, Hide 1, Show 2, geometry 3, layer 4, shutdown 5.

### Validation and admission

Validate exact message destination, atom, format, version, command, request ID, epoch, and current ownership.

Validate the reply window:

- exists under the expected root;
- is InputOnly and unmapped;
- is not root, owner control, body, or popup;
- carries the exact marker type/length and matching identity.

Wrong-version requests receive WrongVersion only when a safely interpretable v1 marker supplies a validated reply route. Unknown commands receive UnknownCommand. Otherwise malformed/unrelated messages are ignored.

One active remote record contains the request identity, command, phase, deadline, and observed progress. A competing request receives Busy without affecting existing work.

An exact duplicate of the active request is ignored; the original will produce one terminal reply. No completed-request history is retained. Replays after completion are outside the conforming one-request/no-retransmission protocol and do not receive an exactly-once guarantee. A stale request normally fails because its reply window has disappeared or its marker/epoch no longer matches.

### Deadlines and disappearance

- Caller deadline: four seconds from beginning discovery after connection establishment.
- Discovery, validation, sending, and response waiting share that budget.
- There is no acceptance acknowledgement or separate acceptance timeout.
- Hide/Show owner deadline: admission plus one second.
- Bring Top total deadline: admission plus two seconds; visibility uses at most the first second, and layer work retains its one-second maximum capped by the total deadline.
- Existing 150 ms geometry confirmation remains; do not dispatch correction without its full confirmation interval remaining.
- No event, phase notification, or duplicate extends a deadline.
- At expiry, make a final fresh observation; dispatch no new mutation.

The caller validates terminal replies against version, reply window, request ID, and discovered control XID. It ignores unrelated/stale replies and exits after the first valid terminal response.

Watch owner destruction and drain buffered replies before declaring owner loss. A validated final response already received remains a valid observed result even if the owner then exits.

If the caller disappears before admission, do not dispatch. After admission, caller disappearance or timeout does not cancel or roll back work. Complete within the owner deadline and attempt one terminal reply. BadWindow on a peer reply is nonfatal to the owner.

If the receiver dies or the reply is lost, the caller reports outcome unknown, not “operation did not happen.” No automatic retry.

### Acknowledgement and trust

Internally distinguish receipt, admission, host dispatch, X processing acknowledgement, fresh WM observation, and terminal result. Only the last two can establish success. Intermediate stages are owner diagnostics/state, not additional wire messages.

Same-session X clients can forge messages, alter markers, steal selections, or interfere with windows. Validation provides routing and accidental-staleness protection, not authentication.

## 6. Visibility model and Hide

### Required observation

Read and strictly decode:

- ICCCM WM_STATE;
- window map attributes;
- complete _NET_WM_STATE, including HIDDEN and layer flags;
- body _NET_WM_DESKTOP;
- root _NET_CURRENT_DESKTOP;
- relevant advertised EWMH support.

Do not interpret missing/malformed _NET_WM_STATE as absence of HIDDEN. For visibility control, require advertised support for _NET_WM_STATE_HIDDEN and the workspace properties used in classification. Unsupported evidence produces Unsupported/Unverifiable, not a guessed state.

Keep the diagnostic fallback to desktop zero out of control classification.

WM_STATE must be WM_STATE/32 with exactly two words. CARDINAL properties require exact payload consistency. All reads are observations, not an atomic snapshot; repeat relevant reads on notifications and before final success. Unexpected workspace changes invalidate the operation.

### Classification

| Combined evidence | Interpretation |
|---|---|
| Iconic + Unmapped + HIDDEN | Actual minimized candidate; confirm through a fresh coherent snapshot |
| Iconic + Unmapped + other workspace + no HIDDEN | Managed off-workspace, not minimized |
| Normal + non-viewable + other workspace + no HIDDEN | Possible managed off-workspace, not minimized; eligible when workspace/property evidence is coherent |
| Normal + Viewable + current/all-desktops + no HIDDEN | Not minimized here; placement readiness remains separate |
| Iconic + Unmapped without readable HIDDEN/workspace evidence | Unverifiable; never sufficient for Hide success |
| Iconic + Unmapped + current workspace + no HIDDEN | Transitional/incoherent |
| Normal + non-viewable + current workspace | Transitional/unexplained unavailability |
| HIDDEN with contradictory WM/map evidence | Transitional/incoherent |
| Absent/Withdrawn WM_STATE | Withdrawn/unmanaged, not successful Hide or Show |
| Body destruction/BadWindow | Destroyed |
| Other malformed or contradictory combinations | Unverifiable |

A window merely being covered by other windows remains Viewable; this protocol does not measure occlusion.

### Hide operation

Validate → reserve → input fence and checked cancellation
→ fresh classification → request actual minimization if needed
→ observe → terminal result

Exact request:

SendEvent(
  false,
  body_root,
  SUBSTRUCTURE_NOTIFY | SUBSTRUCTURE_REDIRECT,
  ClientMessage(
    window = body,
    type = WM_CHANGE_STATE,
    format = 32,
    data = [IconicState(3), 0, 0, 0, 0]
  )
)

Use checked x11rb transport. There is no EWMH source field in this ICCCM message.

- Already coherently minimized: idempotent after settling input obligations.
- Off-workspace without HIDDEN: send the minimization request, even if WM_STATE is already Iconic.
- Success requires fresh Iconic + Unmapped + HIDDEN evidence.
- If Openbox ignores the off-workspace request or cannot distinguish minimization, timeout and stop for host/architecture review. Do not substitute direct HIDDEN writes or workspace movement.
- Never use withdrawal or application UnmapWindow as Hide.
- On failure, restore body input only after coherent local visibility and placement validation; otherwise keep it unavailable.
- No taskbar/pager state mutations accompany Hide.

## 7. Show

Show ensures that the body is not minimized, rather than requiring NormalState everywhere.

### Behavior

- Coherently minimized: keep input disabled, ensure _NET_WM_USER_TIME=0, then issue one checked MapWindow.
- Off-workspace and not minimized: do not map or deiconify it. Return qualified success after preservation checks.
- Locally not minimized and Ready: fresh verification, then no-op.
- Incoherent/current-workspace unavailability: observe within the deadline; do not infer minimization.
- Withdrawn: fail without re-managing.
- Destroyed: fail and follow owner shutdown; never create a replacement body.

No MapRaised, activation, focus, keyboard grab, or stacking request.

### Confirmation and preservation

After actual restoration:

- Current/all-desktops: require no HIDDEN, Normal + Viewable, and validated placement.
- Other workspace: allow either coherent off-workspace representation above, including Iconic + Unmapped + no HIDDEN. Do not require a transition to Normal merely for success.

For Show, snapshot the fresh effective layer and body workspace. Require them to remain unchanged at completion. Reject a contradictory/unverifiable layer baseline.

Do not reassert LayerController.desired, which can retain a failed earlier target. Layer drift produces LayerChanged; workspace drift produces LayoutChanged. No compensating workspace or layer writes.

### Placement

For local restoration, reuse current monitor/workarea refresh, fixed-size/root-coordinate validation, and one-shot correction. Confirm correction before restoring the interactive silhouette.

For another workspace:

- do not use the caller’s workarea for placement;
- do not move the body;
- keep input unavailable;
- report not minimized on the existing workspace, not present here, placement deferred;
- validate placement when later lifecycle/workspace evidence makes the body locally viewable.

Selected-monitor removal and required-correction failure retain existing fatal geometry handling. No destination reconciliation algorithm is added.

## 8. Bring Top

Bring Top composes:

1. Ensure not minimized using the shared visibility machinery.
2. Request/confirm Above using the existing M03.1 controller.

The visibility stage preserves workspace but does not enforce Show’s unchanged-layer predicate, because Above is the explicit target.

| Case | Result |
|---|---|
| Locally visible Normal/Below | Existing Above transition |
| Locally visible Above | Fresh confirmation, no mutation |
| Minimized Above | Map, confirm not minimized, re-observe Above; add only if necessary |
| Minimized Normal/Below | Restore, then Above |
| Off-workspace Iconic/Unmapped without HIDDEN | Do not map; operate through the existing layer controller there |
| Off-workspace Normal/non-viewable without HIDDEN | Same |
| Restoration fails | Do not begin layer mutation |
| Actual restoration succeeds, Above fails | Partial, with observed visibility/layer and reason |
| No restoration was needed, Above fails | Failed, with actual layer state |
| Removal succeeds and addition fails | Report the partial observed layer, without rollback |
| Existing layer operation pending | Busy before remote side effects |

The added off-workspace layer reader must accept only freshly verified managed, not-minimized, other-workspace state. It may include WM_STATE Iconic. Retain Viewable requirements for ordinary local-menu operations.

Each remote layer observation revalidates eligibility; an initial snapshot is not a permanent permission to mutate.

Success requires exact Above without Below plus a fresh compatible not-minimized/workspace observation. Report other-workspace success explicitly. It does not promise current-workspace visibility, focus, or precedence over fullscreen.

Workspace reads are necessary for classification, preservation, truthful diagnostics, and avoiding incorrect workarea use. Neither workspace property is written.

## 9. Input, lifecycle, and event-loop integration

Preserve the original proposal’s native input fence:

1. Checked empty SHAPE input region.
2. Invalidate gesture/menu actions and discard coalesced/final movement.
3. Checked pointer release, including possible queued automatic capture.
4. Attempt popup destruction even after release failure.
5. Dispatch visibility changes only after all obligations settle.

Represent transition-fence release debt explicitly when no ordinary owner is tracked. Restore the exact existing silhouette only after readiness verification.

Use the enabling request sequence to reject older body pointer events, including releases. This is an input freshness boundary, never a WM-confirmation gate.

| Situation | Policy |
|---|---|
| Hide during BodyLeft/OpeningRight/Menu | Neutral cancellation and checked release before dispatch |
| Show/Bring Top already not minimized | Preserve valid interaction unless restoration is actually required |
| Release/cleanup uncertainty | No visibility dispatch; retain debt and preserve fatal-error priority |
| Existing local layer operation | Remote commands return Busy and preserve it |
| Remote visibility operation active | Local layer selection returns Busy after menu cleanup |
| Bring Top layer stage | Later valid drag/menu ownership does not stall layer progression |
| Pending bounds correction | Hide invalidates local completion before hiding; Show/Bring Top return Busy |
| Quit/duration/shutdown | End remote/layer work and run existing ordered cleanup |
| External unmap/iconification | Cancel interaction; classify fresh state; abort incompatible pending work |
| Body/control destruction or connection loss | Fail active operation and follow explicit cleanup obligations |

Expected visibility events trigger observation; they do not blindly abort their own visibility stage. Unexpected layout/workspace changes abort remote work without retries or compensation.

Event-loop changes remain narrow:

- route control/selection events before body/popup handling and movement flush;
- tighten WM_DELETE_WINDOW matching to body + WM_PROTOCOLS + format 32;
- recognize prospective Hide in movement lookahead;
- add only active acquisition/delay/control/visibility deadlines to existing socket waiting;
- check operation deadlines between bounded batches and before mutations;
- drain x11rb-buffered events after synchronous requests;
- allow Unavailable bodies to be revalidated on relevant workspace/property events without requiring MapNotify;
- late WM events update reality and placement safety but never resurrect a terminal command.

No permanent polling or owner-side response-wait timer is added. The owner sends its terminal reply once.

## 10. CLI and result semantics

Pure parser returns:

Help
Diagnose
Owner { delay, duration }
Control { Hide | Show | BringTop }

Keep parsing in main.rs; add no parser dependency.

Control modes are mutually exclusive and incompatible with diagnose/delay/duration. Preserve existing valid owner syntax.

| Exit | Meaning |
|---|---|
| 0 | Confirmed success, help/diagnostics, or normal owner completion |
| 1 | Owner startup/runtime or local connection/resource failure |
| 2 | CLI usage error |
| 3 | No running instance |
| 4 | Duplicate plain launch |
| 5 | Busy, Starting, or Closing |
| 6 | Incompatible/malformed peer or protocol |
| 7 | Operation failed/unconfirmed |
| 8 | Caller timeout or owner disappearance before final response |
| 9 | Confirmed partial result |

Example diagnostics:

hide: confirmed minimized; existing instance remains running

show: already not minimized on another workspace;
      no map request sent; placement validation deferred

bring-top: confirmed not minimized + Above on existing workspace;
           not on current workspace

bring-top: partial — restored, but Above confirmation timed out

show: timeout — final outcome unknown; admitted work may still complete

Off-workspace success exits zero because it fulfills the approved contract. It must always include the location qualifier.

Commands require no terminal, interactive prompt, or optional Openbox configuration.

## 11. File-by-file implementation plan

Source paths are relative to experiments/desktop-probe/src/.

| File | Responsibility and tests |
|---|---|
| New control.rs | Pure command/result types, exact message/marker codecs, correlation, combined visibility classification, phases/deadlines. No mutable ticket/replay state. Own protocol and pure classification/composition tests. |
| New x11/control.rs | Atoms, InputOnly resources, timestamp probe, guarded acquisition, descriptor/marker validation, direct request/reply transport, caller wait and cleanup. Own injected acquisition/lifecycle/peer-failure tests. |
| New x11/visibility.rs | Strict WM_STATE/map/HIDDEN/workspace observation, capability validation, iconify request and non-activating MapWindow. Own decoding and request-envelope tests. |
| main.rs | Pure CLI parser, roles, acquired Starting delay, startup cleanup scope, structured layer results, remote operation orchestration, input fence and event/deadline routing. Extend existing closure-based tests. |
| layer.rs | Bounded entry accepting an outer deadline; retain existing default one-second behavior and transition algorithm. Test deadline capping. |
| x11/state.rs | Shared strict EWMH decoding and verified not-minimized off-workspace observation path, including Iconic/no-HIDDEN cases. Preserve local viewability and transport rules. |
| x11/window.rs | Store WM_PROTOCOLS for strict deletion routing; initialize body input disabled until placement validation; update fixtures. No startup-map timer. |
| x11/pointer.rs | Narrow transition-fence release obligation, unconditional checked cancellation path, and failure/retry tests. |
| x11/shape.rs | Checked empty input-region helper; restore existing silhouette; test exact requests and equivalence. |
| x11/monitors.rs | Strict CARDINAL validation and retained-monitor validation reuse. No destination-workarea algorithm or monitor migration. |
| x11/mod.rs | Declare new X11 modules. |

No planned production changes in interaction, geometry, menu, renderer, or visual selection beyond reuse of their established interfaces.

Documentation scope remains:

- new M03.2-IMPLEMENTATION.md;
- new ACCEPTANCE-M03.2.md;
- current implementation/checkpoint updates in STATUS, ARCHITECTURE, INDEX, and the Openbox guide.

Preserve historical acceptance records. No dependency, lockfile, CI, host-configuration, or mission-authority changes.

## 12. Deterministic test matrix

Use production helpers with small fake clocks, event queues, property replies, and host callbacks. No live Openbox dependency.

| Group | Required cases |
|---|---|
| Timestamp probe | Correct native PropertyNotify; old/wrong-window/wrong-atom/synthetic notification ignored; sequence mismatch; zero timestamp; buffered notification; probe timeout; connection failure |
| Timestamp ownership | Real timestamp reaches SetSelectionOwner; CurrentTime never used; stale timestamp with unowned selection fails verification; fresh retry succeeds; second failure stops; retry obtains timestamp only after ungrab |
| Guarded acquisition | No owner; valid owner; two cooperative launch interleavings; owner appears on retry; protocol errors; ungrab attempted on every path; release failure blocks initialization |
| Starting/delay | Ownership acquired before delay; descriptor Starting; duplicate refuses during delay; control returns Starting; delay does not map body; event traffic cannot extend delay; selection loss during delay cleans up |
| Startup preservation | No new map timeout; Starting remains pending while map is pending; existing duration semantics unchanged; startup failures release resources |
| Discovery | Ready/Starting/Closing; wrong version/root/screen/class; mapped control window; malformed descriptor; stale/destroyed XID |
| Ownership loss | Real versus fabricated SelectionClear; control destruction; crash/connection loss; cleanup cannot clear replacement ownership |
| Protocol | Exact request/reply layouts; immutable marker; wrong version/command/format/window/epoch; malformed marker; reserved bits; no Accepted messages or owner marker writes |
| Correlation | Unique caller IDs/windows; mismatched/unknown request ID; stale/duplicate reply; unrelated messages; first valid terminal response ends client |
| Duplicates/concurrency | Active exact duplicate ignored; different caller gets Busy; existing local layer remains unchanged; no queue, replay cache, automatic resend, or exactly-once claim |
| Peer death | Caller gone before admission; caller gone after admission; owner dies before/after send; terminal reply buffered before destruction; BadWindow on reply is nonfatal |
| Deadlines | Single caller deadline; owner deadlines capped; no extension; final fresh observation; no mutation at expiry; owner may complete after caller timeout |
| Visibility classification | All table combinations, especially Iconic/Unmapped/HIDDEN versus Iconic/Unmapped/other/no-HIDDEN; Normal/non-viewable/other; current-workspace contradictions |
| Evidence validity | Missing HIDDEN support; absent/malformed EWMH state; malformed WM_STATE; unreadable workspace; no desktop-zero fallback; inconsistent snapshots never succeed |
| Hide | Local not-minimized→minimized; already minimized; off-workspace Iconic/no-HIDDEN must send; Normal/off-workspace must send; no success until HIDDEN confirmed |
| Hide failures | Off-workspace iconification ignored; HIDDEN never appears; map/WM/HIDDEN disagreement; send failure; timeout; external unmap/destroy |
| Hide input | Drag, OpeningRight, Menu and queued automatic capture released first; failed release blocks send; popup cleanup still attempted; batch-boundary motion discarded |
| Show | Actual minimized restoration; local no-op; off-workspace Iconic/no-HIDDEN and Normal/non-viewable/no-HIDDEN send no MapWindow |
| Show completion | Off-workspace restoration may finish Iconic/no-HIDDEN; local restoration requires Normal/Viewable; layer/workspace preserved; withdrawn body not re-managed |
| Show geometry | Workarea changed while hidden; one correction; confirmation before input; insufficient remaining budget; correction refusal; retained monitor removed; no caller-workarea correction elsewhere |
| Bring Top | All layer combinations; actual hidden restoration; off-workspace non-minimized cases layer-only; restoration failure blocks Above; partial restoration/layer result |
| Layer regression | Existing controller reused; opposite removal; contradictory flags; support changes; deadline/read ordering; no rollback; later pointer ownership independent |
| Input fence | Empty region before release; explicit debt; failure retention; stale body input ignored; exact silhouette restored |
| Lifecycle | Quit/duration at each phase; shutdown reply attempt; body/control loss; cleanup order; original-error priority |
| Event loop | More than 64 buffered events; deadline fairness; buffered reads before wait; no idle control deadline or polling; late WM events do not restart commands |
| CLI/non-regression | Mode conflicts, exits, duplicate/no-instance behavior; no focus/activation/keyboard/global listener; no workspace writes; no duplicate body; existing click-through/menu/drag behavior |

After implementation, run the project-required build, format-check, Clippy, tests, and diff review using the verification skill. Record actual results separately from historical M03.1 evidence.

## 13. Owner real-host acceptance matrix

Record executable SHA, dirty status, exact commands, host versions/rules, workspace/monitor/workarea facts, and actual typing observations.

| ID | Procedure and required observation |
|---|---|
| H01 | Launch while typing: body appears without focus transfer. |
| H02 | Delayed owner plus plain duplicate: one Starting owner, no second body, appearance remains delayed. |
| H03 | Control during delay/initialization: bounded Starting result, no presentation change. |
| H04 | Hide while typing: actual minimization, unchanged focus. |
| H05 | Show from Above/Normal/Below: same body, workspace and layer, no activation. |
| H06 | At least 20 hide/show cycles: no stale input, duplicate, or orphan resource. |
| H07 | Hide with popup open/pressed row: no leaked action or retained capture. |
| H08 | Hide during drag and OpeningRight, safely using delayed invocation: no final movement/click/menu resurrection. |
| H09 | Fully cover the body: CLI remains independently usable. |
| H10 | Bring Top from Normal and Below: Above confirmed without activation. |
| H11 | Bring Top while actually minimized in each layer: restoration then Above. |
| H12 | Other-workspace, not-minimized body: Show sends no restoration and does not move it. Record WM_STATE/map/HIDDEN evidence. |
| H13 | Other-workspace Iconic/no-HIDDEN representation, if used by Openbox: Bring Top operates there without MapWindow or relocation. |
| H14 | Other-workspace Normal/non-viewable representation, if used: same result. Do not fabricate an unobserved representation as host evidence. |
| H15 | Hide a not-minimized body on another workspace: require actual HIDDEN transition, not merely continued Iconic/unmapped state. |
| H16 | Show/Bring Top an actually minimized body on another workspace: HIDDEN clears; workspace stays unchanged; result says elsewhere. |
| H17 | Verify off-workspace commands never switch the user’s desktop or apply caller-workarea displacement. |
| H18 | No owner: all commands fail without creating a body. |
| H19 | Kill owner, allow X disconnect processing, then control/relaunch: no stale selection blocks startup. |
| H20 | Rapid concurrent commands: explicit busy or terminal result; no duplicate body, queue, or orphan capture. |
| H21 | Quit/duration around remote work: bounded client outcome and normal input recovery. |
| H22 | At least 20 mixed local-menu/layer/drag and remote operations: no accumulated failure. |
| H23 | Workarea changes while minimized: Show validates and confirms correction before input returns. |
| H24 | Recheck transparent padding, translucent patch, offset drag, right cancellation and outside-menu dismissal. |
| H25 | Recheck Above/fullscreen coverage and return: no focus or restack workaround. |
| H26 | Visible and minimized idle CPU: event-driven behavior. |
| H27 | Record taskbar/pager representation; make no skip-state policy changes. |
| H28 | After CLI acceptance only, optional owner-managed Openbox Execute smoke test while typing. |

Timestamp rejection, selection races, malformed states and exact failure timing remain deterministic evidence unless actually reproduced on the host.

## 14. Failure model and stop conditions

Recoverable command failures include Busy, unsupported capability, coherent WM refusal/timeouts, and unverifiable state. Owned-resource/transport failures retain existing fatal cleanup rules. Peer reply-window disappearance is nonfatal.

Stop for architecture review if:

1. Cooperative ownership cannot be established without takeover or an unsafe race.
2. Timestamp acquisition requires CurrentTime fallback or unbounded retries.
3. Server-grab work expands beyond brief ownership arbitration or release cannot be safely attempted.
4. Openbox cannot distinguish actual minimization from inactive-workspace state using the required evidence.
5. Off-workspace Hide cannot cause and confirm HIDDEN through the approved ICCCM path.
6. Restoration needs focus, activation, workspace movement, or re-management.
7. Off-workspace Above requires weakening managed/not-minimized verification.
8. Hide can dispatch with unresolved pointer debt or stale input can recreate capture.
9. Correct placement requires using the caller’s workarea for a different workspace.
10. Correctness requires workspace property writes, polling, retries, restack fighting, or a broader daemon/framework.
11. Show cannot preserve layer/workspace under actual Openbox behavior.
12. Required trust/authentication or hard transport deadlines exceed the approved A00 model.
13. Selection loss or cleanup breaks original-error priority or resource ownership.
14. Any material target-host behavior contradicts the approved predicates.

Timeout means confirmation was not obtained. It never proves that an issued WM request had no effect.

## 15. M03.3 boundary

No implementation or internal algorithm for:

- Bring Here, workspace relocation, M03.3 workspace reconciliation;
- combined Recover;
- arbitrary window-relative stacking;
- global hotkeys, tray framework, or configuration auto-editing;
- A01 behavior/persistence;
- Brain/Spine/memory;
- generic cross-platform IPC;
- daemonization or broad refactoring.

Future commands require separate approval. This protocol reserves no workspace-moving behavior.

## 16. Recommended implementation sequence

1. Reverify baseline; add pure CLI, protocol/result types, and visibility predicates.
2. Implement timestamp probe, guarded acquisition/retry, explicit resources, and Starting-before-delay behavior.
3. Implement minimal caller/terminal-response transport and correlation.
4. Implement strict visibility observation and native request helpers.
5. Implement input fencing and explicit release-debt tests.
6. Integrate Hide, including off-workspace actual-minimization confirmation.
7. Integrate Show, including no-map off-workspace behavior and local placement validation.
8. Add structured layer outcomes and compose Bring Top using the existing controller.
9. Complete lifecycle, deadline, concurrency, and non-regression tests.
10. Run required checks and update implementation/evidence documentation.
11. Obtain independent exact-revision review and owner host acceptance. Publication requires separate authorization.

## 17. Revision summary and readiness

Changes from the previous proposal

- Replaced CurrentTime acquisition with a genuine PropertyNotify timestamp.
- Added one bounded fresh-timestamp retry after ungrab when ownership verification fails.
- Distinguished minimization from inactive-workspace Iconic/unmapped state using HIDDEN and workspace evidence.
- Corrected Hide, Show, and Bring Top accordingly.
- Removed mutable caller tickets, Accepted acknowledgements, persisted final results, and replay recovery.
- Defined one request, one terminal reply, and one caller deadline.
- Moved ownership acquisition before --delay.
- Removed the new startup-map timeout.
- Updated module responsibilities, tests, host acceptance, and stop conditions.
- Retained owner choices with their actual clarification-response provenance.

Remaining mandatory architectural decisions

No user-facing choice or protocol mechanism remains unspecified. Independent ChatGPT approval of the revised design is still required, particularly the visibility predicates, timestamp transaction, and retained input fence. Host feasibility is an acceptance gate, not an assumed guarantee.

New or sharpened stop conditions

The principal additions are inability to obtain a usable real timestamp without fallback/unbounded retry, inability to distinguish off-workspace state from minimization, and inability to minimize an off-workspace body through WM_CHANGE_STATE while confirming HIDDEN.

Implementation readiness

I consider this decision-complete for an implementation session after independent review approves it. It is not implementation authorization. Openbox behavior remains subject to the explicit acceptance tests and stop conditions.

The repository remains unchanged. No implementation, branch, commit, or push was performed.
