# DesktopRoomie A00-M03.2-G — approved implementation plan

**Later status (2026-10-18):** A00-M03.2 received formal final owner H-A acceptance on the separate H executable after H-D documentation publication. This G1/G2/G3 plan remains historical and does not automatically approve M03.3.

**Current outcome (owner-accepted 2026-10-13): STAGE G ACCEPTED.** This document preserves the originally read-only Stage-G plan and owner-approved D1/D2 decisions as **historical planning authority**. References below to G1 next, readiness for implementation or Bring Top Unsupported describe the original planning checkpoint rather than current behavior. Independently reviewed G1 `ba159932d3245fae438a37896cc68623c3dc75b3` (367 tests), G2 `96de45d3a236a2cdedb1ec74d1a855470b86fa9a` (414 tests), and owner-accepted G3 `80d7df4aa1524122bd7c232f5aa09a4e39c8dd9f` ([CI 38468930784](https://github.com/zNe4/DesktopRoomie/actions/runs/38468930784), 447 passed/0 failed) implemented the plan. [Stage-G native evidence](host/OPENBOX.md#stage-g-acceptance-recorded-2026-10-13) covers H10/H11/H13/H16/H17/H20/H22/H25, H14 not observed. Bring Top is now supported on the existing body workspace. **H-N1/N2/N3 were subsequently owner-native tested on separate frozen H executable `a46e432…` with qualifications in [integrated acceptance](../experiments/desktop-probe/ACCEPTANCE-M03.2.md); final H-A owner acceptance is pending.**

**Historical planning status (2026-10-13 before G implementation):** READY FOR BOUNDED IMPLEMENTATION. A–F were the accepted foundation at `ccd01915bef13850067391f70f1524362a6440dc`. The original document specified separately authorized G1/G2/G3, with Bring Top then Unsupported (exit 7).

**Checkpoint decomposition:** G1 = bounded/structured layer adapter while Bring Top stays Unsupported; G2 = visibility→layer composition and CLI exposure using the approved timing; G3 = scheduler/concurrency/failure hardening before native acceptance.

---

Stage G can reuse the accepted visibility machinery and the existing M03.1 layer controller. The required additions are composition bookkeeping, structured layer outcomes, a narrowly scoped remote layer reader, and phase-aware event routing.

The owner approved the recommended deadline amendments on 2026-10-13: a 3500 ms Bring Top owner deadline and a command-specific 5000 ms Bring Top caller deadline. This plan is therefore READY FOR IMPLEMENTATION through separately authorized G1/G2/G3 checkpoints; the accepted executable still returns Unsupported for Bring Top until G is implemented.

Read-only inspection confirmed:

- Branch: m03.
- HEAD: 6d693df5dc55de6ec9f411fc615ec84dde5f1faa.
- Clean working tree.
- No differences from accepted executable ccd01915bef13850067391f70f1524362a6440dc in source, Cargo files, or CI definitions.
- No files edited, commands implemented, commits made, pushes performed, or host configuration changed.
- Required documents and relevant source/tests inspected. No Cargo tests or desktop experiments run; no new host acceptance claimed.

1. Current source ownership

| Source | Current responsibility | Stage-G implication |
|---|---|---|
| control.rs (experiments/desktop-probe/src/control.rs:355) | ActiveOperation, admission, fixed owner deadline, ShowProgress, HideProgress, baselines, results and codecs | Retain the single remote operation. Add only G composition/progress facts. |
| main.rs — visibility orchestration (experiments/desktop-probe/src/main.rs:1963) | VisibilityHost, Show/Hide progression, geometry/input completion, preservation checks | Extract a reusable visibility-stage outcome; standalone Show retains its accepted behavior. |
| main.rs — layer orchestration (experiments/desktop-probe/src/main.rs:1681) | Reads, support checks, checked sends, immediate rereads, deadline progression | Return structured outcomes instead of relying on logs and Ok(()). |
| layer.rs (experiments/desktop-probe/src/layer.rs:87) | One pure absolute layer controller | Retain its transition algorithm; add an outer-deadline cap. |
| x11/state.rs (experiments/desktop-probe/src/x11/state.rs:68) | Viewable-only local layer reader, strict decoding, EWMH envelopes and checked sends | Keep local reader unchanged; add remote eligibility and dispatch evidence support. |
| x11/visibility.rs (experiments/desktop-probe/src/x11/visibility.rs:49) | Complete, repeated ICCCM/EWMH observations; local restore and off-workspace REMOVE-HIDDEN | Already supplies the remote visibility and layer evidence G needs. Reuse its native operations. |
| x11/control.rs (experiments/desktop-probe/src/x11/control.rs:916) | Admission routing, active-record ownership, one terminal response, caller deadline/diagnostics | Admit Bring Top asynchronously; add diagnostics and G-aware shutdown completion. |

ProbeRuntime.layers remains the sole layer controller. ControlWindow.active remains the sole remote request lifetime owner. Neither should contain a second copy of the other’s state machine.

The source confirms several important details:

- Bring Top is currently admitted only long enough to return Failed / Unsupported / Preflight.
- service_layer_with already performs a fresh read after every checked mutation.
- Its recoverable outcomes are largely log-only.
- LayerAtoms::read_body rejects every non-Viewable body.
- VisibilitySnapshot.layer already supports coherent off-workspace observations without that restriction.
- Active visibility routing currently consumes body ConfigureNotify and layer-property events. That routing cannot be carried unchanged into G’s layer phase.
- The loop’s standalone layer-deadline service currently uses the local reader. Remote G must bypass that path.

2. Exact Bring Top contract

Bring Top operates on the existing managed body:

1. Establish that it is not minimized.
2. Request and freshly confirm exact effective Above.
3. Preserve both the body’s original desktop value and the user’s original current-desktop value.

Successful terminal evidence must establish:

- Same body.
- Coherent not-minimized classification.
- ABOVE present and BELOW absent.
- Unchanged workspace values.
- For current/all-desktop success: accepted local placement/input readiness.
- For off-workspace success: placement deferred and local input unavailable.

“Above” means the EWMH effective layer. It does not mean relative raising within Normal, visibility through occluding fullscreen windows, or visibility on the caller’s workspace.

No activation, focus request, workspace transport, MapRaised, generic restacking, second body, or continuous reassertion is permitted.

3. Proposed phase/state composition

Use the existing ActiveOperation.stage for orchestration:

Preflight
    ↓
Show / Geometry          shared visibility stage
    ↓
Layer                    existing LayerController, target Above
    ↓
one terminal response → active record cleared

Geometry remains a diagnostic substage of visibility preparation. Terminal state need not persist after the response attempt.

Reuse ShowProgress for visibility phases, correction state, observation scheduling and baseline. Add a small owner-only BringTopProgress containing:

- Fixed visibility-stage cutoff.
- Whether a minimized starting state was established.
- Historical proof that restoration was freshly confirmed.
- Layer request attempt/dispatch/check facts needed for reporting.
- Whether opposite-flag removal was freshly observed.
- A layer-phase observation/service flag.

Do not copy PendingLayer.phase, target or deadline into this struct. Those remain owned by LayerController.

The baseline remains in one place: operation.show.baseline. Its desktop values remain immutable throughout G. Its initial layer is diagnostic information; G does not impose Show’s unchanged-layer invariant.

Two distinct boundaries are necessary:

- Restoration confirmed: fresh evidence proves minimized → not minimized on the preserved workspace. Record this immediately, even if subsequent geometry/input completion fails.
- Visibility stage ready: local geometry and input restoration have completed, or off-workspace confirmation and settled fencing permit deferred readiness. Only then may layer work begin.

A successful visibility-stage outcome must be consumed internally. It must not send a Show response, release admission, or admit another caller between stages.

4. Current/all-desktop decision table

The minimized rows describe the starting layer. At the composition boundary, use the freshly observed layer; never reassert a cached desired value.

| Starting state | Visibility work | Layer work | Successful result |
|---|---|---|---|
| Ready, Normal | Fresh verification; preserve interaction | ADD ABOVE | Not minimized + Above here |
| Ready, Below | Fresh verification; preserve interaction | REMOVE BELOW → fresh read → ADD ABOVE | Same |
| Ready, Above | Fresh verification | None | Already Above here |
| Minimized, Normal | Accepted fence → user-time zero → checked MapWindow → local readiness | Above transition from fresh layer | Restored + Above here |
| Minimized, Below | Same | Normally serialized removal/addition | Same |
| Minimized, Above | Same | Freshly verify Above; mutate only if needed | Restored; Above confirmed |
| Restoration requires correction | Existing one-shot correction and 150 ms confirmation before input enable | Begins only after readiness | Success only after both stages |
| Visibility confirmation fails | No layer request | None | Failed unless restoration was already positively confirmed |
| Restoration confirmed, input/geometry fails | Preserve safety; no layer request | None | Partial; geometry/input failure reported |
| Visibility stage succeeds, layer fails | No restoration retry or rollback | Stop | Partial if restoration occurred; otherwise Failed |

“All desktops” follows the local path only with coherent local Viewable evidence. Preserve the body’s u32::MAX desktop value and the observed current-desktop baseline.

The no-mutation success case assumes existing readiness is already valid. An Above flag alone must not bypass an unresolved input fence or geometry obligation.

5. Off-workspace decision table

| Starting state | Visibility mutation | Layer work | Terminal location |
|---|---|---|---|
| Not minimized, Normal | None | ADD ABOVE | Another workspace |
| Not minimized, Below | None | REMOVE BELOW → fresh confirmation → ADD ABOVE | Another workspace |
| Not minimized, Above | None | Fresh no-op confirmation | Already Above elsewhere |
| Minimized, Normal | Checked REMOVE-HIDDEN once | Above from fresh state | Restored + Above elsewhere |
| Minimized, Below | Checked REMOVE-HIDDEN once | Normally remove Below, then add Above | Same |
| Minimized, Above | Checked REMOVE-HIDDEN once | Verify retained Above; no redundant addition | Same |

Eligible not-minimized representations are:

- Iconic + Unmapped + no HIDDEN + Other.
- Normal + Unmapped/Unviewable + no HIDDEN + Other.

The first is recorded Openbox host evidence. The second remains an approved coherent deterministic variant unless actually observed on the host.

For every off-workspace row:

- No MapWindow.
- No user-time write.
- No workspace mutation.
- No caller-workarea lookup or placement correction.
- No local input enable.
- Retain or establish accepted safe unavailable-input state as necessary.
- Later ordinary local return uses existing deferred placement/input restoration.

6. Layer-observation eligibility

Add a dedicated remote adapter around the existing complete visibility observation.

A remote layer observation is eligible only when:

1. The body still exists and has coherent managed ICCCM evidence.
2. Full state/support/workspace properties are readable and valid.
3. Repeated observation agrees.
4. Both desktop values match the operation baseline.
5. Visibility is NotMinimizedHere or NotMinimizedElsewhere.
6. Layer is exactly Normal, Above or Below.

For local completion, also require accepted readiness. For elsewhere, require the narrow off-workspace classifications above.

Complete _NET_WM_STATE supplies sufficient property-layer evidence on an eligible managed off-workspace body. It does not prove visual stacking or identify who caused the state change. Real Openbox G acceptance must establish that the requested transition works there.

Keep these cases separate:

| Evidence | Remote handling |
|---|---|
| Coherent eligible snapshot, Above only | May confirm Above |
| ABOVE + BELOW | Unverifiable; never success; do not attempt remote conflict repair |
| Coherent snapshot with unknown/malformed layer evidence | Fail Unverifiable |
| Missing required advertised support | Fail Unsupported |
| Noncoherent/transitional snapshot | No mutation; await relevant evidence within the fixed deadline |
| Minimized/Withdrawn/Destroyed during layer phase | Abort composition; do not restart restoration |

The local menu retains its current Viewable reader, cleanup requirements and runtime eligibility. No global relaxation is needed.

7. M03.1 reuse and structured outcomes

Reuse unchanged:

- Layer, ObservedLayer, Flags, Mutation, Phase, PendingLayer, Step.
- Absolute target selection.
- Serialized opposite-flag removal.
- Fresh-read confirmation.
- No rollback/retry/reassertion behavior.
- LayerAtoms::request and its exact root-directed EWMH envelopes.
- Support decoding and LayerSupport::allows.

Extend LayerController with a bounded entry point:

begin_bounded(target, observed, accepted, outer_deadline)

pending deadline = min(accepted + 1000 ms, outer_deadline)

Existing local begin retains its one-second behavior.

Adapt the existing service_layer_with orchestration to return a small structured outcome such as:

- AlreadyMatches.
- Waiting.
- Confirmed.
- Unsupported.
- Unverifiable.
- TimedOut, retaining controller phase.
- Interrupted.
- Native error through its existing error channel.

Track request attempts, actual dispatch and checked acknowledgement separately. LayerAtoms::send currently returns only Result<()>; a marked-send variant should set dispatch evidence after obtaining the send cookie and before checking it. Keep the current local wrapper and request envelope intact.

For Below → Above:

1. Preflight support for the complete planned transition.
2. Send REMOVE BELOW once.
3. Freshly observe removal.
4. Revalidate support and remote eligibility.
5. Send ADD ABOVE once.
6. Freshly confirm exact Above.

If fresh observation already reaches Above after removal, finish without redundant ADD.

There is no positive WM “refused” acknowledgement in this path. A valid unchanged state through expiry is confirmation timeout, possibly consistent with refusal—not proof of an explicit rejection.

During visibility preparation, coherent layer changes are allowed because Above is the target. During layer work, ordinary controller-compatible intermediate states remain valid; incompatible changes, such as Below reappearing after confirmed removal, terminate as LayerChanged. Conflict terminates as Unverifiable. Neither causes a new transition attempt.

8. Stage E/F reuse

The smallest safe extraction is a shared visibility-stage function returning:

Pending
Ready(visibility/readiness evidence)
Failed(reason/evidence)
native error

Standalone Show wraps this in its current terminal-result behavior. Bring Top consumes Ready and continues.

Use an explicit policy difference:

- Show: preserve baseline layer.
- Bring Top: preserve workspace, require readable nonconflicting layer, permit coherent layer changes.

Do not temporarily rewrite operation.request.command to Show, manufacture a second operation, or overwrite the owner deadline.

Reuse unchanged native restoration primitives:

- Local: checked user-time zero, then checked MapWindow.
- Elsewhere: checked REMOVE-HIDDEN.
- Existing fencing, correction, SHAPE restoration and deferred-return machinery.

The shared visibility function must expose restoration evidence before local readiness completes. The current show_result helper cannot serve as G’s final composer: it hardcodes Show and ties local visibility confirmation to input readiness.

A later layer refusal must not run Show’s failure refencing merely because the overall composite failed. Refence only when visibility/input safety actually requires it.

9. Busy and concurrency

| Situation | Policy |
|---|---|
| Local layer pending when Bring Top arrives | Busy before remote effects; preserve local operation |
| Hide/Show active when Bring Top arrives | Busy |
| Bring Top active when Hide/Show arrives | Busy |
| Different Bring Top caller while G active | Busy |
| Exact duplicate of active request | Ignore; preserve original deadline and single terminal reply |
| Local layer selection during G visibility phase | Existing cancellation/fence may invalidate the gesture; otherwise complete popup cleanup, then reject Busy |
| Local layer selection during G layer phase | Release pointer and destroy selecting popup, then reject Busy |
| Later valid drag or non-layer popup interaction | Allowed; must not stall pending layer work |
| Pending local bounds correction at admission | Busy before G effects |

A fast completed command may allow the next caller to be admitted normally. Do not manufacture Busy after completion.

No queue, completed-request cache, resend, timeout cancellation, or exactly-once guarantee for post-completion replay.

10. Input and pointer policy

For an already-ready local body, layer-only Bring Top leaves gesture state, capture, popup, input shape and movement ownership untouched. It uses the same independence that accepted local layer operations already have.

When restoration or unavailable-input recovery is required, reuse the accepted fence:

1. Checked empty input SHAPE.
2. Invalidate gestures, menu actions and pending movement.
3. Settle explicit and possible automatic pointer capture.
4. Attempt popup cleanup even after release failure.
5. Dispatch restoration only after obligations settle.

Local geometry correction and exact silhouette restoration must finish before layer mutation. Off-workspace restoration keeps input unavailable.

If not-minimized restoration is confirmed but input enabling fails:

- Do not start Above.
- Retain restoration progress.
- Report Partial with input_ready=false.
- Perform accepted one-time safety refencing.
- Preserve the original fatal native error and cleanup priority.

No retry of enabling, restoration or layer mutation follows.

11. Terminal semantics

Use a G-specific result composer; do not infer Partial from the aggregate dispatch bit.

| Status | Exact meaning | Exit |
|---|---|---:|
| Success | Fresh final evidence proves not minimized, exact Above, preserved workspaces and appropriate readiness | 0 |
| Partial | Actual restoration was positively observed during this operation, but the complete Bring Top contract failed | 9 |
| Failed | Complete contract failed and no restoration progress was positively established | 7 |
| Busy | Operation could not proceed because another owner/placement obligation was active; this request performs no restoration/layer work | 5 |

“Failed” does not assert that dispatched requests had no effect.

For G, a partly completed layer transition alone is not terminal Partial. This follows the approved G distinction between “restored, then layer failed” and “no restoration, layer failed.”

Required examples:

| Example | Result |
|---|---|
| Already not minimized and freshly Above, with valid readiness | Success, no restoration/layer mutation |
| Already visible Normal; Above confirmed | Success |
| Minimized Normal; restored; Above confirmed | Success |
| Minimized; restoration never confirmed | Failed; zero Above requests |
| Minimized Normal; restored; Above never confirmed | Partial; not minimized confirmed, Above false |
| Off-workspace minimized; REMOVE-HIDDEN confirmed; Above fails | Partial; explicitly elsewhere |
| Already visible Normal; Above fails | Failed, even though not-minimized was already true |
| Already visible Below; Below removal confirmed; Above addition fails | Failed; report fresh Normal if verified, otherwise Unknown |
| Minimized Above; restored; Above remains | Success without layer mutation |
| Restoration confirmed; later body destruction or evidence loss | Partial describing earlier restoration and current failure; never retain stale current-confirmation bits |

Historical progress is not a rollback promise. A later change can invalidate the current state without erasing what was previously confirmed.

12. Result, wire and CLI changes

The existing v1 response can express the required outcomes without new bits or messages. Retain its layout, enum values and reserved-zero bits.

However, the internal result validation needs a narrow extension.

Today OperationResult::new accepts Partial when a mutation was dispatched and either visibility or Above is confirmed. That is:

- Too permissive for an already-visible body whose layer request fails.
- Too restrictive for preserving proven restoration history after subsequent destruction or loss of current evidence.

Add an owner-only restoration-proof value to a G-specific constructor/composer. It must establish a genuinely minimized starting observation and a later freshly confirmed restoration in the operation’s restoration path. A dispatch bit, candidate snapshot, or user-time write alone is insufficient.

For Bring Top, validate Partial using that proof. Keep other commands’ behavior unchanged.

No current field acquires a historical meaning:

| Field | G meaning |
|---|---|
| Confirmation.visibility | Fresh final coherent visibility, or None if unavailable; independent of local input readiness on unsuccessful results |
| Confirmation.above | Fresh exact Above under valid remote eligibility |
| not_minimized_confirmed | Mirrors current visibility confirmation |
| above_confirmed | Mirrors current Above confirmation |
| placement_deferred | Qualified off-workspace state; no local readiness claim |
| input_ready | Actual accepted local geometry/input readiness; not merely mapped |
| host_mutation_dispatched | Existing command dispatch evidence, extended to include layer sends; never proof of restoration or success |
| Detail.layer | Latest valid terminal layer observation, not cached controller desire |
| stage | Preflight, Show, Geometry, Layer or Shutdown where completion/failure occurred |
| reason | Existing None, Unsupported, Unverifiable, ConfirmationTimeout, LayoutChanged, LayerChanged, ReleaseFailed, X11Failure, etc. |

Partial itself tells a G caller that restoration progress occurred. If current evidence subsequently becomes unavailable, diagnostics must say “restoration was confirmed earlier; current state is unverified”, not “not minimized confirmed now.”

Suggested CLI messages:

bring-top: not minimized + Above confirmed here; original workspace preserved
bring-top: not minimized + Above confirmed on another workspace;
           placement deferred, input unavailable

bring-top: already not minimized + Above here; no restoration/layer request needed
bring-top: already not minimized + Above on another workspace

bring-top: partial — restoration confirmed here, but Above confirmation timed out;
           current effective layer: Normal

bring-top: partial — restoration confirmed on another workspace, but Above failed;
           original workspace preserved

bring-top: failed — visibility confirmation timed out before layer work
bring-top: failed — Above unconfirmed; current effective layer: Normal
bring-top: Busy — another remote or local layer operation is active
bring-top: failed — layer evidence is unsupported/unverifiable

Do not derive “restored” from the aggregate dispatch bit. For successful commands with mutations, “not minimized + Above confirmed” is truthful without adding wire fields to distinguish every internal action.

x11/control.rs changes are required even without a wire change: asynchronous Bring Top admission, caller diagnostics/timing and shutdown handling.

13. Deadline analysis and recommendation

| Option | Safety and reuse | Margin/caller interaction | Complexity and guarantees |
|---|---|---|---|
| A: Keep total 2000 ms; visibility approximately first 1000 ms | Can preserve ordering and safe failure, but cannot offer the full accepted Show budget inside G | More restoration/readiness timeouts on slower hosts; ample caller margin | Small implementation; standalone Show unchanged, but G visibility is deliberately less tolerant |
| B: Owner 3500 ms, visibility up to 2500 ms, layer up to 1000 ms | Preserves accepted visibility budget and full bounded layer behavior | Only 500 ms remains under the current caller budget for all pre-admission and reply overhead | Simple fixed outer deadline with component caps; no weakened Show behavior |
| C: Fit a chosen total below 4000 ms, cap components by remaining time | Safe if caps are enforced; can preserve full visibility budget | A larger transport reserve necessarily reduces worst-case layer time; a larger owner budget reduces transport reserve | More timing tradeoffs; cannot infer caller’s remaining time from current protocol |

Recommend B, with these exact rules:

t0 = owner admission

owner deadline      = t0 + 3500 ms
visibility cutoff   = t0 + 2500 ms
layer deadline      = min(layer admission + 1000 ms, owner deadline)
geometry deadline   = existing 150 ms confirmation,
                      entirely within visibility cutoff

The visibility stage starts immediately; preflight/readiness work consumes its budget. A phase transition never extends the owner deadline.

The caller currently starts its four-second clock before atom/selection discovery, performs repeated discovery and endpoint validation, then sends. The owner starts its clock later, at admission. Consequently, 3500 ms owner work fits a 4000 ms caller only when total pre-admission and return overhead stays below roughly 500 ms.

There is no measured or enforced 500 ms bound in the current source. Four seconds may work normally, but this inspection cannot establish it as sufficient for the slower-host envelope.

**Owner-approved D1 (2026-10-13):** Bring Top owner deadline 2000 → 3500 ms, with the component rules above.

**Owner-approved D2 (2026-10-13):** Bring Top caller deadline 4000 → 5000 ms. Retain 4000 ms for Hide/Show. This restores a nominal 1500 ms non-owner margin, matching the current Show relationship.

Five seconds still does not impose hard timeouts on synchronous X11 calls. Preserve the accepted responsive-server limitation and OutcomeUnknown behavior.

For every option:

- No event or duplicate extends a deadline.
- No visibility or layer retry.
- No new operation mutation at/after its cutoff.
- Final fresh reads may confirm already-achieved state.
- Safety cleanup remains mandatory even after operation expiry.
- At total expiry between stages, a read may establish that Above is already true; otherwise terminate Failed/Partial without starting a mutation.

14. Event-loop progression

Extend the existing owner service to progress the composition, rather than adding another loop.

Required service points remain:

1. Top of the outer loop.
2. Immediately after a consumed control request at the batch routing site.
3. Immediately after a consumed control request at lookahead.
4. Deadline checks during event traffic.
5. After the bounded batch and before socket wait.

When visibility returns Ready, the same service call must enter Layer and invoke existing bounded layer orchestration immediately. That orchestration already rereads after each checked mutation, so an immediately completed transition can finish without another event.

If still pending, relevant notifications or the earliest deadline drive further observation.

The wait deadline becomes the earliest applicable value among:

- Overall remote deadline.
- Visibility-stage cutoff while that phase is active.
- Show geometry confirmation.
- Existing layer controller deadline.
- Existing movement, safety, correction and duration deadlines.

Do not keep an expired visibility cutoff active after entering Layer.

Routing must distinguish phases:

| Phase | Event handling |
|---|---|
| Visibility/Geometry | Retain E/F’s ownership of expected visibility and geometry events |
| Layer | Route layer/support/visibility evidence to the remote adapter; allow ordinary gesture and movement-confirmation handling |
| No remote operation | Existing local routes |

Specific safeguards:

- Skip standalone local service_layer deadline handling while G owns Layer.
- Do not let local Viewable-only property handling observe remote pending work.
- Do not blanket-consume body ConfigureNotify during Layer; that would starve drag confirmation.
- Expected or delayed map notifications trigger fresh remote verification, not unconditional remote abort.
- Actual unavailability cancels unsafe interaction; Layer never restarts Show.
- Workspace/layout interruption ends G and preserves ordinary lifecycle safety.
- Retain 64-event batching, lookahead ordering, buffered-event draining after replies/checks, flush-before-wait and no idle polling.

15. Failure matrix

Below, P/F means Partial only if restoration was previously confirmed; otherwise Failed. Historical restoration never substitutes for fresh terminal visibility.

| Condition | Outcome/action | Why |
|---|---|---|
| Body destroyed before admission/preflight | Failed/BodyDestroyed; no operation mutation; owner follows body-loss lifecycle | No managed target |
| Destroyed during visibility | P/F; no Above; body-loss cleanup | Restoration may or may not already have been confirmed |
| Destroyed during Layer | P/F; abort controller; body-loss cleanup | Target no longer exists |
| Withdrawn | P/F/Withdrawn; no remanagement | Outside managed contract |
| Unsupported visibility evidence | P/F/Unsupported; no further mutation | Required evidence unavailable |
| Malformed visibility evidence | No mutation; bounded observation or Unverifiable failure | Cannot infer minimization |
| Unsupported layer evidence | P/F/Unsupported | No supported transition/confirmation |
| Coherent unknown or malformed layer evidence | P/F/Unverifiable | No guessed layer |
| ABOVE + BELOW | P/F/Unverifiable; no success or repair loop | Contradictory target |
| Body desktop changes | P/F/LayoutChanged; no compensation | Original workspace invariant broken |
| Current desktop changes | P/F/LayoutChanged; ordinary input/lifecycle safety continues | User-workspace baseline changed |
| Coherent layer changes during visibility | Continue from fresh layer | G intentionally targets Above |
| Incompatible layer change during Layer | P/F/LayerChanged; no new transition | Prevent competing reassertion |
| Visibility checked-request error | P/F/X11Failure plus fatal owner error handling | Dispatch uncertainty is not success |
| Visibility confirmation timeout | Failed unless restoration was already confirmed; zero Above requests | No ready composition boundary |
| Above checked-request error | P/F/X11Failure plus fatal owner handling | Preserve accepted native-error policy |
| Above confirmation timeout | P/F/ConfirmationTimeout | Final read failed to prove target |
| Below removal succeeds, Above addition fails | Failed for layer-only case; Partial after restoration; Normal only if freshly verified | Intermediate layer progress is reported, not rolled back |
| Input fence failure | No restoration/layer dispatch; Failed plus existing fatal cleanup | Unsettled capture/SHAPE obligations |
| Input restoration failure | P/F at Geometry; no layer work; refence and fatal cleanup | Visibility success does not prove interaction safety |
| Invalid monitor/body size or refused necessary correction | P/F plus accepted fatal geometry handling | No unsafe input or migration |
| Insufficient correction budget | P/F/ConfirmationTimeout; no correction dispatch | Full 150 ms interval required |
| Caller disappears before admission | No admission or mutation | Reply route invalid |
| Caller disappears after admission | Continue bounded work; attempt one reply | No cancellation protocol |
| Reply endpoint disappears after successful work | Operation remains successful; BadWindow nonfatal; caller may report unknown | Delivery failure does not undo work |
| Other terminal-send/X connection failure | Fatal owner condition; preserve actual operation evidence | Existing transport policy |
| Control selection lost | Stop further work, abort layer, best-effort P/F/OwnershipLost, fatal shutdown | No automatic reacquisition |
| Local layer pending | Busy; no G effects | Single layer owner |
| Remote visibility/G active | Busy; no competing effects | Single remote owner |
| Deadline expires between phases | Final read only; Success if entire target already proven, otherwise P/F | No late Above request |
| Quit/duration during G | Best-effort P/F with Shutdown/Closing and retained progress; ordered shutdown | Do not overwrite restoration history with generic Closing |

Body destruction ends the owner lifecycle; do not unnecessarily redefine its existing owner exit classification. Native resource/transport failures retain their existing fatal classification.

The current generic finish_remote_with emits Closing. G needs a narrow completion path before that fallback so an admitted composite’s confirmed progress is not lost. Requests newly arriving during shutdown still receive Closing/exit 5.

16. Deterministic test matrix

Use the existing fake visibility host, layer callbacks, fake clock and scheduler seams. Tests must exercise production composition and routing.

| Group | Required cases/assertions |
|---|---|
| Local layer-only | Normal→Above; Below→Normal→Above; Above no-op; zero restoration/fence mutations for valid ready interaction |
| Local minimized | Each starting layer; restore then fresh target selection; minimized Above needs no addition when retained |
| Local readiness | Correction before layer; 150 ms budget; safe SHAPE enable before layer; geometry/input failure gives zero layer sends |
| Visibility failures | Unsupported, malformed, transitional, Withdrawn, Destroyed, request error, timeout; no Above before readiness |
| Layer failures | Support lost before first send/before addition; malformed read; conflict; send/check/read errors; timeout; unexpected Below reappearance |
| Partial | Restoration then refusal/timeout/error; off-workspace restoration then failure; input failure after observed restoration; historical restoration followed by unknown/destroyed current state |
| Failed distinction | Already-visible Normal failure; already-visible Below removal-only progress; user-time dispatch without restoration evidence; request acknowledgement alone |
| Off-workspace six cases | All three layers × minimized/not minimized; exact request count/order |
| Off-workspace representation | Iconic/Unmapped/no-HIDDEN; coherent Normal/Unmapped and Normal/Unviewable variants; reject local incoherence |
| Off-workspace prohibitions | Zero MapWindow, user-time, caller-workarea placement, correction and input enable; unchanged desktop values |
| Remote/local reader separation | Remote accepts only qualified managed elsewhere state; local reader still rejects non-Viewable |
| Structured layer adapter | Already, waiting, confirmed, timeout phase, support/property failure, attempted/dispatched/checked distinctions |
| Serialized transition | No ADD until fresh removal; support and eligibility revalidated; target already reached skips redundant ADD |
| Timing | 3500 total; 2500 visibility; ≤1000 layer; no phase/duplicate reset; no late mutation; final read can succeed |
| Caller timing | Full discovery/send/wait share command-specific deadline; injected >500 ms overhead demonstrates old 4 s risk; no retry after unknown |
| Scheduling admission | Request-only batch and lookahead both start work before wait |
| Scheduling composition | Visibility completion immediately starts layer; synchronous layer completion needs no external wake |
| Scheduling terminal | Final layer notification at batch end completes before wait |
| Fairness | More than 64 events cannot starve visibility/layer/total deadlines |
| Pointer coexistence | Drag and popup after layer admission do not stall it; ConfigureNotify still confirms movement |
| Busy | Every Hide/Show/G pairing; local pending layer; local selection during both phases; exact duplicate ignored; no queue |
| Lifecycle/transport | Caller disappearance; terminal reply disappearance; selection loss; body destruction; Quit/duration; one terminal attempt |
| Codec/results | Existing v1 round trips/reserved bits unchanged; strict G Partial proof; clear stale confirmation bits; truthful diagnostics |
| Regression | Existing local Above/Normal/Below; all E/F tests; off-workspace deferred return; map SHAPE synchronization; drag/menu/click-through; selection/transport; idle wait |

Later coding must run all four required Cargo checks. Deterministic tests remain separate from host evidence.

17. Owner real-host acceptance

Record exact executable SHA, XID, host versions, starting/final state properties, body/current desktops, effective layer and actual typing behavior.

| Gate | G procedure |
|---|---|
| H10 | Ready Normal and Below → Bring Top; exact Above, same XID/workspace, no typing interruption. Include Above no-op. |
| H11 | Actually minimize in Normal, Below and Above; invoke G; confirm restoration, Above, safe interaction and click-through. |
| H13 | Use observed Openbox Iconic/Unmapped/no-HIDDEN elsewhere; test all layers; confirm no mapping/transport and truthful elsewhere result. |
| H14 | Exercise Normal/non-viewable elsewhere only if genuinely observed. Otherwise record not observed; deterministic coverage is separate. |
| H16 | Actually minimized elsewhere in each layer; G removes HIDDEN and confirms Above there; no activation or new attention side effect attributable to G. |
| H17 | Verify both desktops unchanged, no caller-workarea displacement, and later manual return restores usable interaction. |
| H20 | Rapid mixed Hide/Show/G callers; truthful Busy or terminal results, one body, no hangs/capture leaks. |
| H22 | At least 20 mixed menu/layer/drag/remote rounds, including G during active interaction and menu selection during G. |
| H25 | Above with a genuinely fullscreen ordinary application; fullscreen may cover Vanilla. Exit fullscreen and observe natural return without detection, polling or re-raise. |

Repeat relevant shared-machinery regressions:

- H04–H08: typing focus, Show from all layers, cycles, popup/pressed-row cancellation, drag/OpeningRight safety.
- H09: independently reachable while covered; distinguish incidental Normal-peer raising from Above.
- H12 and H15–H17: off-workspace Show no-op, Hide, restoration and preservation.
- H21: shutdown around remote work.
- H23/H24: workarea correction, exact input silhouette and gestures.
- H26: visible/minimized idle remains event-driven.

Controlled refusal, malformed evidence and exact deadline races may remain injected tests. Do not label them native Openbox acceptance.

18. Likely future changed paths

| Path | Expected responsibility |
|---|---|
| src/main.rs | Shared visibility-stage outcomes, G composition, structured layer adapter, phase-aware routing, terminal/fatal completion and tests |
| src/control.rs | Small G progress record, deadline constants/admission, restoration proof and G result validation |
| src/layer.rs | Outer-deadline-cap entry point and tests; algorithm unchanged |
| src/x11/state.rs | Remote eligibility adapter and marked layer dispatch; local reader unchanged |
| src/x11/control.rs | Deferred G admission, truthful CLI diagnostics, caller deadline, G-aware shutdown result |
| src/x11/visibility.rs | Ideally no production changes; existing observer and primitives are sufficient |

No expected production changes to geometry, interaction, pointer, shape, monitors, rendering, dependencies or CI.

The deadline amendment is owner-approved and recorded by this plan. Each G coding checkpoint still requires an explicit bounded assignment and publication authority. Stage-H implementation/acceptance documents remain outside this assignment.

19. Explicit non-goals

M03.3 Bring Here, combined Recover, workspace movement, global hotkeys, Openbox configuration edits, tray/service architecture, Brain/Spine, A01 relative stacking/perching, other-WM implementation, Wayland, fullscreen detection/fighting, retries, polling and permanent layer reassertion.

20. Open decisions and stop conditions

Owner-approved decisions:

- **D1 APPROVED:** 3500 ms Bring Top owner deadline, with 2500 ms visibility and capped one-second layer work.
- **D2 APPROVED:** 5000 ms Bring Top caller deadline; Hide/Show remain 4000 ms.

The proposed internal result extension requires plan review, but no wire change is presently necessary. Existing status and detail fields can distinguish historical restoration progress from current evidence when the G composer validates them explicitly.

Stop implementation for architectural review if:

- Off-workspace Above cannot be freshly verified with the qualified reader.
- It requires activation or workspace transport.
- Composition requires another layer state machine.
- Truthful results require additional wire semantics beyond this reviewed design.
- Approved deadlines prove inadequate without weakening accepted Show behavior.
- Local menu eligibility must be broadly weakened.
- Input/movement routing cannot preserve accepted interaction safety.
- Real host evidence contradicts approved predicates.
- Fullscreen behavior appears to require WM fighting.

Do not replace these gates with guessed support or compensating mutations.

21. Suggested coding decomposition

Three reviewable commits within one separately authorized G assignment are reasonable:

1. Bounded layer adapter: capped entry, structured outcomes, marked dispatch, remote eligibility and regression tests. Bring Top remains Unsupported.
2. Composition and exposure: shared visibility-stage result, progress/result semantics, event routing, asynchronous admission, CLI and timing.
3. Focused hardening: remaining failure/scheduler/concurrency coverage and explicitly authorized technical notes.

Each checkpoint must build and preserve existing behavior. This is a proposed commit structure, not authorization to create commits or advance stages.

Final Git state remains clean on m03 at 6d693df5dc55de6ec9f411fc615ec84dde5f1faa. git diff --check passed; no implementation or host acceptance was performed.

RECOMMENDATION FOR IMPLEMENTATION:
READY

Historical owner-approved decisions: D1—3500 ms Bring Top owner deadline; D2—5000 ms Bring Top caller deadline. At the original read-only planning checkpoint G1 was next; this historical plan did not itself authorize executable changes. Later implementation and native acceptance are recorded in the current outcome above.
