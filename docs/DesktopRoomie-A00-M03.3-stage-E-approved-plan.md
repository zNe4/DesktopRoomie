# DesktopRoomie A00-M03.3 Stage E — approved Recover implementation plan

**Decision — 2026-10-20: E-P architecture APPROVED for staged implementation planning.** Independent ChatGPT source/contract review approved the four checkpoints **E1 → E2 → E3 → E4** with the binding clarifications below. The owner authorized publishing this **documentation-only** plan on `m03`. **No E1 implementation or later checkpoint is authorized by this approval.** Recover remains unavailable at the public CLI and Unsupported in the owner/service paths on the accepted Stage-D executable.

**Plan evidence identity:** Codex's original E-P read-only inspection used local/remote documentation HEAD `59014b7adcc4fd11e0f799f4371377f262538253`. Frozen accepted Stage-D executable: `4c67da0b09ffe7b69605ac4a415d256acf8abd23`, matching [CI 39377294762](https://github.com/zNe4/DesktopRoomie/actions/runs/39377294762) (677 tests). The later E-P documentation commit does **not** introduce source/build/workflow changes or prove new Rust/host tests. Historical statements below about the clean working tree, no edits, or pending reviewer approval describe Codex's E-P planning session, not a second test of the published documentation revision.

### Approved review refinements (binding for E1–E4)

1. **Handoff resets phase-specific current proof, not history.** Here→Top must clear/minimization-preservation and any stale Here-only *current* evidence that would contradict intentional restoration or layer change. Retain the original baseline, immutable destination/layout identity, consumed desktop and correction attempts, dispatch history and separately confirmed relocation/restoration history. Handoff itself establishes neither restoration nor a completed Recover result. Rebuild current placement/progress/visibility/Above/input evidence from fresh Top observations before final success.
2. **Top initialization is one-time and cannot replenish authority.** Compute the Top visibility cutoff from the actual validated handoff timestamp, capped by the unchanged 7000 ms owner deadline. Use the existing Show/Top progress carefully: no second geometry correction, no reset of failed/uncertain attempts, no new layer controller, and no phase transition that renews a mutation or total deadline.
3. **One Recover-aware fatal/terminal policy across both phases and external event errors.** Generalize the accepted D4.1 finalizer instead of invoking two sequential service wrappers. Test native errors before Here completes, immediately after a successful handoff, and after Show/Above may have dispatched, including errors from the outer event loop. In all cases preserve the original failure, invalidate *all* current success assertions, keep only actually confirmed history, safely re-fence if needed, clear the one active slot, and attempt at most one correlated terminal reply.

These refinements are the authoritative review outcome for this E-P plan. If their integration conflicts with an existing approved source invariant, stop and request a bounded revision rather than weakening proof or broadening architecture.

## 1. Verified baseline and authority

**The required baseline matches. No implementation was performed.**

| Item | Verified result |
| --- | --- |
| Repository root | Owner's existing repository checkout (machine-specific absolute path omitted in published plan) |
| Branch | `m03` |
| Local HEAD | `59014b7adcc4fd11e0f799f4371377f262538253` |
| Local `origin/m03` | Same SHA |
| Live remote `refs/heads/m03` | Same SHA, checked before inspection and again at completion |
| Working tree | Clean |
| Source/build/CI comparison against accepted executable | Empty diff for probe source, Cargo manifest/lockfile and `.github/workflows` |
| Accepted executable | `4c67da0b09ffe7b69605ac4a415d256acf8abd23` |
| Recorded matching CI | `39377294762`, 677 tests; not rerun or independently queried during E-P |

The governing documents were inspected in the requested order. The approved M03.3 roadmap and this assignment determine Recover’s intended behavior. Current STATUS, source and Stage-D acceptance establish implemented facts; the Stage-D proposed plan supplies historical rationale.

D4.1 is a mandatory dependency: the original D4 candidate passed CI but failed source review because external native errors could bypass Here’s finalizer. D4.1 corrected that boundary before native acceptance.

No files were edited. No Cargo checks, native workspace tests, commits, pushes or branch changes were performed.

## 2. Existing reusable structures and missing pieces

The principal source boundaries are [control.rs](../experiments/desktop-probe/src/control.rs), [main.rs](../experiments/desktop-probe/src/main.rs), and [x11/control.rs](../experiments/desktop-probe/src/x11/control.rs).

| Boundary | Confirmed current behavior | Proposed narrow change |
| --- | --- | --- |
| `ActiveOperation` | One request, active slot, immutable deadline, placement/Show/Top progress and `RecoverPhase::{Here,Top}` | Retain this owner; implement a one-way phase handoff |
| `PlacementProgress` | Frozen destination, immutable original baseline/layout, Here phases, consumed desktop/correction authority, current proof and history | Add Recover Top observation/history methods without applying Here’s preservation rules to intentional restoration/layer changes |
| `PlacementProof` / `RecoveryHistory` | Current confirmation and historical relocation/restoration are separate; `clear_current()` preserves history | Reuse unchanged semantics; make runtime code establish restoration history truthfully |
| `OperationResult::placement` | Already validates Recover Success/Partial/Failed, including complete local Above/geometry/input proof | Keep the validator authoritative; use it for every Recover result |
| Here progression | Complete standalone service with strict evidence, no-op, fencing, relocation, correction and input confirmation | Separate phase completion from standalone result construction |
| Here owner completion | `service_here_owner_with` completes and clears when standalone Here becomes terminal | For Recover, retain the slot on phase completion and continue to Top |
| Show mechanisms | Accepted restoration, geometry, exact SHAPE and fence handling | Reuse through phase-aware eligibility and strict Recover destination guards |
| Bring Top mechanisms | Accepted visibility→single `LayerController` composition | Separate reusable progression/final observations from standalone Bring Top result construction |
| Protocol | Version 2 reserves Recover, destination and both history bits; budgets already exist | No wire change |
| Caller | Bring Here alone performs frozen destination capture/revalidation | Extend that path to Recover with its existing 9000 ms budget |
| Production guards | CLI rejects `--recover`; transport and service return Unsupported | Keep closed through E3; enable together in E4 |
| Shutdown/fatal handling | Here has proof-aware D4.1 handling; Recover still falls through generic/unimplemented paths | Extend the existing policy across both Recover phases |

Two source facts prevent direct reuse without factoring:

- `observe_here()` deliberately rejects changed minimization and effective layer. Calling it during Recover Top would reject successful restoration or an intentional Normal/Below→Above transition.
- Recover’s admission currently initializes `bring_top.visibility_cutoff` at admission +2500 ms. That placeholder cannot serve a Top phase beginning after Here.

Existing protocol values, result bits, native relocation envelope, visibility classifier, monitor validation policy and layer controller need no replacement.

## 3. Recommended ownership and phase architecture

Recover remains **one `Command::Recover` request in the existing active slot**. Its request ID, reply endpoint, epoch, frozen destination, original placement baseline and total deadline remain unchanged.

### Here progression versus command finalization

Factor `step_bring_here_with` and its fresh verification helpers to produce an internal phase outcome:

- pending;
- phase ready, with validated current detail/confirmation;
- rejected or incomplete, with reason and stage;
- native `HostError`.

This outcome is a function result, not another state machine or request owner.

The standalone Here wrapper continues constructing its accepted result and invoking the existing owner finalizer. Recover consumes the same progression outcome differently:

1. Pending retains the active operation.
2. Failure terminates Recover through its placement result rules.
3. Ready triggers a fresh handoff validation.
4. A valid handoff changes `recover_phase` from Here to Top once, initializes existing Show/Top progress, and immediately services actionable Top work.
5. No intermediate terminal response is emitted and the active slot is not cleared.

Handoff preserves consumed desktop/correction authority, historical proof and aggregate dispatch evidence. It does not change `request.command` to impersonate Bring Top.

### Top mechanisms versus result construction

Factor the existing Top service into reusable progression and final-observation work, with separate command result adapters:

- Standalone Bring Top continues using `OperationResult::bring_top`.
- Recover uses `OperationResult::placement(Command::Recover, ...)`.

Recover Top reuses the existing Show phases, restoration helpers, geometry confirmation, exact SHAPE restoration and sole `runtime.layers`. It must never return a standalone Bring Top result.

Use the existing borrowed native adapter pattern. Extend the placement adapter with delegated visibility/Top capabilities and Recover-only validation gates; do not introduce a parallel visibility or layer implementation. Ordinary invalid evidence must remain distinct from native acquisition failure, preserving D2.1’s error separation.

### Strict destination continuity

During Top, retain Here’s admitted `DestinationIdentity`. Validate that identity around observations, before each recovery mutation, and during final confirmation.

The guard must cover desktop count/current target, target workarea, viewport policy, retained monitor identity/bounds, usable area, fixed body size and valid origins. A failed guard grants no mutation authority. Native guard errors enter the shared fatal path.

Do not rely solely on legacy Show placement reads: those do not establish the full frozen Stage-B identity.

### Correction budget

Use **one correction allowance for the entire Recover operation**:

- Here may consume it for a visible body.
- Minimized Here defers geometry, leaving it available for Top restoration.
- Top inherits an already-consumed allowance; initializing Show progress must not grant another correction.
- A failed/uncertain attempt retains the existing fence-local automatic-correction restriction. **One Recover-wide consumed-correction record is authoritative across both Here and Top and cannot be reset by initializing ShowProgress or relabeling the phase.**

Independent Show and Bring Top commands keep their accepted budgets and behavior.

## 4. Phase transition and deadline policy

Let:

- `A` = Recover owner admission.
- `D = A +7000 ms` = immutable owner deadline.
- `H = min(A +3500 ms, D)` = Here cutoff.
- `T` = time of the single validated Here→Top handoff.
- `V = min(T +2500 ms, D)` = Top visibility cutoff.
- `L` = first layer admission after visibility readiness.
- `B = min(L +1000 ms, D)` = layer cutoff.
- `C = caller start +9000 ms`, before desktop capture/discovery.

`H` can be derived from the existing Recover deadline as `D −3500 ms`; no second owner timestamp is necessary. Standalone Here continues using its existing deadline.

At handoff, replace Recover’s unused admission-time Top cutoff with `V` **once**. Keep `D` unchanged. Existing `layer_admitted` records `L` once. **Clear stale Here-only current placement/progress proof atomically with that handoff, retaining historical transitions and all previously consumed mutation authority; fresh Top evidence must rebuild current proof.**

| Phase | Entry condition | Permitted work | Deadline / exit |
| --- | --- | --- | --- |
| Here | Recover admitted | Existing strict Here progression | `H`; earlier outstanding correction deadline also wakes service |
| Handoff | Fresh complete Here conditions, ownership retained, time before `H` | Initialize Top progress; no reply | One-way transition at `T` |
| Top visibility | Valid frozen destination and same body | Accepted local restoration, remaining correction allowance, safe input readiness | `V` |
| Top layer | Fresh local visibility/geometry/input readiness before `V` | Existing absolute Above controller | `B` |
| Final validation | Mechanism completed or applicable cutoff reached | Bounded read-only current proof; terminal construction | Cannot authorize further recovery mutation |
| Caller | Before capture/discovery | One capture, marker, request and correlated wait | `C`; never renewed |

A correction requires its full 150 ms confirmation interval inside the applicable Here/visibility cutoff.

At any cutoff, a bounded read-only final observation may recognize an already-complete Recover postcondition. It cannot start restoration, correction, enabling or layer mutation. If Here reaches its cutoff without allowing Top execution, minimized relocation alone cannot produce Recover Success.

Check time after synchronous observations and immediately before native dispatch. Recover-only mutation guards may themselves consume time, so check again after them. Events, duplicate requests, final reads and delayed scheduling never renew deadlines.

Mandatory safety re-fencing/release/teardown after failure remains cleanup, not additional recovery authority. It cannot restore input, retry a mutation or resume a phase. Logical deadlines continue to assume a responsive synchronous X server.

## 5. Phase-A boundary and proof rules

### Exactly when Here permits Top

| Here outcome/evidence | Recover action |
| --- | --- |
| Already-local visible, safe, fixed-size, in bounds | Freshly validate Here; proceed to Top. No desktop send |
| Already-local genuinely minimized | Require preserved genuine minimization, valid fit and settled empty input fence; proceed to Top restoration |
| Remote visible | Require actual destination membership, preserved initial minimization/layer, verified geometry and safe input readiness; then Top |
| Remote genuinely minimized | Require actual destination membership, preserved minimization/layer, valid deferred fit and settled fence; then Top |
| Off-workspace Iconic + Unmapped without HIDDEN | Treat as not minimized. Require confirmed local arrival/readiness; do not infer restoration history |
| Visible out-of-bounds geometry | Allow the single checked correction; transition only after actual geometry/input confirmation |
| Initial conflicting/transitional or unverifiable evidence | Fail without Show/Above mutation |
| Transitional evidence after valid baseline/fence | Wait for relevant evidence or cutoff; no handoff while unsettled |
| Changed current desktop/count/target workarea/viewport/retained-monitor identity or bounds | Invalidate and terminate; no retargeting |
| Retained monitor unavailable, impossible fit or wrong fixed size | Terminate; no migration, resize or geometry-learning restoration |
| Coherent unexpected minimization/layer change during Here | Terminate preservation failure; no compensating Show/Above |
| Ownership mismatch | OwnershipLost; no Top mutation |
| Ownership-query/native checked-dispatch failure | X11Failure through shared fatal finalization |
| Here timeout with incomplete conditions | Terminate using retained historical proof |

Desktop-send acknowledgement never satisfies the relocation requirement. A Here phase failure cannot enter Show or Above even when historical progress permits Partial.

### Historical versus current evidence

**Current source:** relocation is recorded by `observe_here()` only after a remote baseline, consumed/possibly dispatched desktop attempt and fresh supported destination/current membership. It survives later invalidation. Pure Recover proof validation already permits independent restoration history, but the runtime has no Recover implementation establishing it.

**Proposed rules:**

- Keep `placement.baseline` as the original pre-Recover body state.
- Record restoration only after that baseline was genuinely minimized and a later fresh, coherent, supported same-body observation confirms not-minimized state.
- Record the historical fact before separately evaluating layout/layer readiness. Independent evidence loss must not erase an already-confirmed transition.
- Apply this rule to Recover observations, including bounded fatal-history observations. An externally completed restoration can establish history; the diagnostic claims confirmation, not causation.
- If restoration occurs during Here and violates Here’s preservation obligation, terminate Here. Its historical fact does not authorize Top execution.
- Show dispatch, user-time write, MapWindow/HIDDEN acknowledgement, geometry correction, Above confirmation and an initially visible body do not establish restoration.
- Keep existing `BringTopProgress` restoration bookkeeping consistent with Recover history; it must not become competing terminal authority.

Top must rebuild current placement proof using Recover rules. It cannot reuse `observe_here()` unchanged. **The one-way handoff must first invalidate Here-only current proof without losing original identity/history; neither phase switch nor ordinary request acknowledgement establishes restoration, Above, local geometry, input readiness or Success.** Intentional restoration clears `minimization_preserved`; intentional layer changes clear `layer_preserved` when the original layer no longer matches.

After fatal failure or committed shutdown, clear current placement, visibility/Above/input confirmation and associated detail coherently. `clear_current()` alone does not reset all those fields.

### Terminal outcome matrix

| Fresh current postcondition | Relocation history | Restoration history | Recover result |
| --- | --- | --- | --- |
| Complete local, not minimized, Above, verified geometry/input | Any | Any | Success / None |
| Incomplete | No | No | Failed |
| Incomplete | Yes | No | Partial: relocation confirmed earlier |
| Incomplete | No | Yes | Partial: restoration confirmed earlier |
| Incomplete | Yes | Yes | Partial: both transitions confirmed earlier |
| Fatal teardown or admitted shutdown | Retain | Retain | Failed/Partial after clearing current proof |
| Pre-admission lifecycle/Busy/protocol rejection | None | None | Existing status semantics |
| Caller lacks a trustworthy timely reply | Unknown | Unknown | OutcomeUnknown |

Recover Success uses `Stage::Layer`, including an entirely satisfied no-op. Complete proof cannot be mislabeled Partial merely because history exists.

## 6. Event loop, visibility/layer reuse and fatal routing

### Phase-aware scheduling and ownership

Replace the current Recover placeholders—unconditional service due and ignored observation marking—with actual phase scheduling.

| Active phase | Placement/evidence ownership | Layer ownership |
| --- | --- | --- |
| Here | Existing Here routing and deadline, extended to Recover Here | None; local layer service excluded |
| Top visibility/geometry | Recover consumes placement/map/workspace evidence; legacy correction and deferred restoration excluded | None; local layer service excluded |
| Top layer | Strict destination/lifecycle events remain Recover-owned; accepted Configure reconciliation may continue | Existing controller exclusively owned by Recover |

During Top layer, preserve accepted Bring Top handling of later valid gestures. Ordinary movement must not become a second recovery correction or bypass frozen-layout validation. Movement invalidates stale final geometry proof, which must be freshly established before Success. An unfinished gesture/input obligation cannot be disguised as successful readiness.

Adapt these helpers together:

- `remote_owns_placement`, `remote_owns_layer`, `local_layer_service_allowed`;
- `remote_operation_deadline`, `remote_service_due`, `mark_remote_observation`;
- `route_here_evidence_event` and Recover Top evidence routing;
- `route_owner_request`, `service_owner_operation` and wait-deadline selection.

Recover-specific layout routing must include count and viewport properties throughout Top; the legacy visibility predicate alone omits them. Equal relevant layout evidence may continue. Confirmed changed identity latches failure even if later values return.

Keep progression at loop top, both consumed-request sites, deadlines within batches, and end-of-batch before sleep. A successful handoff must immediately service Top or return to an explicit due-service boundary before socket wait. Do not wait for an additional WM notification merely to start the next phase.

Clear due flags after bounded work. Re-arm only for relevant evidence or immediate actionable transitions. Preserve 64-event batching, buffered-event draining and earliest-deadline waits; add no polling loop.

### Visibility and layer adaptations

Make command-dependent branches recognize **Recover Top**, rather than treating every Recover phase as Bring Top.

- Here retains original minimization/layer preservation.
- Top establishes its workspace baseline on the frozen destination.
- Recover never accepts off-workspace deferred Show/Top completion as recovery readiness.
- Existing standalone off-workspace Show/Bring Top eligibility remains unchanged.
- Expected layer-controller transitions remain valid; a Normal→Above notification is not automatically LayerChanged.
- Unsupported/conflicting evidence or incompatible layer interference follows accepted controller failure rules.
- Validate the destination before each removal/addition, including the second mutation after removing Below.
- Final Success requires fresh effective Above plus complete destination/geometry/input evidence. Layer acknowledgement alone is insufficient.

### Extend D4.1; do not add a competing fatal policy

Generalize the existing Here-aware owner/fatal seam to handle standalone Here and both Recover phases. **The source-review gate must inspect native exceptions both inside each phase and outside the operation scheduler (including just after handoff), so they cannot fall into an unrelated generic Closing/failure path.**

The same finalizer must receive errors from phase service, ownership queries, checked native calls, asynchronous `Event::Error`, buffered-event inspection, poll/read and outer-loop failures.

Required sequence:

1. Preserve the original `HostError`.
2. Commit failure and invalidate current success evidence.
3. Abort Recover-owned layer work without rollback.
4. Perform necessary safety re-fencing/release cleanup while preserving original-error priority.
5. When the connection/body/ownership permit, perform one bounded read-only history observation using phase-appropriate interpretation.
6. Construct Recover Failed/Partial with `X11Failure` and Shutdown.
7. Clear the active slot before the fatal reply attempt and global cleanup.
8. Attempt one correlated terminal response.
9. Continue ordered global cleanup without a duplicate reply.

Extend `fail_here_owner_with`, `finalize_bring_here_with`, `complete_here_owner_with`, `fail_owner_operation` and `ControlWindow::service_here_with` coherently. Naming may become placement-oriented; their policy remains shared.

`finish_probe_run_with` retains its central external-error boundary. Clean Quit/duration remains distinct from native failure. Actual selection loss remains OwnershipLost; a failed query remains X11Failure. `finish_remote_with` needs Recover-aware shutdown construction so generic Closing cannot erase history.

## 7. Caller, public interface and diagnostics

Reuse Bring Here’s caller transport for both destination commands:

1. Start the command-specific immutable budget before capture/discovery.
2. Perform the existing two strict count/current reads.
3. Freeze the destination before owner discovery.
4. Discover/watch the owner and write one v2 immutable marker carrying the actual command.
5. Rediscover the same owner XID/epoch and require Ready.
6. Revalidate captured count/current without rewriting the marker.
7. Recheck the deadline, send one request and await correlation.
8. Preserve existing cleanup and OutcomeUnknown behavior.

Generalize the hard-coded Here marker builder to accept BringHere or Recover. Do not add a desktop argument, new marker field, wire version, owner redirect or second request.

Only E4 exposes `--recover`, with the existing command exclusivity/help conventions. Until then, CLI rejection and both production Unsupported guards remain intact.

Recover diagnostics must distinguish:

| Result | Required meaning |
| --- | --- |
| Success | Same body confirmed on frozen workspace, not minimized, Above, placement verified and input ready |
| Failed without history | Complete Recover unconfirmed; no accepted relocation/restoration history |
| Partial relocation | Relocation was confirmed earlier; complete current recovery failed |
| Partial restoration | Restoration was confirmed earlier; complete current recovery failed |
| Partial both | Both transitions were confirmed earlier; current complete postcondition is unconfirmed |
| Timeout/layout/layer/ownership/X11 failure | Identify the actual reason and stage without converting history into current truth |
| OutcomeUnknown | No trustworthy timely terminal result; no resend or inference from eventual WM state |

Include uncertain-dispatch wording when appropriate. Preserve established exit mapping, including Success 0, OperationFailed 7, OutcomeUnknown 8 and Partial 9.

## 8. Deterministic and owner-native verification

All tests below are future implementation requirements; none ran during E-P.

| Group | Required scenarios and assertions |
| --- | --- |
| **Pure phase/deadline model** | One-way handoff; rejection without Here readiness; original baseline/layout retained; exact 3500/2500/1000 partitions inside 7000 total; one-time cutoff initialization; correction interval fit; expiry at every boundary; duplicates/events cannot renew time |
| **Pure proof model** | Complete/incomplete × all four history combinations; initial visible/local/no-op/correction/ack-only negative history; genuine restoration confirmation; history surviving current loss; minimized Here readiness never equals Recover Success; shutdown/fatal invalidation; existing codec/result cross-products retained |
| **Injected normal paths** | Local/remote × visible/minimized × Normal/Above/Below; safe no-op; Iconic/no-HIDDEN; minimized relocation followed by restoration before Above; same body and frozen destination |
| **Injected mutation gates** | Drift before/after desktop send, restoration, correction, input enable, layer removal/addition and final read; invalid count/workarea/viewport/monitor/size; no mutation after invalidation or cutoff |
| **Injected native failures** | Failure before cookie and after possible dispatch; user-time versus map dispatch; failed correction/SHAPE/ungrab/popup cleanup; no retries; consumed authority retained; exact input fence and original error preserved |
| **Injected history failures** | Relocation then failure; restoration then failure; both then current evidence loss; restoration observed before later layout/layer failure; fatal history observation cannot reconstruct Success |
| **Production scheduling/routing** | Actual request and owner seams; one active slot; no intermediate reply; immediate admission/handoff service; missing wake event; >64-event traffic; earliest deadline/correction wakeups; late events do not reopen work |
| **Production lifecycle/fatal** | Outer error before/after every phase boundary; actual ownership loss versus failed query; Quit/duration/body/control destruction; caller death/reply failure; active cleared and one reply attempt; global cleanup cannot reply again |
| **Caller/public** | Capture before discovery; frozen count/current; immutable marker; owner epoch replacement; deadline consumed during every synchronous step; one send; late reply Unknown; parser exclusivity/help; reason/history diagnostics and unchanged exits |
| **Regressions** | Full accepted Here, Hide, Show, Bring Top, menu-layer, pointer debt, neutral cancellation, exact SHAPE, click-through, retained monitor, startup/duplicates/Busy/epoch/correlation and cleanup behavior |

Use ordered fake-host logs, scripted snapshots and virtual time. Tests must exercise the production orchestration seams, not a separate simulation of the intended algorithm.

After E4 source review and matching CI, the owner-native matrix should cover:

- Local and remote visible/minimized Normal/Above/Below, including already-ready Above and off-workspace Iconic/no-HIDDEN.
- Continuous real typing, focus metadata, no activation/workspace switch, same body XID.
- Retained monitor when invoked from either monitor; verified placement and click-through after restoration.
- Drag, right-hold and popup cancellation; stale release suppression; fresh gestures afterward.
- Covered/minimized independent control, repeated recovery, concurrent callers/Busy and independent-command regressions.
- Quit, duration, kill/relaunch and truthful terminal/Unknown outcomes where safely reproducible.
- Passive post-operation stability using existing observation practice.

Record exact executable SHA/CI, commands/exits, relevant state/geometry, actual typing and qualifications.

Malformed root properties, synthetic conflicting states, forced checked errors, reply loss, exact deadline crossings, monitor-loss simulations and WM-refusal races should remain deterministic obligations. Do not corrupt the owner’s Openbox session or remove displays merely to manufacture native coverage. Additional instrumentation/configuration experiments require separate authorization.

## 9. Proposed implementation checkpoints

All code paths below are relative to `experiments/desktop-probe/`.

Each checkpoint requires a separate assignment, exact-SHA source review, matching CI and explicit acceptance before advancement. Publication requires mission-specific authority. No checkpoint implicitly permits documentation, Cargo, dependency or CI edits.

For each code checkpoint, run the four mandated Cargo checks with the probe manifest, review the complete scoped diff, and run `git diff --check`. No future test-count target is prescribed.

### E1 — Pure Recover handoff, timing and proof policy

- **Prerequisite:** approval of this plan and reverified assignment baseline.
- **Permitted files:** `src/control.rs`.
- **Purpose/ownership:** extend existing operation methods for Recover-aware Here eligibility, one-way handoff, phase cutoff derivation, Top current-proof rebuilding, independent restoration history and shutdown.
- **Expected behavior:** no new production execution; standalone command behavior unchanged.
- **Tests:** pure phase/deadline/proof matrix, **handoff clears all contradictory Here-only current proof without erasing history/identity or consumed authority**, correction consumption across handoff (including failed/uncertain correction), one-time Top cutoff, and all existing placement result/codec cross-products.
- **Forbidden:** host calls, new controller/state owner, wire changes, validator relaxation, public enablement.
- **Verification/gate:** mandated checks, exact diff/CI review, reviewer agreement that the pure record suffices for E2/E3.
- **Reachability/native:** CLI and both Unsupported guards remain; no Recover native testing.

### E2 — Reusable progression and guarded Recover mechanisms

- **Prerequisite:** accepted E1 SHA/CI and separate assignment.
- **Permitted files:** `src/main.rs`.
- **Purpose/ownership:** separate Here phase outcomes and Top mechanism outcomes from standalone command result construction; implement internally callable Recover composition and strict mutation guards on borrowed native resources.
- **Expected behavior:** direct injected execution can complete Here→Top under one operation; existing public commands retain their semantics.
- **Tests:** injected normal, minimized, no-op, ordering, drift, correction-budget, fence, deadline and checked-failure cases; standalone regression call logs.
- **Forbidden:** production Recover routing, CLI enablement, another layer controller, changed native envelope/classifier/monitor policies, unrelated source splitting.
- **Verification/gate:** mandated checks, exact diff/CI review, source reachability audit proving both production guards remain closed.
- **Reachability/native:** Recover remains unavailable; no native Recover run.

### E3 — Guarded owner, scheduler, lifecycle and fatal integration

- **Prerequisite:** accepted E2 SHA/CI and separate assignment.
- **Permitted files:** `src/main.rs`, `src/x11/control.rs`, narrowly `src/control.rs` for integration-required pure tests/refinements.
- **Purpose/ownership:** connect the complete internal service to phase-aware scheduling/event ownership and extend D4.1’s single owner/fatal/shutdown finalizer.
- **Expected behavior:** deterministic production-seam tests prove one active operation, no intermediate response, timely phase progression and one final reply.
- **Tests:** full production routing/lifecycle/fatal matrix, both request sites, handoff-before-sleep, sustained traffic, shutdown/history, query failure versus loss, duplicate reply prevention and caller disappearance; **especially external X11 failures before Here finishes, immediately after handoff, and after possible Show/Above dispatch, with one Recover-aware fatal/terminal result**.
- **Forbidden:** removing either Unsupported guard, exposing CLI, postponing fatal/input safety to Stage F, caller/public behavior changes.
- **Verification/gate:** mandated checks, exact diff/CI review and explicit audit of every terminal/error/global-cleanup route.
- **Reachability/native:** Recover remains publicly unreachable; no native Recover testing.

### E4 — Caller/public enablement and bounded native acceptance

- **Prerequisite:** accepted E3 SHA/CI and separate enablement assignment.
- **Permitted files:** `src/main.rs`, `src/x11/control.rs`.
- **Purpose/ownership:** generalize frozen caller capture/marker, initialize Recover admission, remove both execution guards together, expose CLI/help and diagnostics.
- **Expected behavior:** one public `desktop-probe --recover` request executes the fully reviewed internal operation.
- **Tests:** caller/public matrix and integrated caller→transport→owner→terminal tests; full deterministic regressions retained.
- **Forbidden:** new behavior, wire version, dependencies, host configuration edits or automatic Stage F/G advancement.
- **Verification/gate:** mandated checks, exact public diff review and matching CI first; then owner-native matrix on that frozen executable and explicit owner acceptance.
- **Reachability/native:** Recover becomes a complete public candidate here; native testing is appropriate only after the review gate.

This split keeps each new concern reviewable while preventing public exposure before fatal, shutdown and scheduling integration are complete.

## 10. Risks, approval points and first checkpoint

The principal source-level risks are:

- Completing/clearing the active slot at Here success.
- Applying Here preservation checks to intentional Top changes.
- Reusing the admission-time Top cutoff.
- Granting a second correction during handoff.
- Allowing legacy layout/input recovery to race Recover-owned work.
- Recording restoration from dispatch or pre-existing visibility.
- Returning standalone Bring Top proof as a Recover result.
- Losing historical progress or the original error through D4.1’s external-error/global-cleanup boundary.
- Sleeping before newly actionable admission/handoff work.

No unavoidable protocol conflict or need for another controller was found.

**Review disposition — approved 2026-10-20:** the owner approved the four-checkpoint split, one correction allowance across Recover, separated progression/finalization, independently confirmed restoration history without a causation claim, strict destination validation throughout Top, and extension of D4.1 rather than a new fatal policy. The three binding handoff/current-proof, consumed-authority, and cross-phase/external-fatal refinements at the top are required E1–E4 acceptance criteria. **Implementation of E1 still requires a separate assignment.** These are plan-review decisions, not implementation authorization.

Stop and report if implementation requires broader file scope, weaker proof validation, another owner/controller/queue, repeated correction, property forcing, focus/activation/workspace mutation, monitor migration, or a changed native-error policy. Also stop for branch/base/owner-change mismatch, accepted-command regression, failed review/CI, or native behavior violating the contract. Stage F/G remain separately gated.

**Recommended first checkpoint: E1.** The existing pure model is close to sufficient, and proving handoff, timing, historical evidence and terminal partition first gives E2/E3 a precise contract without exposing native Recover behavior.

At the time of Codex's **read-only** E-P session, the repository remained clean on `m03` at `59014b7adcc4fd11e0f799f4371377f262538253`; no commit or push was made **during that planning session**. Subsequent review approved E-P for documentation-only publication; this approval does not change the accepted executable or authorize Stage E code.
