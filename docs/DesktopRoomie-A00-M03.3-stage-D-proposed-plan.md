# DesktopRoomie — A00-M03.3 Stage D proposed implementation plan

**Historical D-P status: PROPOSED — not then owner-approved, implemented, or accepted.**

**Lifecycle update — 2026-10-20:** Stage D was subsequently implemented and owner-accepted at final D4.1 executable `4c67da0b09ffe7b69605ac4a415d256acf8abd23`, [CI 39377294762](https://github.com/zNe4/DesktopRoomie/actions/runs/39377294762), **677 passing tests**, after a D4 source-review blocker required correction. [Stage-D acceptance](../experiments/desktop-probe/M03.3-D-ACCEPTANCE.md) is the current evidence/decision; original future-tense plan requirements below are preserved historically. Recover remains Unsupported; Stage E is separately gated.

**Planning assignment:** A00-M03.3 D-P, 2026-10-18.

**Repository / required branch:** `zNe4/DesktopRoomie` / `m03`.

**Verified starting local and remote HEAD:** `817dd48de5794d4b225f17b8ce2c90efb8cfca13`.

**Scope:** documentation for standalone Bring Here only. Publication of this proposal is authorized by D-P; implementing any checkpoint requires a separate approved assignment.

## 1. Authority, dependencies, and intended result

The governing contract is [the approved M03.3 roadmap, especially §9](DesktopRoomie-A00-M03.3-staged-implementation-roadmap.md#9-stage-d--standalone-bring-here), read with [WORKFLOW](WORKFLOW.md), [STATUS](STATUS.md), [ARCHITECTURE](ARCHITECTURE.md), [INVARIANTS](INVARIANTS.md), and [AGENTS](../AGENTS.md). Older implementation notes describe their own checkpoints; their historical next-stage language does not override STATUS or authorize execution.

Accepted dependencies inspected for this proposal:

| Dependency | Executable / evidence | What Stage D may rely on |
| --- | --- | --- |
| M03.2 final | `a46e432c1f97b03c576737b1fc0ac35d7f90c158`, CI 39098551853, 453 tests | Independent control lifecycle; accepted Hide, Show, Bring Top; input fence, pointer release, fixed geometry, exact SHAPE and deferred local readiness. |
| M03.3 A | `d7727b3abf9e2c047e389f13a9c5cac19f9ed1a8`, CI 39152686625, 488 tests; [record](../experiments/desktop-probe/M03.3-A-IMPLEMENTATION.md) | V2 immutable marker and destination identity; one ActiveOperation; placement baseline/current/history proof and strict result partition; fixed deadlines. |
| M03.3 B | `4e2bd4dbfce786cc2b269bd2af0693881324cc93`, CI 39155316992, 537 tests; [record](../experiments/desktop-probe/M03.3-B-IMPLEMENTATION.md) | Strict destination layout, two complete agreeing reads, retained monitor and fit, workarea/viewport rules. |
| M03.3 H0 | Owner-native Openbox 3.6.1 / X11 / Picom v13 on Stage B; [record](../experiments/desktop-probe/M03.3-H0-OPENBOX.md) | One-shot source-1 relocation feasibility, same XID, layer/minimization/focus/workspace observations. Observed event order is not a required order. |
| M03.3 C | `58a1ffa884c2527797d96023570542848407bcbe`, [CI 39159965288](https://github.com/zNe4/DesktopRoomie/actions/runs/39159965288), 542 tests; [record](../experiments/desktop-probe/M03.3-C-IMPLEMENTATION.md) | Checked, consuming DesktopRequest; possible-dispatch accounting; no native runtime consumer. Independently reviewed and owner-accepted. |

The starting HEAD has no difference from Stage C in `experiments/desktop-probe/src`, Cargo manifest/lockfile, or `.github/workflows`. This is a source comparison, not a fresh test run. Initial branch and remote HEAD matched and the working tree was clean.

Bring Here moves the **existing managed body** to the invoking caller's frozen current desktop. It preserves genuine minimization, the effective Normal/Above/Below layer, retained monitor identity, user desktop, keyboard focus and activation. It does not imply peer z-order, Show, Above, or a newly created body. Bring Top keeps its independent existing-workspace semantics. Recover remains Unsupported; no Here→Top composition is implemented in D.

## 2. Inspected source ownership and integration map

Paths in this section are relative to `experiments/desktop-probe/`.

| Source and actual boundary | Existing behavior / hazard | Proposed minimal integration |
| --- | --- | --- |
| `src/control.rs`: `ActiveOperation`, `PlacementProgress`, `PlacementBaseline`, `OperationResult::placement`, `admit` | ActiveOperation is Copy and owns one immutable deadline. PlacementProgress currently contains only destination, optional baseline, and proof; it has no runnable Here phase. `clear_current()` preserves history. | Add small pure Here phase/evidence/attempt fields under PlacementProgress, with a value-only layout identity. Keep admission, wire numbers, marker shape, result validator and deadlines unchanged. |
| `src/x11/control.rs`: `caller_with`, `CallerHost`, `NativeCaller::create_reply`, `invoke` | Deadline starts before discovery; marker is written once, but caller always supplies NO_DESTINATION. `invoke` opens only a caller connection. | Add strict capture/revalidation methods and Bring Here diagnostics. Freeze before request dispatch; never update the marker. Caller does not own the selected monitor or body geometry. |
| `src/x11/control.rs`: `validated_route`, `route_request_with` | Validates peer, epoch and destination marker; `admit` checks lifecycle/Busy/duplicate. Then **both** BringHere and Recover are completed Unsupported. | At final enablement, retain the guard only for Recover; initialize admitted Bring Here at Placement with observation due. Keep transport free of body mutation. |
| `src/x11/control.rs`: `finish_remote_with`, `complete`, `finish_active` | Shutdown handles Bring Top history specially; other active commands use generic Closing detail. `finish_active` clears after one reply attempt, even failure. | Add admitted Bring Here shutdown construction through placement proof rules. Preserve history, clear current evidence, attempt one correlated reply, and clear active state. New requests during Closing retain existing lifecycle replies. |
| `src/main.rs`: `parse_args`, `print_help`, `service_remote_with`, `unsupported_placement` | CLI exposes three accepted commands. Service boundary independently rejects both placement commands. | Expose only `--bring-here` in the last checkpoint and dispatch it to a dedicated Here progression function. Retain Recover rejection at both boundaries and CLI rejection of `--recover`. |
| `src/main.rs`: `service_owner_operation` | Verifies selection, services when due, and completes once. Its placement error arm currently fabricates only Unsupported because no placement execution exists. | Route Bring Here failures through the new proof-aware finalizer, including native error and possible dispatch; preserve original fatal errors after reply/cleanup attempts. |
| `src/main.rs`: `VisibilityHost`, `BringTopHost`, `NativeVisibility` | Native adapter borrows runtime/window/bounds; it owns no operation. Existing observation compares body desktop against a fixed visibility baseline and therefore cannot accept planned relocation. | Add a narrow `BringHereHost` seam and implementation on the existing adapter. Reuse low-level observation/fence/enable/correction capabilities, but use separate Here preservation rules and the Stage-B layout reader. Do not route Here into `service_show_with` or `service_bring_top_with`. |
| `src/main.rs`: `ProbeRuntime`, `InputFence`, `fence_visibility_input_with`, `visibility_dispatch_eligible`, `restore_visibility_input_with` | Owns gestures, release debt, popup, movement, body availability, SHAPE proof/freshness. `noop_ready()` alone does not prove geometry or settled gestures. | Keep all native resource/input ownership here. Require a stronger Here no-op predicate; reuse checked fencing and restoration. Add only the local recovery restriction described in §8 if needed to prevent a second correction. |
| `src/main.rs`: `run_probe`, both `route_owner_request` sites, `remote_service_due`, `remote_operation_deadline`, `mark_remote_observation` | Bounded 64-event batches, immediate post-admission and end-of-batch service, buffered-event draining. Placeholder placement branches either always service or ignore observations. | Add command-specific due flags/deadlines/event routing. Planned desktop/map churn must not fall into legacy layout correction or an unconditional interruption. |
| `src/x11/monitors/destination.rs`: `query_destination_layout` | Takes an already frozen target and owner-retained monitor; exactly two validated reads. It does **not** capture the caller target. | Keep its policies intact. Add a small caller count/current reader sharing strict decoders, and project accepted DestinationLayout into a pure equality identity for the operation. |
| `src/x11/monitors.rs`: `LayoutAtoms::subscribe`, `query_control_layout` | Number/viewport atoms exist and root subscriptions already cover properties, but legacy event predicates ignore these atoms. Legacy reader has different workarea policy. | No new subscription or legacy policy change is needed. Observe number/viewport only in Here-specific routing. Do not call `subscribe` merely to capture a caller desktop. |
| `src/x11/desktop.rs`: `DesktopRequest::new(...).send_before(...)` | Consumes one envelope; marks possible dispatch after cookie creation, before check; no WM confirmation. | Call only from the admitted/fenced/revalidated Here send phase. Keep the accepted implementation unchanged unless an independently reviewed necessity arises. |
| `src/x11/visibility.rs`: `VisibilityAtoms::observe`, `VisibilitySnapshot` | Two-read strict ICCCM/EWMH evidence, genuine HIDDEN classification, layer flags, BadWindow handling. | Reuse unchanged snapshots as evidence. Add operation-level eligibility in main, not a looser global classifier. |
| `src/x11/state.rs`: `read_body`, `qualify_remote` | Local layer reads require Viewable; qualified remote reads reject minimized bodies and require the old body desktop. | Neither is a general Here layer validator. Use coherent visibility snapshot layer/support fields, which can describe a genuinely minimized body, with Here's own membership policy. Do not weaken existing layer semantics or create another controller. |
| `src/x11/window.rs`: `correct_show_position`, geometry query/reconciliation | Checked fixed-size position correction, no pending moves, 150 ms reserved inside owner budget. | Narrowly generalize its name/documentation for bounded placement correction, keeping the Show wrapper/behavior or updating its call mechanically. No resize/stack flags. Here records attempt consumption before calling it. |

`layer.rs`, `interaction.rs`, pointer/menu/SHAPE implementations, Cargo and CI should need no behavioral change. Reuse their contracts; stop if changing them becomes necessary beyond an approved checkpoint's files. There is no new service, request queue, controller, background task, or completed-request history.

## 3. Caller identity, capture, and owner admission

### 3.1 Capture without inventing owner information

The caller cannot call the full Stage-B destination reader meaningfully: it has neither the owner's startup-retained monitor atom nor a reason to choose another monitor. Capture only the desktop identity. Full destination/layout admission belongs to the owner.

Proposed caller flow, under the existing single **5000 ms** Bring Here deadline:

1. Allocate the immutable deadline at the current `caller_with` entry, before atom/selection discovery. Check it between synchronous phases; reads never renew it.
2. For Bring Here only, intern the two count/current atoms without selecting events, and perform exactly two complete count/current snapshots. Use Stage-B exact CARDINAL metadata checks, nonzero count, ordinary current index below count, and pair agreement. Freeze that current index before owner discovery proceeds. Failure causes a local diagnostic and no request; no default desktop 0, sticky sentinel, or retry.
3. Discover and watch the owner using existing lifecycle/epoch validation. Create the never-mapped reply endpoint and write the eight-word v2 marker **once**, with this frozen index. Existing commands continue using NO_DESTINATION and do not acquire the new layout dependency.
4. At the existing send-boundary owner revalidation, read count/current again with the same bounded two-read helper. Require agreement with the captured count/current pair. Any change or unreadability ends this invocation without dispatch; do not rewrite the marker or start a second invocation. The count is local capture evidence, not a new wire field.
5. Recheck the same deadline, send the private request once, and await the correlated response using existing bounded draining/peer-loss behavior. A user desktop change after sending must be caught independently by the owner. No reconnect or redirect to a replacement owner.
6. Attempt existing checked caller cleanup on every exit. Keep current outcome/exit conventions: capture failure is a local pre-send failure, with explicit “request not sent” wording; a post-send timeout or lost owner/reply is OutcomeUnknown. Existing pre-send expiry wording may remain, but must explicitly identify that no request was sent.

The count/current reader is a narrow extension in `destination.rs`, sharing `one_word`/`parse_count` rules. It does not repurpose legacy diagnostic/control readers or introduce full monitor/workarea capture into IPC. It is bounded read agreement, not an atomic snapshot. A change away and back between all observations can escape detection; no protocol here establishes continuous atomic desktop identity.

### 3.2 Admission and single ownership

Retain `validated_route` and `admit` correlation, immutable marker copying, Starting/Closing/Busy behavior, duplicate matching and local-layer Busy arbitration. The single owner deadline is **admission +3500 ms**. An exact active duplicate does not reset it; a changed marker cannot retarget the active request. No completed-request replay cache is added.

At final enablement only, `route_request_with` passes admitted Bring Here to main with Placement/observation-due state. Recover still completes Failed/Unsupported/Preflight before any body host method. `service_remote_with` retains the same defensive Recover guard. A valid forged/internal Recover request must remain harmless, even though protocol v2 can represent it.

A pre-existing ordinary geometry correction may return Busy at preflight before baseline/progress/mutation; its ownership must not be stolen. A normal drag or popup is not a new queue: when Here needs safety work, the accepted fence cancels it neutrally. Local layer requests remain Busy while any remote operation is active; Bring Here never borrows or drives the LayerController.

## 4. Proposed operation state and evidence

All request-specific state remains inside `ActiveOperation.placement`. Suggested internal names are implementation guidance, not new wire values:

| Value | Purpose |
| --- | --- |
| `HerePhase::{Observe, Fenced, DesktopAttempted, AwaitDestination, Geometry, EnablingInput, InputRestored}` | Bounded progression; no recursive admission. Attempted is consumed before native send. Finalization is a function, not a second controller. |
| Existing immutable `destination_desktop` and `baseline` | Original ordinary body desktop, genuine minimization boolean and effective layer, written once from admissible evidence before any fence/relocation. Never replace baseline on later snapshots. |
| Optional pure layout identity | Exact count/current/target, retained atom and rectangle, target workarea presence/value, viewport presence/all-zero evidence, fixed size and checked bounds/usable area. Store values, not a hash or native connection. |
| Observation-due flag and latched invalidation reason | Events schedule fresh reads; confirmed layout/preservation invalidation prevents further operation mutation even if later data happens to match again. |
| Desktop attempted / possible-dispatch / checked facts | Separate attempt consumption from the aggregate wire `host_mutation_dispatched` bit and from actual membership proof. No flag alone proves relocation. |
| Correction attempted and optional `(target, confirmation_deadline)` | At most one correction, consumed before the native call including checked failure. It is not `runtime.correction` and does not extend the total deadline. |
| Input-enable attempted/restored facts | Distinguish safe checked restoration from an uncertain SHAPE write and support one safety re-fence. |
| Existing `proof.current` and `proof.history` | Current postcondition is rebuilt from fresh evidence; history is monotonic and independent. No restoration history is set by Here. |

Use pure geometry/value types for the stored layout identity in `control.rs`; keep native `DestinationLayout` and X11 errors in the host adapter. Test that the projection preserves every Stage-B equality field and excludes only the same irrelevant monitor metadata. This preserves source direction and avoids putting a native module type into pure control state. It can retain ActiveOperation's Copy/value semantics.

An evidence round is a bounded **layout → visibility/body → layout** sequence: each layout read uses Stage B's existing two-read agreement, and visibility uses its accepted two-read observer. Both layouts must match each other and the operation's admitted layout identity, and the visibility current desktop must equal the frozen target. Geometry is inserted into this bracket when needed; the final round also rereads visibility after geometry. This strengthens cross-reader consistency without claiming an atomic server transaction. There is no loop within a read helper, third-read repair, or server grab.

Before a baseline exists, accept only a coherent supported ordinary body desktop below the observed desktop count, a readable exact Normal/Above/Below layer, and one of Minimized, NotMinimizedHere, NotMinimizedElsewhere. Reject all-desktops as Unsupported; reject missing/malformed desktop/state/support and conflicting layers as Unverifiable. An initially transitional/incoherent body returns Failed/Unverifiable at preflight without fencing or relocation: do not retain an unfenced pending operation that could swallow ordinary input/lifecycle safety events. Once a baseline and fence exist, expected transitional state may await a relevant event or the fixed deadline. Withdrawn/destroyed state ends the operation. Initial unsupported viewport is Unsupported; invalid destination evidence is Unverifiable; a stale frozen current desktop is LayoutChanged.

After the baseline is captured, its layer and genuine minimization remain preservation obligations. For an originally non-minimized off-workspace body, Iconic + Unmapped + **no HIDDEN** is valid NotMinimizedElsewhere, not minimization. A coherent opposite genuine-minimization state is a preservation failure (Unverifiable with a specific diagnostic); do not “repair” it. Coherent different layer is LayerChanged; contradictory/unreadable flags do not establish Normal.

## 5. Bring Here state machine

### 5.1 Observe and already-local decisions

Start with fresh ownership and the evidence round above. Do not empty input merely to read it.

**Safe visible local no-op:** body already belongs to the frozen target; layer/minimization baseline is valid; strict destination layout fits; actual geometry has the fixed 160×160 size and is in checked bounds on the retained monitor; runtime input is already enabled with valid freshness, Ready, and no unsettled release, popup, gesture, pending/final/in-flight movement or correction obligation. Perform the final fresh evidence round and return Success without desktop, geometry, SHAPE, map, layer or pointer mutation. `noop_ready()` alone is insufficient. An already-local no-op creates no historical relocation.

**Local but unsafe or out of bounds:** if a separate local correction owns geometry, return preflight Busy. Otherwise, a readable valid layout plus repairable input/gesture/position state proceeds through fencing and at most one clamp correction, with **zero desktop requests**. Invalid size, unsupported evidence, absent/ambiguous monitor or impossible fit is failure, not centering, resizing or monitor fallback. A correction/input repair without relocation can succeed when the full postcondition is confirmed, but cannot authorize Partial.

**Already-local genuinely minimized:** if a previously checked empty fence is fully settled, no extra fence is necessary; otherwise settle it. Return only qualified minimized Success after fresh preservation/layout validation. No actual minimized geometry correction or input enable occurs, and no desktop request is sent.

**Remote body:** proceed to Fenced after valid original evidence. Do not require remote actual geometry to be trustworthy before relocation. The strict destination capacity/representability check must already pass; actual geometry is read after local viewability. Preserve the existing root origin as far as valid placement permits rather than inventing a center.

### 5.2 Fence and revalidate before any relocation

Use the accepted checked fence order: logically block input; clear stale post-map allowance; empty native input SHAPE; invalidate popup generation/gesture/coalesced and final movement; establish transition release debt; attempt checked ungrab; attempt popup destruction even after an earlier failure. Keep original error priority and retained resource obligations. Cancellation never flushes a final drag target, toggles color, invokes a popup selection, or replays a click.

`visibility_dispatch_eligible()` must be true before the desktop request: checked empty SHAPE, no tracked/transition pointer debt, no popup, only Idle or suppressed-release interaction, no queued/final movement, no local correction, no safety observation/deadline, live body. Failed empty SHAPE or release/cleanup means **no relocation**. Existing settled fences may be reused; an unsettled fence is a safety failure, not permission to send.

After cancellation, discard any prior geometry origin sample and repeat owner ownership plus the full evidence round. The frozen layout must equal its original identity, the body must still have the baseline layer/minimization, and body desktop must remain the baseline desktop. A coherent body move before our send is external layout interruption, even if it happened to reach the target; do not credit it as our planned request or silently adopt a new baseline. No mutation starts if the deadline was crossed during these reads.

### 5.3 Single native request and confirmation

For a remote baseline only, consume DesktopAttempted **before** constructing/calling `DesktopRequest`. Use the existing body XID, owner root and already interned desktop atom; supply the immutable target and operation.deadline. The exact Stage-C envelope remains `[target, 1, 0, 0, 0]`, format 32, body event window, root destination, `SUBSTRUCTURE_NOTIFY | SUBSTRUCTURE_REDIRECT`, propagate false.

Record possible dispatch separately, and OR it into aggregate dispatch detail without ever clearing a prior true value. `Ok(false)` means expired without a send; `Ok(true)` means checked X transport only. Both send failure and check failure consume the attempt. No callback, timeout, event, duplicate or late observation may recreate the request.

After a successful checked send, return to the existing event loop to drain buffered WM events. AwaitDestination allows fresh coherent membership on the original desktop while waiting, or on the frozen target. Any third desktop, sticky conversion, changed frozen-current relationship or changed layout identity invalidates the operation. After destination membership has been confirmed, a later return to the original desktop is also an interruption; it is not permission to send again.

Set historical relocation exactly once when fresh supported/coherent evidence confirms the same body on the target, target still current, with an ordinary baseline on another desktop and a consumed possible-dispatch attempt. Membership can be established while map/WM_STATE readiness is still transitional; it is not yet Success. Layer/minimization/geometry/input checks are separate. If another preservation check fails in that same observation, retain independently established membership transition evidence as history before producing the incomplete result. In particular, a rejected destination layout must prevent readiness/mutation, but need not erase relocation that the strict body/current reader independently proved. No history is added when membership/current evidence itself is unreadable, incoherent or stale. Neither request acknowledgement, event payload nor geometry correction establishes history.

### 5.4 Two completion branches

**Genuinely minimized baseline:** require fresh Iconic + Unmapped + HIDDEN, target membership/current relation, original effective layer, matching strict destination layout and representable fit, and a fully settled checked empty fence. Always use the conservative Stage-D branch: `geometry=Deferred`, `placement_deferred=true`, `input_ready=false`, `minimization_preserved=true`. Report that actual geometry/input readiness awaits later local restoration. H0's trustworthy minimized geometry is not permission to correct or enable a minimized body. No MapWindow, REMOVE-HIDDEN, user-time mutation or Above request is allowed.

**Non-minimized baseline:** wait for coherent NotMinimizedHere (Normal + Viewable + no HIDDEN on the target). Observe actual translated root geometry with the fixed size. Use the Stage-B retained-monitor/target-workarea bounds plus the existing signed X11 coordinate representability check (`checked_probe_bounds` equivalent). If origin fits, reconcile live origin and stale movement bookkeeping. If it does not fit, clamp **that live existing origin** once into the retained monitor's valid target bounds. Do not center, move to the caller's pointer monitor, or choose the new primary.

Before correction, revalidate layout, membership, viewability, original layer/minimization and settled fence; require the entire **150 ms** confirmation interval to fit within the remaining 3500 ms deadline. Consume correction attempt before calling the generalized checked correction helper. Its cookie acknowledgement proves no WM movement. Wait event-driven until its fixed confirmation deadline and query actual geometry; require exact target, fixed size and current bounds. Ignore notification coordinates as proof. A mismatch, changed bounds or checked error cannot trigger a second correction.

After placement, repeat fresh layout/visibility/geometry checks. Install the verified destination usable area and bounds into the existing borrowed runtime layout values before local interaction resumes, so subsequent dragging and popup fit use that same validated area; never replace the selected monitor atom. Restore the exact original silhouette through `restore_visibility_input_with` only when all local conditions are satisfied; set Ready and confirmed origin through the existing adapter, consume stale movement, and install the returned full X sequence as the freshness boundary. No expired owner phase may enable input. Clear any pending map-input allowance because this checked restoration itself supplies the post-map shape.

Finally perform a fresh read-only evidence round after enabling. Success requires all current postconditions at this final boundary, including actual geometry and safely enabled input, not flags retained from earlier phases. If final evidence fails after an attempted enable, logically block and perform one checked safety re-fence/release/cleanup before completing; never retry enabling. A proposed local no-op whose final read loses local visibility/geometry safety also requires this safety cancellation; it is no longer the fully safe no-op case. Safety cleanup may be necessary after the deadline but does not extend it or permit recovery mutations.

## 6. Event routing, deadlines, and lifecycle

### 6.1 Planned change versus interruption

Introduce a Here-specific evidence-event predicate and routing branch before the legacy visibility/layout paths. Do not broaden `relevant_layout_event` for all commands. Relevant Here inputs are:

- Body `_NET_WM_DESKTOP`, `_NET_WM_STATE`, `WM_STATE`, MapNotify, UnmapNotify, ConfigureNotify and DestroyNotify.
- Root `_NET_CURRENT_DESKTOP`, `_NET_NUMBER_OF_DESKTOPS`, `_NET_WORKAREA`, `_NET_DESKTOP_VIEWPORT`, `_NET_SUPPORTED`, root ConfigureNotify and RandR layout notifications.
- Existing selection/control lifecycle and asynchronous error events, with their existing priority.

Events request observation, never confirm membership, geometry, layer or minimization. A body desktop notification after the one dispatch is expected evidence, not an unconditional LayoutChanged. A root/RandR notification triggers a strict comparison; equal relevant evidence may continue, but a changed count, target workarea presence/value, viewport presence/zero compatibility, retained identity/rectangle or frozen-current relation latches failure. A valid unrelated monitor/primary/order change or valid non-target workarea change must follow Stage-B equality semantics rather than fail merely for receiving a notification. Malformed/unsupported evidence cannot refresh an old valid layout into permission.

Do not hard-code H0's order. Desktop-first, map-first, WM_STATE-first, duplicated, delayed and missing notifications must all converge through fresh snapshots. During a fenced pending operation, transitional/incoherent observations clear current confirmation and wait for another relevant event or the final deadline; initial preflight follows §4. No permanent polling timer is added. Unsupported, withdrawn, destroyed and coherent preservation drift are terminal. An unreadable snapshot is never success; where waiting could resolve WM churn, the owner deadline bounds it. Persistent unreadability ends Unverifiable or ConfirmationTimeout according to whether evidence is invalid or merely not settled.

While Here owns placement, consume its geometry/map/desktop evidence so it cannot fall through to ordinary `runtime.placement`, automatic correction, `synchronize_map_input_with`, or legacy visibility-baseline checks. Expected Unmap must leave input unavailable/fenced and not discard Here's request/history/correction-attempt state. Genuine body destruction still marks `window_resource` externally destroyed, invalidates current proof, settles lifecycle cleanup and ends the run. Delayed Unmap after a map is resolved through current reads; it must not manufacture a new transition or reopen input. Popup lifecycle/resource errors still use existing cleanup bookkeeping.

After completion, late events become ordinary lifecycle evidence. They do not reopen the completed operation, generate another reply, retarget, or repeat its desktop/correction request. They also cannot bypass the retained input fence. Map-related SHAPE behavior must be tested with late queued MapNotify after a successful Here: the already restored exact shape has no new reassertion allowance.

### 6.2 Scheduling and fixed time

Keep service at loop top, immediately after both consumed-control routing sites, at active deadlines within event batches, and after each bounded batch before sleep. Set a Here observation-due flag for relevant events and phase transitions that have immediate read-only work. Clear it after service; replace the placeholder unconditional `true` in `remote_service_due` so Here does not busy-spin.

For Here, `remote_operation_deadline` returns the earlier of its immutable total and an outstanding correction confirmation deadline. Do not read ShowProgress or Bring Top's 2500 ms visibility cutoff for Here. Socket wait still uses the minimum of all actual operation/runtime/duration deadlines and drains buffered events before waiting. More than 64 incoming events, duplicate requests and held gestures cannot defer the fixed deadline indefinitely.

Check time before every operation mutation and after synchronous evidence phases. At expiry, do one bounded final read-only validation; allow Success only if complete readiness was already safely established (or the minimized qualified postcondition is already true). No send, correction or input enable starts at expiry. Reads that cross the deadline lead to this finalization, not a renewed phase budget. Accepted synchronous X11 calls still assume a responsive server; these logical deadlines are not independently enforced transport timeouts.

### 6.3 Lifecycle and completion paths

Before servicing and immediately before native dispatch, verify current selection ownership. Never operate a replacement owner/body or use a stale epoch. Control loss, body destruction, Quit/WM close, duration expiry and fatal X11 errors must reach the placement finalizer or placement-aware shutdown path. Shutdown clears all current confirmation and input-readiness claims, retains historical relocation and dispatch, chooses Failed/Partial according to §7, and attempts one reply before ordered teardown. The existing generic Closing result is appropriate for a newly arriving request during Closing, not for erasing an active Here's history.

Caller disappearance does not authorize retries or a new request. The owner can finish its existing bounded safety work; BadWindow when replying clears the active operation after the one attempt. If owner/connection loss prevents a trustworthy reply, the caller reports OutcomeUnknown. Cleanup continues after errors, retaining the original fatal error and resource/release obligations. A successful terminal send is not a guarantee that the caller received it.

## 7. Proof, results, diagnostics, and native errors

Use `OperationResult::placement(Command::BringHere, ...)` for every admitted Here completion, including errors and shutdown. Preserve the accepted validator rather than loosening it to fit an implementation. Use Placement or Geometry for executed results, Preflight for unstarted rejection, Shutdown for admitted shutdown. Bring Here never reports Stage::Show or Stage::Layer and never sets historical restoration.

| Evidence at completion | Required result |
| --- | --- |
| Complete fresh non-minimized local postcondition, original minimization/layer preserved, valid geometry and safely enabled input | Success / None. History may be false for local no-op/repair or true for relocation. |
| Complete fresh minimized branch with checked empty fence, preserved minimization/layer and admitted/deferred fit | Qualified Success / None, explicitly deferred geometry/input. It does not claim visibility. |
| Incomplete current postcondition, previously confirmed real relocation from a remote baseline | Partial with actual failure reason; retain history even if all current proof is now unknown. |
| Incomplete current postcondition without confirmed relocation | Failed with actual reason. Dispatch, local no-op and correction alone cannot yield Partial. |
| Pre-admission Busy/Starting/Closing/protocol rejection | Existing lifecycle/transport result with no placement progress. |
| Caller lacks a trustworthy timely terminal reply after dispatch | OutcomeUnknown; no inference from owner logs or eventual WM state. |

Current proof is rebuilt, not accumulated: clear destination/geometry/minimization/layer proof and current visibility/input/Above confirmation whenever their supporting evidence is lost or invalidated. Keep independently justified current fields only when they pass the constructor together. `PlacementProgress::clear_current()` alone does not clear `Detail.progress`, visibility/workspace/layer observations or Confirmation; the finalizer must reset those coherently too. Historical relocation survives later layout drift, layer drift, unreadability, destruction and shutdown. Complete postconditions must not be encoded as Partial/Failed merely because historical progress exists; this was the corrected Stage-A partition.

For a genuinely minimized Above body, `detail.layer=Above` and `layer_preserved=true` describe its layer; **do not** set `above_confirmed`, whose accepted semantics require a confirmed not-minimized state. Likewise deferred geometry proves retained-monitor/area fit, not actual origin. Do not infer “moved” from `host_mutation_dispatched`, since that bit may represent a geometry request and is not a transition proof.

Proposed reason mapping: stale/different valid frozen layout or unexpected desktop → LayoutChanged; coherent different layer → LayerChanged; absent required WM capability or unsupported viewport/sticky conversion → Unsupported; malformed/unreadable/contradictory preservation evidence or changed genuine minimization → Unverifiable; pending coherent transition at expiry → ConfirmationTimeout; unsettled checked release → ReleaseFailed; native request/SHAPE/resource failure → X11Failure; destroyed/withdrawn/ownership loss/closing retain their existing explicit reasons. Initial invalid fit/monitor/size is Unverifiable with a precise diagnostic. If an established valid layout later becomes invalid, invalidate the operation; never substitute a fallback or new baseline. Preserve error specificity in logs without adding protocol reason values.

**Checked send failure after possible dispatch:** consume the attempt, retain dispatch, and forbid any subsequent desktop or geometry request for this operation. Preserve the existing `service_owner_operation` policy that native HostError ends the owner run after reply/cleanup attempts. Enter fatal teardown state before constructing the terminal result; readiness/current confirmation is invalidated by that lifecycle transition. If the connection is usable, a bounded read-only observation may still establish historical relocation, including for a minimized body, but must not reestablish an operation Success while teardown is committed. Clear current proof and use Failed or Partial from the retained history, with X11Failure and Shutdown for fatal teardown. This follows the accepted shutdown model rather than presenting a complete continuing postcondition and then labeling it Failed. Never describe Failed as proof that the WM did not move the body. Preserve the original native error even if final observation, terminal reply or cleanup also fails. Do not make an uncertain checked error a retry trigger or introduce a new recoverable-error policy in Stage D.

The same discipline applies to correction/SHAPE errors: mark attempts before calling native helpers; retain possible dispatch on checked failure; no retry or rollback. Failed or uncertain input enabling requires the one safety re-fence, not a second enable. All constructor failures are implementation bugs to catch in deterministic tests, not reasons to fall back to Unsupported or discard history.

Bring Here diagnostics should distinguish “already here; placement/input verified,” “destination confirmed; original layer preserved,” “still genuinely minimized; geometry/input deferred,” and “relocation confirmed earlier; current placement unverified” for Partial. Avoid asserting causal relocation or visibility from a send acknowledgement. Report current proof and historical progress separately, especially after LayoutChanged and timeout.

## 8. Deferred readiness and correction ownership

Minimized qualified success leaves the same body/input fence in place. A later explicit accepted Show or ordinary genuine local viewability transition independently revalidates visibility, selected monitor, current workarea, actual fixed-size geometry, settled obligations and exact SHAPE before local input. It does not revive Here, retain its deadline, reuse its destination as a new command, or change the completed reply. Recover remains a future Stage-E caller of accepted readiness behavior.

Review `restore_deferred_input` explicitly: today it uses the legacy `query_control_layout` and may start `runtime.placement_with` correction. It must never run while Here owns placement; Here uses strict Stage-B evidence throughout. After successful minimized deferral, later ordinary recovery can use the accepted independent local policy, since no Here correction was attempted and no historical result is being extended.

Prevent a failed/uncertain Here correction from becoming an automatic second correction through that deferred path. Proposed minimal runtime state is a fence-local **automatic correction permitted** flag, consumed before a Here correction attempt and retained if Here ends fenced. It is a local safety restriction, contains no request ID/destination/result/history, and is not a completed-request service. While consumed, deferred recovery may reread and enable an already safely in-bounds body, but may not send another ConfigureWindow. A newly admitted explicit Show or Bring Top can use its own accepted correction budget after fresh preflight; a new explicit Here has its own budget. Do not reset the restriction merely on late Map/Configure/property events or when `active` becomes None. A successful operation that restores input can clear the restriction with the fence; minimized Here success without an attempt leaves independent later correction available. Preserve legacy behavior for fences originating solely in Hide/Show/Bring Top.

This flag and the guarded recovery adapter must be tested together with all normal event paths: while the restricted fence exists, no ordinary `runtime.placement`, timeout handler or map branch may dispatch a correction behind it. If code inspection during implementation shows the existing fatal-error path already prevents a particular retry, retain that protection, but do not rely on process exit to cover nonfatal result completion.

## 9. Proposed dependency-ordered checkpoints

These are **M03.3 D1–D4**, not the historical M03.2 D or Stage-G D1/D2 deadline decisions. Each requires a new bounded assignment and independent review before moving on. All source paths below are relative to `experiments/desktop-probe/`. No checkpoint automatically authorizes evidence/STATUS/AGENTS/Cargo/CI edits; an implementation note, if requested, must be explicitly included in that assignment.

For each code checkpoint, review the exact diff and matching CI after local `cargo build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`, all using `--manifest-path experiments/desktop-probe/Cargo.toml`, plus `git diff --check`. Retain all accepted tests; new tests must assert observable policy/order/proof, not only internal field values. No fixed future test-count target is proposed.

### D1 — Pure Here progression and proof decisions

- **Objective:** define value-only Here phases/layout identity, attempt/correction/deadline facts and pure membership/preservation/result decisions under existing PlacementProgress. Do not wire a host call.
- **Permitted files:** `src/control.rs` only, including its tests.
- **Ownership:** expand the existing single ActiveOperation's placement record; no runtime resource ownership, second state machine owner or change to wire protocol.
- **Invariants:** destination/baseline/deadline immutable; current/history separation; correction and desktop attempts consumed once; Here cannot set restoration history; accepted result partition and other command budgets unchanged.
- **Deterministic tests:** all phase exits, original/target/third desktop decisions before and after dispatch, local no-op/correction without Partial, relocation then lost evidence, minimized Deferred Success, Above-minimized bit constraints, immutable 3500/5000 budgets, correction interval fit/expiry, shutdown proof partition, and accepted A cross-products unchanged.
- **Native host testing:** not required; no execution path changes. **External behavior:** none; both guards and CLI remain unchanged.
- **Review gate:** pure definitions must be sufficient for D2/D3 and independent of native modules. Tests must not weaken accepted validators. **Exclusions/stop:** no Recover phase implementation, protocol migration or general framework; stop if new result bits/reasons or a second owner seem necessary.

### D2 — Read-only capture and narrowly bounded native adapters

- **Objective:** implement caller count/current capture/revalidation helpers, strict destination-to-pure-identity conversion, and Here host adapters for observation/one-shot desktop transport/checked correction. Native relocation remains unreachable from production admission and service guards.
- **Permitted files:** `src/x11/monitors/destination.rs`, its existing `src/x11/monitors/destination/tests.rs`, `src/x11/control.rs`, `src/main.rs`, `src/x11/window.rs`. No changes to Stage-B validation policy or Stage-C envelope.
- **Ownership:** caller stores temporary captured identity under its existing deadline; the adapter borrows existing native resources and stores no request. Pure operation definitions come from D1. Capture helpers can be tested before production `caller_with` chooses them in D4.
- **Invariants:** two reads, no retry/default target, one eventual immutable marker, legacy command readers unchanged; request adapter requires explicit admitted target/deadline and settled fence; checked correction preserves full 150 ms and possible dispatch. No new subscription or monitor choice in caller.
- **Deterministic tests:** strict count/current metadata/range/agreement, send-boundary drift, projection equality including workarea/viewport presence, signed coordinate range, exact root/body/atom/target passed to the accepted transport seam, deadline-before-mutation, fence refusal, correction checked error accounting, and legacy Show correction equivalence. Unwired adapter tests use injected closures/hosts, not live relocation.
- **Native host testing:** not required for unreachable adapters. **External behavior:** none; no new CLI flag, both Unsupported boundaries unchanged, existing commands unchanged.
- **Review gate:** source reachability audit proves no production Here mutation; all accepted Stage-B/C tests and M03.2 checks pass. Any narrowly necessary dead-code allowance must identify the guarded stage and be removed when wired. **Exclusions/stop:** no widened property fallback, changed remote layer qualifier, or desktop transport modification; return for review if an adapter requires those.

### D3 — Complete guarded owner service, lifecycle and recovery integration

- **Objective:** implement the full Here service/finalizer, native adapter use, phase-aware event/deadline treatment, input/correction ownership and shutdown/error completion. Keep production transport and service Unsupported guards; invoke the complete Here service directly only through deterministic test seams.
- **Permitted files:** `src/main.rs`, `src/control.rs` for narrowly required pure progression refinements/tests, `src/x11/control.rs` for shutdown construction and guarded routing tests. Reopening D2 files for behavioral changes requires reporting the need in the assignment/review.
- **Ownership:** PlacementProgress remains sole request progression; ProbeRuntime retains input/resources plus the fence-local deferred-correction restriction; no request history is retained after completion. Both guards prevent partially integrated production exposure.
- **Invariants:** every rule in §§3–8, especially expected desktop/map churn, one send/one correction, safe local no-op, preservation on minimized bodies, final fresh truth, one reply and original-error priority. Existing Hide/Show/Bring Top services and LayerController behavior stay command-specific.
- **Deterministic tests:** full matrix in §10, using a fake Here host with an ordered call log, virtual time, scripted snapshots and injected errors; drive actual scheduler/routing helpers at both consumed-request sites and batch/sleep boundaries. Verify no native actions occur through guarded public paths even when a valid Bring Here marker is injected.
- **Native host testing:** no Here host run while guarded; none is required for acceptance of this internal checkpoint. **External behavior:** none. Shared-path regressions still block review.
- **Review gate:** independent review must account for every operation terminal/error/shutdown route, event consumer and deferred recovery entry, and match CI on the exact guarded SHA. This is the largest checkpoint, but it remains bounded to one fully testable service and its existing loop integration; splitting event/error handling away from it would leave an unsafe public path. **Exclusions/stop:** no removal of guards, no Recover, no retry/rollback, no hiding an unresolved lifecycle hole behind later Stage F.

### D4 — Public Bring Here enablement and owner acceptance

- **Objective:** connect the prepared capture into `caller_with`, admit/service only Bring Here, expose `--bring-here`, and give truthful diagnostics. Perform integrated owner-native acceptance after independent source/CI review.
- **Permitted files:** `src/main.rs`, `src/x11/control.rs`; tests in those files. A newly required behavioral correction in other files requires an explicitly revised bounded assignment before expansion.
- **Ownership:** no new owner/state; D4 connects the already reviewed caller and owner paths. Retain both Recover Unsupported guards.
- **Invariants:** integrated identity/deadline/correlation, same body, no implicit Show/layer mutation, no partial public implementation. New command is mutually exclusive with other commands/diagnose/delay/duration; no desktop-number argument or menu shortcut is introduced.
- **Deterministic tests:** real public routing through the injected host, marker frozen once, Bring Here admission and dispatch, recovered shutdown/error result, diagnostics/exit codes, parser/help, Recover guard, unchanged Hide/Show/Bring Top call logs, and duplicate/Busy/owner-loss scenarios from §10. Retain the guarded checkpoint's service tests; update only tests whose expectation changes deliberately at enablement.
- **Native host testing:** required, §11, on the exact reviewed executable. **External behavior:** this is the first publicly callable Bring Here checkpoint. It exposes a complete candidate, not a partial relocation demo.
- **Review gate:** exact diff and matching CI first, then owner Openbox matrix and explicit acceptance. Publication/CI alone is not acceptance. **Exclusions/stop:** no Recover/Stage E, no host configuration editing, no changes to main/merge/release policy. A failed focus, minimization, monitor, input, preservation or no-retry gate stops acceptance and requires a scoped correction; do not work around it with forbidden mutations.

Four checkpoints separate pure policy, native seams and the high-risk owner integration from a small public switch. A smaller division is reasonable only if review can keep D2 and D3 fully auditable together; keeping two Unsupported boundaries until the last checkpoint is mandatory in this proposal. Further subdivisions must not expose an incomplete command.

## 10. Deterministic / injected verification matrix

These tests establish software policy and failure accounting, not native Openbox behavior. Use existing fake-host patterns and virtual clocks; no sleeps or physical desktop changes. Record action counts and ordering, including all forbidden host operations.

| ID | Scenario | Required assertions |
| --- | --- | --- |
| T01 | Capture success, absent/malformed/current out of range/sticky, two-read disagreement, capture or discovery consumes budget | Frozen exact target; bounded reads; one marker write; zero request on capture failure; no fallback/retry/deadline renewal; legacy commands do not capture. |
| T02 | Owner changes, current/count changes before send, marker altered after admission, exact duplicate | No redirect; marker not rewritten; duplicate does not renew deadline; active destination unaffected; one request/reply owner. |
| T03 | Starting/Closing/local-layer Busy/active remote Busy; valid Recover request | Existing lifecycle policy; Recover rejected in transport and service with zero body/WM/input calls. |
| T04 | Already-local visible safe no-op for Normal/Above/Below | Fresh strict layout/geometry and final evidence; zero fence, pointer release, relocation, correction, map or layer requests; no history. |
| T05 | Local unsafe gesture/popup/input or out-of-bounds geometry; existing local correction | Fence/neutral cancellation when needed; zero desktop sends; at most one clamp; no Partial from repair; local correction preflight Busy without interference. |
| T06 | Remote visible each layer and off-workspace Iconic/no-HIDDEN | Baseline not minimized; exactly one target send after settled fence/revalidation; no Show/layer calls; target confirmation distinct from local readiness. |
| T07 | Local/remote genuinely minimized each layer | No restore/map/user-time/HIDDEN/layer mutation; zero correction; qualified Deferred Success only with fit/empty fence; Above flag semantics remain valid. |
| T08 | Empty SHAPE, ungrab, popup cleanup failure, transition debt without tracked owner, stale opening/right/chord/release | No send before complete obligations; cleanup continues; no stale click/toggle/popup action or stuck logical permission; original error retained. |
| T09 | Desktop/map/unmap/WM_STATE/configure permutations, duplicates and delayed events | Fresh reads drive phases; no hardcoded order; no legacy correction/shape re-enable during Here; transitional state waits boundedly. |
| T10 | Baseline→target→third/original desktop; target notification without actual change | History only after fresh target confirmation; later drift invalidates current proof; no resend; Partial only with real prior confirmation. |
| T11 | Count/current/workarea/viewport/retained monitor change or unsupported/malformed properties before and after send | No pre-send mutation on invalidation; no retarget; post-progress Partial clears current evidence; valid non-target/primary-only changes follow Stage-B equality; no monitor fallback. |
| T12 | Layer drift, conflict/absent state, minimization flip, stale/incoherent snapshots | Initial transitional preflight ends without an unfenced pending operation; fenced churn waits boundedly. Preserve original baseline; no compensating layer/visibility request; no invented Normal/minimized state; result proof matches fresh truth. |
| T13 | Fixed size mismatch, impossible bounds, negative roots/wire limits, WM geometry unchanged/mismatch | Never resize/center/migrate; at most one position-only checked clamp; full 150 ms fit; actual target confirmation required. |
| T14 | Desktop send-before-cookie error, checked error after possible dispatch, expired send, late target evidence | Attempt consumed in every case; dispatch/history distinguished; no retry; bounded final read; fatal teardown clears current proof and retains history for Failed/Partial; ordinary expiry retains the complete/incomplete partition; owner original-error priority. |
| T15 | Correction send/check failure and duplicate Configure; enable/check failure; final evidence fails after enable | No second correction/enable; one safety re-fence; late events cannot bypass fence or trigger deferred retry. |
| T16 | Deadline crosses in each synchronous observation/mutation boundary; >64 events; missing event at final state | Fixed total and correction deadlines serviced before wait; final fresh read; no expired mutation; no busy-spin or deadline renewal. |
| T17 | Completion cross-product: complete/incomplete × history/no history, including no-op, correction and acknowledgement only | Exact accepted Success/Partial/Failed partition and constructor/terminal encode/decode validity; restoration history always false for Here. |
| T18 | Quit, WM close, duration expiry, selection loss, body destruction, caller death, failed reply at each phase | One reply attempt/clear; admitted shutdown retains history only; no stale current success; original cleanup failure priority and resource accounting. Caller no valid reply → OutcomeUnknown. |
| T19 | Successful minimized deferral then later Show/local return; failed correction then late local evidence | No revived Here/second result; independent full readiness checks; deferred correction restriction enforced across every ordinary path; explicit new command has its own budget. |
| T20 | Full accepted Hide/Show/Bring Top and local layer/movement/focus-hint/input regression suite | Existing readers/budgets/preservation and scheduling unchanged; no new destination property requirement for existing commands; single LayerController. |

Also inspect production call sites for forbidden APIs/requests: no direct WM desktop/state rewrite, current-desktop mutation, SetInputFocus, activation, MapRaised, generic restack, visibility restore or layer send reachable from Here. Existing uses elsewhere are not globally removed. Stage-C byte/envelope tests remain the authority for transport construction; integrated tests establish its invocation count and preconditions.

## 11. Proposed owner-native Openbox acceptance matrix

Run only after a separately authorized D4 executable has passed source/CI review. Use the recorded Openbox/X11/Picom host and existing dual-monitor configuration (`eDP-2` 1920×1080 +0+0, `DP-1-0` 1920×1080 +1920+0), recording actual current geometry/configuration if it differs. No host experiment is performed by D-P. Any temporary property/failure instrumentation or configuration changes require a separate explicit host-test authorization; injected tests can cover cases unsafe or impractical to reproduce natively.

For each applicable case record exact executable SHA/CI, owner command/exit/diagnostic, body and control XIDs, before/after body/current desktop, WM_STATE/map/HIDDEN/layer, geometry/retained monitor, active-window/focus metadata and **actual typing continuity**, plus interaction/click-through observations. Metadata alone does not prove focus preservation. Use independent second-process control so a covered/minimized body remains controllable. Do not run a compensating Show/Bring Top until the Bring Here result and preservation evidence have been recorded.

| ID | Owner scene | Acceptance expectation / evidence boundary |
| --- | --- | --- |
| N01 | Already-local, visible, idle, in-bounds body | Success; same position/XID/layer, no visible disturbance; no relocation/fence/correction in trace. Repeat covered Normal/Below to avoid confusing presence with raising. |
| N02 | Visible body on another workspace; caller on destination | Same body moves to frozen caller desktop, remains not minimized, geometry/input verified. User desktop and typing focus do not change. |
| N03 | Repeat N02 for Normal, Above and Below | Exact effective layer preserved. No guarantee of relative Normal-peer order or “on top”; Below may remain covered. |
| N04 | Genuinely minimize body, move caller to another desktop, invoke Here; repeat layers | Body membership changes; Iconic + Unmapped + HIDDEN and layer preserved; qualified success says geometry/input deferred. No implicit restoration. Later explicit Show independently restores interaction/click-through. |
| N05 | Non-minimized body off-workspace represented Iconic + Unmapped + no HIDDEN | Here brings it locally without treating Iconic alone as genuine minimization or invoking Show. Normal + Viewable + no HIDDEN eventually confirmed. |
| N06 | Continuous typing in a separate application throughout N01–N05 | No lost typing, body activation, keyboard-focus transfer or user desktop switch. Record real typing and metadata; failure is a stop condition. |
| N07 | After visible relocation and after explicit Show following N04 | Transparent padding click-through, interactive translucent patch, click toggle, drag and popup all work; stale releases do not trigger actions. |
| N08 | Invoke Here from an independent process during body drag, opening-right hold, popup open and popup press | Neutral cancellation, popup closed, pointer released, no click replay/final drag flush/layer selection; later fresh interaction works. Include already-local unsafe case. |
| N09 | Existing dual monitors, caller/typing app on either monitor, body retains startup-selected monitor | Destination placement uses retained name atom; no pointer/primary-driven migration. Verify clamp only if necessary and same fixed size. |
| N10 | Already-local out-of-bounds or destination with differing usable area, when safely reproducible | One necessary checked clamp followed by actual confirmation and restored silhouette; no center/migration/repeated correction. Synthetic layout tests remain T11/T13 if native setup is unavailable. |
| N11 | Change current desktop during an active invocation; safely reproducible layout/count/workarea change | Frozen invocation invalidates; no silent follow/retarget/rollback. Failed before confirmed movement, Partial after historical confirmation; current readiness claims cleared. Fast completion may require separately authorized bounded instrumentation. |
| N12 | Unsupported/nonzero viewport, missing/malformed property, monitor removal/identity loss | Mandatory injected coverage T11; native cases only if safely and explicitly arranged. Do not corrupt session properties or remove a display merely to claim a pass. Record native unexercised cases honestly. |
| N13 | Delayed/refused WM response, expiry, repeated/duplicate commands and sustained event traffic | Bounded logical outcome; no second relocation/correction or deadline extension. Actual desktop timing observations supplement T14/T16; deterministic failure injection is not proof of real WM refusal. |
| N14 | Body fully covered/Below or genuinely minimized; invoke from independent terminal/process | Control reachable; truthful success/deferral or bounded failure; never creates duplicate body or changes body XID. |
| N15 | Repeated safe relocation, explicit later Hide/Show/Bring Top, concurrent caller attempt, duration/Quit/kill/relaunch | Existing operations preserve their independent semantics; Busy/no queue; truthful shutdown/unknown results; no stuck pointer/fence, duplicate instance or stale owner reuse. Record kill/timeout as Unknown when no trustworthy terminal arrives. |
| N16 | Passive idle after completed visible/minimized scenes | Event-driven idle remains; no repeated request/shape/geometry activity. Use existing observational practice without adding a permanent watcher to the application. |

N01–N09 and N14 are core native gates, together with feasible N15 regressions. N10–N13 need explicit qualifications where setup cannot safely reproduce the condition; deterministic obligations remain mandatory and the owner/independent reviewer must agree whether any unexercised native case blocks acceptance. H0 does not substitute for any command-level D4 test. Freeze executable identity before this matrix; later evidence-only commits do not create a newly tested executable.

## 12. Risks, review decisions, and stop conditions

No known architecture blocker requires a new controller or protocol revision. The existing proof model can represent standalone Here, including qualified minimized success. The primary risk is integrating that model correctly with the event loop and every cleanup path; the checked desktop helper alone cannot deliver Stage D.

The owner and authoritative reviewer must approve this proposal before D1, specifically:

1. The four-checkpoint split and both production guards remaining closed through D3.
2. Caller capture before owner discovery with bounded count/current revalidation before send; owner-only full Stage-B layout validation, with no protocol expansion.
3. Conservative always-deferred minimized geometry, stronger safe-local no-op predicate, and local unsafe repair using cancellation/one correction without a desktop request.
4. Here-specific observation/routing rather than weakening legacy visibility/layer preservation; exact layout equality and evidence rounds without atomicity claims.
5. The fence-local automatic-correction restriction after an attempted Here correction, so completed failures cannot initiate hidden retries.
6. Proof-aware fatal/shutdown finalization: preserve the existing fatal native-error policy, invalidate current readiness on committed teardown, retain independently confirmed relocation history, and never send Success for a body the owner is already tearing down. Any proposal to keep the owner running after these native errors needs a separate decision; it is not assumed here.

These are concrete proposed implementation choices under the roadmap, not claims of approval. If review finds any incompatible accepted invariant or result encoding, return the specific conflict for an explicit decision. No unresolved request to adopt an external architecture or conduct comparative research is part of D-P.

Stop implementation/acceptance and report if real Openbox relocation steals focus/activation, switches the user's desktop, loses genuine minimization or effective layer, or safe placement needs migration, repeated correction, restoring merely to read geometry, direct property forcing or restacking. Also stop for unverifiable bounded destination evidence, input enabling before complete safety, an extra request owner/queue, inability to retain historical proof truthfully, existing-command regressions, or scope/branch/base/owner-change divergence. Do not defer essential Stage-D safety to Stage F, work around a failed gate, or advance to Recover.

D-P changes only this proposed-plan document. Its verification is required-reading/source consistency review, complete document review, scope/status/remote checks and `git diff --check`; Cargo and native tests are not run for a documentation-only assignment. Publication is a handoff for independent review and owner approval. It neither starts D1 nor accepts Stage D.
