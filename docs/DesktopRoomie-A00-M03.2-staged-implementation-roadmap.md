# DesktopRoomie A00-M03.2 — Approved staged implementation roadmap

**Status (2026-10-09): OWNER-APPROVED STAGE DECOMPOSITION; IMPLEMENTATION NOT AUTHORIZED.** This document allocates the approved M03.2 design into eight independently reviewed implementation checkpoints (A–H). Publishing this roadmap **does not authorize M03.2-A**, any Rust changes, subsequent stages, host-configuration edits, or M03.3. A separately approved, bounded Codex assignment is required for every stage.

**Technical authority:** [Approved M03.2 implementation plan](DesktopRoomie-A00-M03.2-approved-implementation-plan.md). Its protocol, classification, lifecycle, tests (Section 12), stop rules (Section 14), and host matrix H01–H28 (Section 13) remain authoritative. This roadmap assigns work; it does not replace or silently revise that technical contract. Follow [AGENTS.md](../AGENTS.md), [WORKFLOW.md](WORKFLOW.md), [INVARIANTS.md](INVARIANTS.md), and [STATUS.md](STATUS.md) for execution and acceptance. The original plan's Section 16 is an earlier sequencing proposal; the stage allocation below supersedes **only that implementation ordering**.

**Repository/target:** `zNe4/DesktopRoomie`, mission branch `m03`, probe `experiments/desktop-probe/`; Archcraft/Openbox 3.6.1/X11/Picom v13. Approved source baseline before staged work: `f8332ec99e876db1d809cd718dc76dff9fd38e27`. The stage assignment must verify the **then-current remote `m03` HEAD** and name its exact starting SHA. A later docs-only HEAD is not a newly tested executable revision.

## 1. Mission scope and dependency order

M03.2 owns: **Hide** (managed minimization preserving independently reachable instance), **Show** (not-minimized state without workspace/layer transport), and **Bring Top** (not-minimized plus Above on the body's **existing workspace**), reached by a short-lived second CLI invocation through a dedicated never-mapped X11 control window.

M03.3 alone owns **Bring Here**, workspace movement and **combined Recover**. M03.2 never writes `_NET_WM_DESKTOP` or `_NET_CURRENT_DESKTOP`, changes the user's current desktop, creates a generic daemon/hotkey service, or silently edits Openbox configuration. Preserve M01/M02/M03.1 focus isolation, input ownership, transparent click-through, bounds, layer and event-loop guarantees.

| Stage | Primary delivery | Hard prerequisite | Proof before proceeding |
| --- | --- | --- | --- |
| **A** | Pure CLI/protocol/results and visibility predicates | Accepted M03.1 source + approved design | Deterministic tests, no falsely successful new command |
| **B** | Instance selection ownership and Starting-before-delay | A accepted | Ownership/race/cleanup tests + host startup smoke |
| **C** | Two-invocation request/terminal-reply transport | B accepted | Real two-process round trip + protocol/failure tests |
| **D** | Strict X11 visibility observation and native input-transition safety | C accepted | Contradictions and capture debt verified; no production visibility dispatch |
| **E** | **Show first** via externally minimized body | D accepted | Owner real-host restoration and preservation checks |
| **F** | **Hide** using verified Show recovery | E accepted | Owner real-host minimization, input safety and recovery |
| **G** | **Bring Top** composed with existing layer controller | F accepted | Owner layer/remote/off-workspace and partial-result checks |
| **H** | Integrated evidence, regression and documentation | G accepted | Full H01–H28 record, independent review, matching CI, owner acceptance |

**Why Show precedes Hide:** Openbox can independently minimize the body. Verify the CLI's safe restoration first; do not create a checkpoint where a newly introduced Hide can strand Vanilla with no confirmed remote recovery.

**Integration rule:** Every stage leaves the probe buildable and preserves already accepted behavior. A–D can contain inert/testable infrastructure, but cannot claim functional Hide/Show/Bring Top. E–G each expose only what is genuinely implemented and verified. H is not a dumping ground for postponed executable integration or safety fixes.

## 2. Universal stage contract and evidence gates

Every stage receives **one explicitly approved Codex prompt**, with:

1. Stage identifier, concrete deliverable, non-goals, exact **expected branch/base SHA**, current source/CI baseline, and authorized paths. Candidate paths below are planning guidance, **not standing editing permission**.
2. Selected Codex model and reasoning effort appropriate to that stage and models actually available; bounded instructions and a stop-on-ambiguity policy.
3. Precise pure/injected/X11/owner-host tests, including applicable cases from approved-plan Section 12 and H01–H28, and explicit assertions about non-regression.
4. Explicit stage-only authorization to commit and ordinary non-force push **to `m03`**, if the owner grants it in that assignment; no default Git publication rights.
5. Required report: starting/ending HEAD, changed paths, complete diff, build/fmt/Clippy/tests, actual host evidence (if performed), exact pushed SHA, CI and outstanding limitations, final Git status.

**Before modifying:** confirm branch, remote HEAD, local status and planned diff scope. Stop on base mismatch, divergence, unexpected dirty owner files, remote movement or rejected push. Never merge/rebase/discard/force-push automatically; no direct push to `main`, tags or mission advancement.

**Before publication:** review staged paths and full diff, ensure requested checks pass, and recheck remote state. Rust work uses the verification requirements in AGENTS and `.agents/skills/desktoproomie-verify/SKILL.md`: `cargo build`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` against the probe's manifest, plus stage-specific tests. Documentation-only work requires link/authority/diff review and `git diff --check`, not a fictitious host or fresh Rust test claim.

**After publication:** ChatGPT independently reviews the exact pushed SHA/diff and matching GitHub CI; the owner performs applicable **real Openbox** acceptance. Injected tests, Xvfb or synthetic event results do not count as owner desktop observations. A push or green CI alone is **not acceptance**. Record accepted executable SHA separately from later documentation/evidence-only commits. Formal owner acceptance is needed before the next stage.

**During unfinished stages:** newly parsed control commands must fail truthfully as unsupported/not implemented where applicable, never report success for receipt, admission, request delivery or a WM request alone. When no functioning client transport exists, do not pretend a command was delivered. Preserve approved exit/result semantics when each transport or operation becomes active.

**Permanent boundaries:** one owner/control endpoint, one active remote operation, one request/one terminal response, no queue/replay cache/retries beyond the approved timestamp retry, bounded deadlines (assuming a responsive X server), no perpetual polling/restack fight. Same-X11-session validation is **not authentication**.

## 3. Stage contracts

### M03.2-A — Pure CLI, protocol/results and visibility classification

**Deliver:** pure parsing for Help/Diagnose/Owner/Control and CLI mode conflicts; versioned request/reply descriptor and marker value types/codecs; command, stage, status, reason, exit-code and diagnostic data models; combined ICCCM/map/HIDDEN/workspace **pure** classification and result composition helpers. Keep clocks and X11 acquisition out of this stage.

**Candidate paths:** `experiments/desktop-probe/src/main.rs`, new `src/control.rs`; module declaration only as needed. Do not add native X11 ownership or visibility mutations.

**Tests:** existing valid flags unchanged; control command combinations rejected appropriately; exact wire layouts, version, reserved-bit and correlation helpers; each supported/contradictory visibility predicate, especially Iconic/Unmapped/HIDDEN versus Iconic/Unmapped/other-desktop/no-HIDDEN; missing/malformed evidence never produces success; truthful result/exit mapping.

**Acceptance:** clean full Rust checks; deterministic tests prove logic independent of live WM; existing owner/diagnostics function as before. Any newly accepted CLI operation that is not yet connected to C must explicitly fail rather than create a body or falsely claim success. No owner-host test required beyond existing behavior if unchanged.

**Stop:** if pure classification requires silently guessing WM evidence, or CLI integration changes normal owner behavior.

### M03.2-B — Selection ownership, timestamps and Starting lifecycle

**Deliver:** dedicated never-mapped `InputOnly` control window; per-screen private selection; genuine PropertyNotify timestamp probe; short `GrabServer` lookup/claim/verification transaction and guaranteed ungrab attempts; at most one fresh-timestamp retry **outside** server grab; startup ownership and Starting descriptor **before `--delay`**; lifecycle and cleanup/selection-loss handling. Preserve old body startup/map and duration semantics; no speculative new map timeout.

**Candidate paths:** new `src/x11/control.rs`, `src/x11/mod.rs`, `src/main.rs` and pure helpers from A. No renderer/WM layer redesign.

**Tests:** request sequence and native timestamp matching, stale/synthetic/zero notifications, bounded probe, duplicate and two-owner interleavings, failed claim verification, no `CurrentTime` fallback, release on all failures, fabricated SelectionClear recheck, real loss/control destruction, descriptor Starting/Ready/Closing, delayed owner blocking duplicates, no body before delay, resource ownership and original-error priority.

**Acceptance:** deterministic arbitration/lifecycle checks and focused real-X11 smoke: one body, duplicate plain invocation refuses during normal startup and `--delay`, no focus transfer. No command round trip is promised **until C**; intermediate owner must not falsely accept unsupported commands or promise a final reply it cannot send.

**Stop:** cannot acquire with real timestamp and bounded retry, unsafe server grab/release or takeover needed, lost selection re-acquired, duplicate body, or startup regression.

### M03.2-C — Independent request/reply transport

**Deliver:** short-lived second-invocation caller, root/screen/owner descriptor discovery, immutable participation marker, direct private `ClientMessage`, correlation, lifecycle/preflight validation, terminal response and exit mapping; one active admission, Busy and invalid-peer handling, bounded caller/owner timing and caller death semantics. Integrate event-loop deadline waiting without a daemon. Existing local pending layer remains mutually exclusive where contract requires.

**Candidate paths:** `src/control.rs`, `src/x11/control.rs`, `src/main.rs`.

**Tests:** exact envelope and marker validation, version/epoch/request ID, malformed destination, unmapped InputOnly reply root matching, duplicate active request, concurrent caller Busy, Starting/Closing, no owner/duplicate, timeout and unknown outcome, buffered final reply before owner destruction, peer death before/after admission, no ticket/replay/resend service, no second body.

**Acceptance:** **two independently launched processes** prove discovery and one correlated terminal response on real X11; all unimplemented Hide/Show/Bring Top return explicit **Failed/Unsupported**, not success. During Starting/Closing a supported preflight return is bounded; no command causes a visibility mutation. Full Rust checks and focused transport smoke pass.

**Stop:** cannot establish a correct reply route/lifecycle without a generic service, unbounded wait, hidden second instance or unauthorized mutation.

### M03.2-D — Native visibility observer and input-transition fence

**Deliver:** strict native property readers/decoders for `WM_STATE`, map state, complete `_NET_WM_STATE` (including HIDDEN and layer flags), body/root desktop properties and EWMH support; checked ICCCM iconify and nonactivating MapWindow request **helpers only**. Build reusable checked empty SHAPE input region, action/motion invalidation, unconditional attempted pointer release (including possible automatic grab), popup cleanup, explicit transition-fence release debt and event-sequence freshness; re-enable the existing silhouette only after readiness validation.

**Candidate paths:** new `src/x11/visibility.rs`, `src/x11/state.rs`, `src/x11/pointer.rs`, `src/x11/shape.rs`, `src/x11/window.rs`, `src/x11/monitors.rs`, `src/main.rs`. Reuse existing resource, geometry and layer components.

**Tests:** all Section 12 visibility/evidence combinations; malformed/absent support stays unverifiable; release debt surviving failure, automatic queued capture, cleanup despite failure, stale body events and pending movement discarded before mutation, exact input-shape restoration, retained monitor and bounds checks, event deadline fairness. Existing input/drag/menu/layer behavior unchanged.

**Acceptance:** no production Hide/Show dispatch is enabled in D. Deterministic/injected host-operation tests establish ordering; smoke-test existing focus, clicking, dragging and popup on Openbox if runtime paths change. A narrowly scoped initial input-disabled-until-placement refinement is allowed only with proof it re-enables safely; if not independently safe, defer that wiring to E.

**Stop:** unresolved pointer release can reach a visibility dispatch, stale events can revive a gesture, or observation weakens existing local layer eligibility.

### M03.2-E — Show (external iconification recovery first)

**Deliver:** first real `--show` command through accepted C transport. Use D observer/fence: restore **genuinely minimized** managed body with checked nonactivating `MapWindow`; if coherently not minimized here or elsewhere, no MapWindow. Preserve **fresh effective layer and workspace**, not the old layer controller desired value. For local restoration, validate retained-monitor geometry and one-shot correction before restoring body input; off-workspace return qualified success with deferred local placement and input unavailable.

**Candidate paths:** `src/main.rs`, `src/control.rs`, `src/x11/visibility.rs`, `src/x11/window.rs`, `src/x11/monitors.rs` and narrow test additions.

**Tests:** minimized local and off-workspace restoration, current/all/other-workspace no-op, both off-workspace Iconic/no-HIDDEN and Normal/non-viewable/no-HIDDEN representations, malformed/withdrawn/destroyed body failure, confirmed no-HIDDEN and geometry, no workarea correction based on caller workspace, layer/workspace drift failure, deadline and input re-enable failures.

**Acceptance:** owner **externally minimizes through Openbox**, invokes Show independently, and confirms same body, no focus theft, no workspace/layer change, safe input/geometry; tests also verify an off-workspace not-minimized body is not deiconified. Show must work **before** implementing the Hide command. Cite relevant H05, H12, H16, H17, H23, H24; mark unobserved host representations pending rather than fabricating evidence.

**Stop:** Openbox restoration requires activation, re-management, workspace movement, or untruthful preservation/visibility claims.

### M03.2-F — Hide (safe only after Show accepted)

**Deliver:** real `--hide` using `WM_CHANGE_STATE / IconicState` under D input fence, then fresh confirmed minimization (`Iconic + Unmapped + HIDDEN`). Off-workspace not-minimized cases **must request** actual minimization even if their ICCCM state says Iconic. Already minimized is idempotent only after settling input obligations; the same control owner remains available.

**Candidate paths:** `src/main.rs`, `src/control.rs`, `src/x11/visibility.rs`, `src/x11/pointer.rs` and narrow tests.

**Tests:** local/off-workspace Hide, idempotence, HIDDEN never appears, refusal/timeouts and incoherent snapshots, mouse-left drag, right opening, active popup row, queued automatic grabs, cleanup failure and release debt blocking dispatch, no stale finishing movement/click, repeated Hide/Show. No direct HIDDEN property write, unmanage/withdraw, taskbar/pager mutation or workspace move.

**Acceptance:** owner performs H04, H06–H09, H15, H20 and applicable H22. The complete independent Show path remains usable after hiding; focused tests confirm no orphan capture or focus theft, including partial failures.

**Stop:** cannot confirm off-workspace Hide via approved ICCCM+HIDDEN path, or Hide can strand a body due to unresolved input cleanup. Return for architecture review; do not add alternative hiding mechanisms.

### M03.2-G — Bring Top via existing M03.1 layer controller

**Deliver:** `--bring-top` = ensure-not-minimized shared E/F visibility stage + existing absolute Above transition. Convert current layer orchestration's log-only distinctions into **structured** remote outcomes without rewriting its state machine. Add a **narrow** off-workspace managed/not-minimized layer reader, leaving ordinary local-menu Viewable requirement intact; cap existing layer deadline under remote total deadline.

**Candidate paths:** `src/main.rs`, `src/layer.rs`, `src/x11/state.rs`, `src/control.rs` and narrow tests.

**Tests:** Above/Normal/Below, mixed flags, minimized Above and minimized Normal/Below, off-workspace Iconic/unmapped/no-HIDDEN and Normal/non-viewable/no-HIDDEN (where genuinely coherent), no MapWindow for not-minimized-other-workspace, no Above mutation when restore fails, **Partial** when actual restoration succeeded but Above failed, Failed when no restore occurred and Above failed, removal/addition partial, busy local/remote layer, no WM fight or workspace write.

**Acceptance:** owner performs H10, H11, H13/H14 when represented on host, H16/H17, H20/H22 and H25; verify Above and not-minimized as **fresh** observations on body's original workspace. Never promise visible to the caller on the current workspace, focus activation, or precedence over fullscreen. Existing local layer menu and input remain unchanged.

**Stop:** off-workspace Above requires weaker state verification, new layer controller, forbidden workspace movement or compensating restack loop.

### M03.2-H — Full integration, evidence and mission acceptance

**Deliver:** full Section 12 regression reconciliation; all applicable H01–H28 owner host instructions/results; CLI usability and optionally owner-managed Openbox Execute smoke; precise source/CI/review/owner acceptance evidence. Create `experiments/desktop-probe/M03.2-IMPLEMENTATION.md`, `experiments/desktop-probe/ACCEPTANCE-M03.2.md`, and update `docs/STATUS.md`, `docs/ARCHITECTURE.md`, `docs/INDEX.md`, `docs/host/OPENBOX.md` within explicit assignment scope. Preserve prior reports without rewriting historical observations.

**Acceptance:** final Rust build/fmt/Clippy/tests, matching GitHub CI, independent exact-diff review, recorded actual Openbox focus/click-through/fullscreen/layer/recovery behavior and bounded command results, explicit limitations and untested cases; **owner formally accepts M03.2**. Keep the exact reviewed executable SHA separate from later evidence-only commits.

**No deferred implementation:** any essential protocol, cleanup or functional problem discovered during H is a bounded correction, separately reviewed/tested/published as authorized, not a broad refactor hidden under acceptance paperwork. H does not automatically open M03.3 or perform Bring Here/Recover.

## 4. Cross-stage risks, timing and verification discipline

- **B→C:** ownership/discovery is necessary but not sufficient for independent control. A Starting descriptor alone is not a successful reply. Until C, lack of working transport is an explicit intermediate limitation.
- **C→D→E:** testable transport and visibility observers come before an enabled WM mutation. A native input fence must settle release debt before actual visibility dispatch. External iconification permits validating Show without a Hide implementation.
- **E→F:** real-host Show and preservation evidence is the safety gate for activating Hide.
- **F→G:** Bring Top composes reused visibility and layer operations. Structured outcomes reflect actual partial progress; no independent third recovery engine.
- **G→H:** all intended user-visible operations must already be implemented. H validates and records them rather than integrating them for the first time.
- **Time/failure:** deadlines are finite logical deadlines assuming responsive X11; synchronous X replies are not guaranteed hard transport timeouts. Caller timeout can mean the admitted operation still completes. No implicit retries or rollback.
- **Stop/owner review:** any approved-plan Section 14 stop condition, including unverifiable minimization, pointer debt, unsafe selection arbitration, workspace transport, focus theft or host predicates contradicted by Openbox, suspends implementation until architectural review.

## 5. Model and stage selection

Select the actual Codex model and reasoning level **at assignment time**, based on the current model menu, tool support and narrow scope; do not hardcode a potentially obsolete availability claim.

| Stage | Initial effort target | Why |
| --- | --- | --- |
| A | Medium | Pure types/parsing/codecs and deterministic tests |
| B | High | Real X server timestamps, arbitration, resource cleanup races |
| C | High | Peer protocol, lifecycle races, terminal result correctness |
| D | High | X11 visibility evidence, implicit pointer capture and release debt |
| E | High | First actual restoration, WM semantics and placement/input readiness |
| F | High | Hiding safety, off-workspace iconification, input release |
| G | High | Remote layer composition, partial results, off-workspace eligibility |
| H | Medium implementation + strong independent review | Integration/evidence audit; escalate only if necessary |

The owner and ChatGPT approve one bounded prompt at a time. Codex implements only the authorized stage and stops after reporting/publishing as explicitly allowed. No stage is automatically authorized by passing a previous one.

---

**Decision record:** Eight stages A–H approved on 2026-10-09 after independent architectural review. The primary design remains unchanged. This is a planning/publication checkpoint; **no M03.2 executable changes or stage-A coding authorization accompany it**.
