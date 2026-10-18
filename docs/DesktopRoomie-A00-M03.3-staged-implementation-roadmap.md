# DesktopRoomie — A00-M03.3 approved staged implementation roadmap

**Status:** owner-approved P0 architecture and staged implementation contract, 2026-10-18.  
**Repository/branch at approval:** `zNe4/DesktopRoomie` / `m03`.  
**Planning base:** `46adda6a6dc1e25930c9656e7c84d125296c8691` (documentation-only handoff HEAD).  
**Accepted executable foundation:** M03.2 `a46e432c1f97b03c576737b1fc0ac35d7f90c158`, matching CI 39098551853, 453 tests.  
**Authority:** this document freezes design and stage boundaries. It does **not** authorize implementation, commits, pushes, host configuration edits, or advancement between stages. Each mission remains separately owner-authorized and independently reviewed.

## 1. Objective

M03.3 adds same-instance workspace placement without weakening accepted M01/M02/M03.1/M03.2 behavior:

- **Bring Here** moves the existing managed body to the caller/user's frozen current workspace, without changing the user's workspace, stealing focus, changing the effective Above/Normal/Below layer, replacing the body, or silently undoing genuine minimization.
- **Recover** is an explicit composition of Bring Here followed by the accepted Bring Top behavior. It is not a third recovery controller.
- Destination workarea and retained-monitor placement are validated before local input is restored.
- Results distinguish request dispatch, historically confirmed progress and current verified truth.

M03.4 integrated A00 closure and A01 behavior remain out of scope.

## 2. Frozen P0 architecture decisions

| Area | Approved decision |
| --- | --- |
| Destination semantics | Caller captures the current desktop, freezes that exact index into the request identity, and the owner independently validates it before mutation. A later current-desktop change invalidates the frozen request; never retarget silently. |
| Genuine minimization | Bring Here preserves genuine minimization. It neither MapWindows nor removes HIDDEN merely because the body is moved. |
| Sticky/all-desktop body | A body on all desktops is not silently converted to one desktop. Treat that conversion as unsupported unless separately approved. |
| Workarea | Validate destination desktop count and destination workarea strictly. Only a genuinely absent `_NET_WORKAREA` may use selected-monitor bounds as the documented fallback; malformed/empty/truncated values fail. |
| Monitor and geometry | Preserve the startup-selected physical monitor identity. Clamp the existing root origin into valid destination bounds and allow at most one checked correction; no automatic monitor migration or centering policy. |
| Recover ordering | Bring Here first, then the existing Bring Top visibility→Above composition under one active operation. |
| Partial | Partial requires a **meaningful verified recovery transition**: historically confirmed relocation and/or historically confirmed restoration. An already-local no-op, geometry correction, dispatch acknowledgment or pre-existing satisfied state is insufficient by itself. |
| Protocol | Upgrade the private same-X-session control protocol to v2. Reuse the selection, never-mapped endpoint, caller identity/correlation and single active operation; do not add another IPC daemon or queue. |
| Deadlines | Bring Here: 3500 ms owner / 5000 ms caller. Recover: 7000 ms immutable owner total / 9000 ms caller. Existing Hide/Show/Bring Top budgets remain unchanged. Phase deadlines are capped by the immutable total and never extended by events, duplicates or retries. |
| Native evidence | Openbox-specific relocation/minimized/geometry behavior must receive owner-executed native evidence before those implementation stages can be accepted. |

## 3. Result and safety contract

### 3.1 Bring Here success

A non-minimized local final state succeeds only when fresh evidence confirms:

1. body membership on the frozen destination desktop;
2. frozen destination still equals the caller's intended current desktop;
3. preserved effective layer;
4. retained selected monitor and valid destination workarea;
5. actual fixed-size geometry within checked bounds, including confirmation of the one allowed correction when needed;
6. safe original input silhouette restored with all pointer/popup/release obligations settled.

A genuinely minimized final state may also be a **qualified Bring Here success** when fresh evidence confirms destination membership and preserved genuine minimization, the retained monitor/destination area can safely fit the body, and input remains disabled. If Openbox cannot provide trustworthy minimized-body geometry, actual correction may be deferred until later Show/Recover; the result must explicitly report that geometry/input readiness is deferred rather than fabricated.

### 3.2 Recover success and Partial

Recover success requires the complete current postcondition: frozen destination, not minimized, exact Above, valid local placement, input readiness, no focus/activation/user-workspace mutation and the same body instance.

If complete success cannot be established, Partial is valid only when the operation previously **confirmed** at least one meaningful transition:

- the body began on another desktop and relocation to the frozen destination was confirmed; and/or
- the body began genuinely minimized and restoration to not-minimized was confirmed.

An already-local Bring Here no-op does not create Partial authority. A geometry correction alone does not create Partial authority. A sent request with unknown WM outcome does not create Partial authority.

Historical proof must never be presented as current truth. If later evidence becomes unreadable or the layout changes, current confirmation bits are cleared while diagnostics may state what was confirmed earlier.

Caller timeout/owner disappearance before a trustworthy terminal reply remains OutcomeUnknown even if the owner may have progressed.

### 3.3 No rollback or retry controller

Do not implement automatic rollback after a Partial or failure. A rollback would itself be another asynchronous WM mutation and could undo legitimate user action. Do not blindly retry uncertain desktop, visibility, geometry or layer mutations.

## 4. Common non-regression obligations

Every stage preserves:

- one managed procedural body and one cooperative control owner;
- one active remote operation, no queue and no completed-request replay service;
- one existing `LayerController`; Recover reuses it rather than constructing another layer state machine;
- no keyboard grab, `SetInputFocus`, activation message, global key listener, `_NET_CURRENT_DESKTOP` request or direct WM state/property rewrite;
- M02 pointer ownership, checked release debt, cancellation ordering, stale-input suppression and outside-popup no-replay semantics;
- exact click-through silhouette and post-map synchronization;
- selected-monitor identity, checked root/local coordinates, fixed body size, representable bounds and honest live geometry;
- event-driven idle behavior and bounded operation deadlines;
- checked cleanup with original-error priority;
- exact executable-SHA / CI / owner-native evidence separation.

## 5. Stage A — protocol v2 and pure operation foundations

**Status — ACCEPTED 2026-10-18:** corrected executable `d7727b3abf9e2c047e389f13a9c5cac19f9ed1a8`; matching [CI 39152686625](https://github.com/zNe4/DesktopRoomie/actions/runs/39152686625), 488 passed / 0 failed; independent ChatGPT source review and explicit owner acceptance. First candidate `10f49920…` remains historical failed-review evidence because its terminal-result partition was too permissive. This acceptance does not authorize Stage B.

**Purpose:** make the new commands and proof model representable without enabling native workspace relocation.

Primary expected files: `src/control.rs`, `src/x11/control.rs`, narrowly `src/main.rs` parser/help/tests.

Required work:

- add `BringHere = 4` and `Recover = 5`;
- move private owner/caller/terminal protocol identity to VERSION 2;
- keep the five-word request and terminal ClientMessages, but extend the immutable caller marker to carry the frozen destination desktop;
- existing Hide/Show/Bring Top require a reserved no-destination sentinel; Bring Here/Recover require an ordinary desktop index and reject the sticky `u32::MAX` sentinel;
- add pure placement and Recover progress to the existing single `ActiveOperation`;
- represent historically confirmed relocation/restoration separately from current confirmation;
- validate qualified minimized Bring Here success;
- encode the approved 3500/5000 and 7000/9000 deadlines;
- leave Bring Here/Recover unsupported at the native execution boundary.

Required tests include complete codec round trips, mixed-v1/v2 rejection, reserved/unknown fields, destination-marker correlation, every command/lifecycle/status/reason combination, deadline immutability, single-operation admission and all legal/illegal Success/Partial proof cross-products.

**Gate — satisfied:** matching v2 pure/protocol behavior and Stage-A execution guards passed source review and CI at `d7727b3…`; no native workspace mutation exists. **Next:** Stage B only under a separate owner assignment.

**Recommended Codex:** GPT-6, High reasoning.

## 6. Stage B — strict destination/layout evidence

**Purpose:** create a read-only destination model that can prove whether a frozen target is safe and still current.

Primary expected files: `src/x11/monitors.rs`, `src/x11/visibility.rs`, possibly a small pure placement helper.

Add strict reads/validation for:

- `_NET_NUMBER_OF_DESKTOPS` as exactly one nonzero CARDINAL;
- frozen target strictly below that count;
- `_NET_WORKAREA` complete and structurally consistent with the advertised desktop count;
- selected target rectangle with positive checked dimensions;
- selected-monitor intersection and fixed-body valid origin bounds;
- `_NET_DESKTOP_VIEWPORT` compatibility, initially rejecting unsupported/nonzero large-desktop behavior rather than guessing;
- retained monitor identity and geometry;
- repeated-read agreement/freshness sufficient for operation evidence, without claiming atomicity.

A changed count, target workarea, viewport, retained monitor or frozen-current-desktop relationship invalidates the operation; do not silently recompute a different target.

**Gate:** deterministic/injected tests cover missing, malformed, truncated, overflow, too-small, stale and changing destination evidence. No state-changing workspace request exists.

**Recommended Codex:** GPT-6, High reasoning.

## 7. H0 — owner-native Openbox feasibility gate

This is an evidence checkpoint, not a coding stage. Before standalone relocation is accepted, owner-run reversible tests on DesktopRoomie's own body must establish:

| Gate | Required Openbox evidence |
| --- | --- |
| H0-1 | Exact WM-directed `_NET_WM_DESKTOP` relocation can move the managed UTILITY body without activation, focus theft or changing `_NET_CURRENT_DESKTOP`. |
| H0-2 | Normal/Above/Below effective layer behavior across relocation is observed and recordable. |
| H0-3 | Genuine HIDDEN minimization is preserved when a minimized body is relocated. |
| H0-4 | Actual PropertyNotify/MapNotify/UnmapNotify/WM_STATE ordering and settled representation are recorded. |
| H0-5 | Determine whether minimized-body root geometry is trustworthy or must remain deferred. |
| H0-6 | Real `_NET_NUMBER_OF_DESKTOPS`, `_NET_WORKAREA` and viewport data fit the Stage-B model. |

Start with passive snapshots. Any state-changing experiment requires explicit owner authorization, before/after evidence, real typing-focus observation and restoration instructions.

**Stop:** return to architecture review if Openbox requires focus/workspace switching, loses genuine minimization, cannot support safe destination evidence, or requires property-forcing/WM-fighting workarounds.

## 8. Stage C — checked WM-directed relocation primitive

**Purpose:** implement one narrow native request seam, not a new controller.

A helper may live in a small `src/x11/desktop.rs` if that keeps responsibilities clear. It sends exactly one root-directed `_NET_WM_DESKTOP` ClientMessage for an admitted frozen destination:

- destination root;
- body as event window;
- `SUBSTRUCTURE_NOTIFY | SUBSTRUCTURE_REDIRECT`;
- data[0] = frozen destination;
- data[1] = source indication 1;
- remaining words zero.

It must check its logical deadline before mutation, mark possible dispatch before checked completion, distinguish X acknowledgement from WM state confirmation, and never retry an uncertain request. It never writes `_NET_WM_DESKTOP` directly and never sends `_NET_CURRENT_DESKTOP`, activation or focus requests.

**Gate:** exact envelope and deadline/dispatch/error tests; no command-level success is inferred from transport acknowledgement.

**Recommended Codex:** GPT-6, Medium/High reasoning.

## 9. Stage D — standalone Bring Here

**Purpose:** enable `--bring-here` through the accepted same-instance event loop.

Recommended phase order:

1. admit and validate immutable caller destination;
2. observe original body desktop, minimization, layer and retained-monitor placement;
3. avoid fencing/mutation for a fully safe already-local no-op;
4. when relocation/correction is needed, settle the accepted input fence, gesture, popup and release obligations;
5. revalidate frozen layout before the single desktop request;
6. confirm actual destination membership and preserved minimization/layer;
7. validate destination geometry; use at most one checked clamp correction when applicable;
8. restore exact input only after full local readiness, or return qualified minimized success with input disabled/deferred;
9. perform final fresh evidence validation and issue one correlated terminal result.

Bring Here never Show-restores a minimized body and never forces Above.

Partial requires historically confirmed actual relocation, not a no-op or correction.

**Gate:** deterministic tests plus owner Openbox local/remote visible/minimized and Normal/Above/Below cases; no focus theft/user-workspace switch; retained monitor, click-through and input safety preserved.

**Recommended Codex:** GPT-6, High reasoning.

## 10. Stage E — composed Recover

**Purpose:** enable `--recover` as one operation: Bring Here → accepted Bring Top.

Do not invoke a second CLI/process/request and do not admit a nested active operation. Reuse the existing Show/geometry readiness and single absolute Above controller after the Here phase succeeds.

Required deadline policy:

- immutable owner total: 7000 ms from Recover admission;
- Here subphase: at most its 3500 ms budget and never beyond the total;
- Top phase: accepted 2500 ms visibility cutoff plus at most 1000 ms layer work, capped by the remaining immutable total;
- caller total: 9000 ms from before discovery;
- duplicate requests/events/phase transitions never renew a deadline.

Existing helpers that branch on `Command::BringTop` must be made phase-aware narrowly enough for Recover; do not add another scheduler.

A current-desktop/layout change invalidates the frozen Recover destination and prevents further recovery mutation. Final Success needs a fresh complete postcondition. Partial follows the verified relocation/restoration proof rule.

**Gate:** already-local and remote, visible and genuinely minimized, Normal/Below/Above inputs; Phase-A failure never starts Phase B; truthful Failed/Partial/Unknown results.

**Recommended Codex:** GPT-6, High reasoning.

## 11. Stage F — lifecycle, concurrency and deadline hardening

**Purpose:** no new behavior; exhaustively harden the integrated operations.

Required coverage:

- concurrent Hide/Show/Bring Top/Bring Here/Recover and local layer requests;
- duplicate/stale requests, owner epoch change, caller death and replacement owner;
- owner Quit, duration expiry, selection loss and body destruction at each phase boundary;
- target/current/count/workarea/viewport/monitor drift before and after relocation;
- late property/map/configure events after terminal or failed operations;
- checked native errors with possible dispatch;
- input-release or popup cleanup failure;
- sustained >64-event traffic across fixed deadlines;
- terminal reply loss and caller OutcomeUnknown;
- full M03.2 deterministic regressions.

**Gate:** no queue, duplicate body, stuck grab/input fence, invalid Success/Partial, retry loop, deadline renewal or M03.2 regression.

**Recommended Codex:** GPT-6, High reasoning.

## 12. Stage G — integrated M03.3 acceptance

Freeze an exact executable SHA and perform:

1. complete local fmt/build/Clippy/tests;
2. independent source/diff review against the approved roadmap and invariants;
3. matching clean-environment CI;
4. owner-native Openbox acceptance on that exact executable;
5. documentation of actual observations, qualifications and unexercised cases;
6. explicit owner acceptance.

Suggested native matrix includes: remote Normal/Above/Below Bring Here, local no-op, off-workspace Iconic/no-HIDDEN representation, genuine minimized relocation, visible/minimized Recover, dual-monitor retained identity, differing destination workarea, continuous typing, drag/popup interruption, covered/minimized CLI control, repeated/concurrent requests, Quit/expiry/kill/relaunch and safely reproducible destination/workarea drift.

Injected malformed/failure tests remain distinct from native evidence.

Stage G closes **M03.3 only**. It does not perform M03.4 integrated A00 architecture closure.

## 13. Expected source ownership

| File/module | M03.3 responsibility |
| --- | --- |
| `control.rs` | v2 protocol values, frozen target, placement/Recover pure progress, proof/result validation and deadlines |
| `main.rs` | CLI, single active operation progression, input fence, Bring Here phases and Recover composition |
| `x11/control.rs` | destination capture/marker, v2 identity/correlation, caller budget and diagnostics |
| `x11/monitors.rs` | desktop count, destination workarea/viewport, retained monitor and checked fit |
| `x11/visibility.rs` | fresh body desktop/minimization/layer evidence |
| `x11/window.rs` | generalize the existing one-shot checked correction narrowly if needed |
| `x11/state.rs` | existing layer/support observations; no duplicate layer controller |
| `layer.rs` | ideally unchanged |
| optional `x11/desktop.rs` | thin checked `_NET_WM_DESKTOP` transport only |

Do not broadly refactor `main.rs` or introduce a generic platform/service architecture merely for future reuse.

## 14. Stop conditions

Stop and return for owner/ChatGPT architecture review if:

- real Openbox relocation steals focus, activates the body or changes the user's current desktop;
- genuine minimization cannot be preserved under the approved contract;
- safe placement requires monitor migration, repeated correction, forced property writes or restoring a body merely to learn geometry;
- destination/workarea/viewport evidence cannot support truthful bounded confirmation;
- input could become active before geometry and pointer-release obligations are settled;
- truthful Recover composition requires another controller/queue;
- an implementation stage silently enters M03.4/A01;
- branch/base identity, unrelated owner changes, CI or accepted M03.2 behavior diverges unexpectedly.

## 15. Publication and evidence discipline

Every implementation stage is separately authorized. Before publication inspect branch, base SHA, status/diff and intended paths. Ordinary non-force push to `m03` only when the assignment explicitly authorizes it. Never merge, rewrite history, force-push, or treat CI as owner acceptance.

Record executable code SHA separately from later documentation-only commits. Preserve historical failed/conditional M03.2 evidence rather than rewriting it.

## 16. Immediate next gate

**A00-M03.3-A — protocol v2 and pure operation foundations.**

Starting branch for the first implementation prompt is whatever exact `m03` documentation HEAD contains this approved roadmap. The mission must name that SHA explicitly before Codex edits.

Stage A enables no native workspace movement. After its implementation, matching CI and independent review, the owner decides whether to accept A and authorize Stage B.
