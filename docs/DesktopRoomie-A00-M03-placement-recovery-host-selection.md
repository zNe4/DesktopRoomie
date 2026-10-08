# DesktopRoomie — A00-M03: Placement, Recovery and Host Selection

Status: architecture/design contract approved for **M03.1 read-only Codex planning**. No M03 implementation or real-host acceptance is claimed.
Prepared against main revision 614dfb22b944e5f9ec0d30a1dcfcf15e3ca41223 (2026-10-08).
Owner authority: owner runs target-host acceptance and authorizes implementation, commits, pushes and merges. Codex may not silently advance.
Roadmap mapping: A00-G3.1 (layers, fullscreen, hide/recovery); A00-G3.2 (workspaces); A00-G3.3 (final host decision).

## 1. Objective and prior evidence

Complete A00 using the existing Rust/x11rb native managed X11 probe, without replacing the tested body and pointer/menu machinery. The executable remains experiments/desktop-probe. Work is sequential and evidence gated; real Openbox/Picom acceptance is essential.

Baseline: A00-M02 is formally accepted at tested executable revision d8dc1abef8b6a8d9395342b234f2f0beb912bc7b, with later docs-only revisions. Review AGENTS.md, docs/DesktopRoomie-A00-A01-plan-v0.1.md, docs/DesktopRoomie-A00-M02-dragging-and-release.md, experiments/desktop-probe/ACCEPTANCE-M02.md, docs/research/studies/R00-M01-nekoai-linux-host-a00-g3.md and .agents/skills/desktoproomie-verify/SKILL.md before code changes.

A00-M03 retains native Rust + x11rb + a managed X11 body unless real-host evidence exposes a disqualifying limitation. Openbox 3.6.1 / X11 / Picom v13 on Archcraft is the first supported host. M03 does not assume support for other window managers, Wayland, or desktop platforms.

## 2. Non-regression invariants for every M03 mission

- Keep a single managed, fixed-size procedural body. Do not swap out the host or add private Vanilla Spine assets.
- Never use keyboard grabs, SetInputFocus, activation ClientMessages, unexpected workspace switches, or a global key listener. Confirm non-activation by typing in another app, not solely from _NET_ACTIVE_WINDOW.
- Transparent padding remains click-through; body gestures and mouse-only popup retain tested semantics.
- Preserve M02's single pointer owner, checked acquisition/release, cancellation before teardown, outstanding release obligations, original-error priority, and no replay of an outside popup click.
- Preserve selected physical-monitor identity, checked workarea intersection, representable coordinates, and verified geometry. Do not migrate to another monitor automatically.
- Use event-driven waiting; no permanent window surveillance, restack loop, button polling while idle, or unbounded retries.
- State what was requested, what was acknowledged at the protocol level, and what Openbox actually did. Never equate a sent ClientMessage with success.
- Support explicit clean Quit and existing duration-expiry recovery through all stages.

## 3. Mission boundaries and order

| Mission | Scope and run state | Gate |
| --- | --- | --- |
| M03.1 | Above / Normal / Below state and fullscreen experiment; body remains directly controllable through its existing mouse menu. | Approved read-only plan, bounded implementation, deterministic tests and owner layer/fullscreen checks. |
| M03.2 | Managed hide/show and independent same-instance recovery/control channel. | No orphan grab, hidden/covered recovery, no focus theft, duplicate and stale process handling. |
| M03.3 | Same-instance workspace placement and destination workarea reconciliation; recover to current desktop. | Valid bounded placement, no change of _NET_CURRENT_DESKTOP, tested invalid/racy destinations. |
| M03.4 | Integrated A00 acceptance and native-host architecture decision record. | Source/diff and CI review plus owner real-host acceptance and a documented capabilities matrix. |

Do not implement a later mission in an earlier checkpoint. In particular, M03.1 has no independent second-invocation CLI; it can extend the already-tested short-lived mouse menu as the direct operator control.

## 4. M03.1 — presentation layers and fullscreen: implementation contract

### Behavior and user control

Provide explicit, idempotent requests for Above, Normal, and Below on the **same existing body**. Use the existing mouse-only right-click menu, extended with three layer choices while retaining Dismiss and Quit. The chosen menu item should dismiss the popup and settle pointer ownership **before** changing host layer state. Re-evaluate popup fit and hit testing for the larger menu using M02's checked bounds; if it does not fit, fail safely and leave input usable.

The current running body must be able to change among these modes without restart. Preserve current startup behavior unless a documented and accepted reason requires a change. This checkpoint neither creates a second process nor introduces generic remote commands.

Use EWMH-managed policy (_NET_WM_STATE; _NET_WM_STATE_ABOVE and _NET_WM_STATE_BELOW). For a managed window, request changes through root-directed ClientMessages with the correct source indication and masks rather than mutating its state property directly or aggressively calling restack on every event. Normal means neither ABOVE nor BELOW; Above means ABOVE and not BELOW; Below means BELOW and not ABOVE. If transitions require separate remove/add requests, serialize them and verify the resulting state. Never accept a contradictory ABOVE+BELOW result as a successful final state.

Check the WM's advertised _NET_SUPPORTED capabilities where applicable. Maintain distinct desired state, observed state, and a pending finite confirmation state. If the property already matches the desired state, succeed without sending a needless change. If a request is asynchronous, observe PropertyNotify and re-read the property, with a bounded explicit-operation deadline (no lifetime polling). The implementation plan should select the exact deadline/diagnostic path. A successful request write alone is not success; on WM rejection, missing support, unreadable/malformed property, denied host operation or timeout, preserve the observed truth and report an actionable outcome.

No unbounded oscillation or automatic reassertion if the user or WM rearranges ordinary windows. A new user request while one is pending must have a specified, deterministic policy (prefer reject as busy with diagnostic to accidental conflicting requests); Codex should propose the narrowest design compatible with existing event ownership.

### Fullscreen contract

When Vanilla is Above normal windows and an ordinary application goes genuinely fullscreen, observe whether focused fullscreen naturally covers Vanilla in this Openbox session. When fullscreen ends, verify whether Vanilla appears Above ordinary windows again without an activation request. M03.1 must **not** implement fullscreen detection, fullscreen polling, automatic bring-to-front fixes, top-layer fighting, or active-window content observation just to simulate the expected result. If target WM behavior differs, report it as evidence for a separate architectural decision.

### Ownership and interaction contract

Layer actions must not interrupt or create a left drag; menu selection occurs after appropriate button matching, popup dismissal and checked pointer cleanup. Existing workspace-change, unmap, body lifecycle, abort, and fatal cleanup behavior remains in force. Keep M02 click-through, rendering, position, focus and selected-monitor limits intact. No keyboard-based menu navigation is required.

### Code touch boundaries (candidates, not mandatory paths)

- x11/window.rs or a small x11/state.rs for atoms, EWMH requests and observed property decoding;
- main.rs for operation orchestration, event observation and pending request outcome;
- x11/menu.rs and the tested menu selection runtime for the extra menu items;
- pure transition logic/tests where it fits current module ownership;
- M03.1-specific implementation note and acceptance evidence. Avoid unrelated crate/dependency churn or broad main.rs refactoring.

Codex must inspect the actual code before finalizing file placement. No architecture or dependency change merely because a helper looks reusable elsewhere.

### Deterministic verification requirements

Include focused tests for:
1. three absolute desired-state predicates; idempotence; opposite-flag removal; contradictory observed combinations;
2. action mapping and ordered completion for the menu's additional items, including release mismatch/outside dismissal and inability to fit;
3. support/property decoding: empty or absent property, malformed format/data, unrelated atoms, missing advertised support, and safe outcomes;
4. pending-confirmation state: acknowledged property, unrelated PropertyNotify, stale responses, timeout, failed send/read, and later fresh operation;
5. no spurious pointer ownership, release debt, or popup resurrection around a layer selection;
6. checked cleanup/fatal error precedence when host operations fail.

Avoid network or actual Openbox dependencies in unit tests; use small seams/fakes where necessary, not a second state framework. Keep existing 112 M02 tests passing.

### Owner M03.1 real-host acceptance matrix (initial contract)

| ID | Real-host procedure | Required observation |
| --- | --- | --- |
| L01 | Launch with another app actively receiving typing | Body appears as before; application keeps typing focus. |
| L02 | Select Above with two partially overlapping ordinary app windows | Vanilla stacks above both where their rectangles overlap and menu capture is gone. |
| L03 | Select Normal, then change ordinary app focus/raise | Vanilla belongs to the normal WM layer; record actual ordering, not a promised fixed spot. |
| L04 | Select Below with ordinary windows present | Ordinary windows cover the body in overlap; deliberate independent recovery is addressed in M03.2, not required yet. |
| L05 | Cycle Above → Below → Normal → Above repeatedly | No contradictory/stuck property, duplicate body or unexpected keyboard activation. |
| L06 | Request same layer twice | Idempotent result; no unexpected flicker, capture or focus transfer. |
| L07 | Activate actual fullscreen application while body is Above | Record whether fullscreen covers the body naturally. |
| L08 | Exit fullscreen, keep the previously focused app usable | Earlier Above policy resumes naturally or a precise WM limitation is recorded; no polling fix. |
| L09 | Type continuously during several menu/layer changes | No keyboard focus theft; actual typing evidence recorded. |
| L10 | Drag, right-cancel, click through padding and menu-dismiss after layer changes | All corresponding M02 input safety behaviors remain. |
| L11 | Menu near corners and narrow workarea | Menu fits and works, or safely refuses to open without stuck grab. |
| L12 | Duration expiry with menu open or a pending state change | Clean exit, checked cleanup, input remains usable. |
| L13 | At least 20 mixed layer/menu/drag operations | No growth of stale pointer ownership, orphan window, or control instability. |

Controlled/injected acceptance: WM advertised-state denial; malformed state; failed checked send/read; confirmation not forthcoming; contradictory flags; popup open/close failure and cleanup ordering. Clearly mark injected evidence rather than claiming real Openbox failure races were reproduced.

Record code SHA, Git status, exact local verification commands/outcomes, corresponding GitHub CI run for same SHA, owner desktop results with actual observed stacking, focus observation method, environment snapshot, and unresolved limitations. Tests/CI alone cannot close L01–L13.

### M03.1 stop conditions

Stop for ChatGPT/owner architecture review if Openbox cannot safely provide basic requested layering, if any implementation path steals focus, if popup/gesture ownership cannot be released reliably, if meaningful behavior requires WM-fighting loops, or if reliable layer verification is unavailable without an architectural expansion. A known Below restriction may be considered a **named reduced-scope fallback** only with owner approval; do not call it a pass automatically.

## 5. M03.2 — preliminary design boundary (not yet implementation authority)

Introduce managed iconification through WM_CHANGE_STATE / IconicState, followed by MapWindow restoration and actual-state confirmation. Cancel any gesture/popup and release ownership before hiding; never set _NET_WM_STATE_HIDDEN manually or withdraw the managed body as the default. Consider SKIP_TASKBAR / SKIP_PAGER only after real target-host observation.

Keep a dedicated never-mapped X11 control window, potentially InputOnly, independent of body visibility. Establish per-display/screen single-instance discovery with a private selection such as _DESKTOPROOMIE_INSTANCE_S0. A second invocation can query its owner and send a direct private ClientMessage; define message authorization/trust bounds, request IDs/acknowledgment, finite response deadlines, ownership races, dead-client handling, and duplicate-launch semantics during the M03.2 plan.

Candidate commands: --show, --hide, --recover, --layer [above|normal|below]. Show means restore visibility without an unrequested desktop or layer change; Recover is an explicitly stronger user action that may move body to current desktop, clamp position and choose Above, still without activation. A CLI send reply is not proof of confirmed state; record explicit confirmation/failure.

M03.2 must have its own detailed plan review, deterministic failure tests, manual acceptance and stop gate before coding.

## 6. M03.3 — preliminary design boundary (not yet implementation authority)

Move **the same instance** using the window's _NET_WM_DESKTOP, never the user's _NET_CURRENT_DESKTOP. Validate target index against _NET_NUMBER_OF_DESKTOPS, read that target's _NET_WORKAREA rectangle, intersect with selected monitor and checked body fit, and reconcile body position before allowing input. Observe effective destination desktop and resulting geometry; reject unsupported/stale destinations, changed desktop count, invalid workarea, or WM refusal with diagnostics. Design owner tests covering two real workspaces, no focus theft or switch, no duplicate, target-workarea bounds, recovery to current desktop, and workspace-count change via deterministic injection when live reproduction is unsafe.

## 7. M03.4 — integrated evidence and exit gates

Reuse all accepted M02 contracts and record the combined implementation's exact executable SHA, final docs SHA separately if needed, local fmt/build/clippy/tests, corresponding clean-environment CI, target Openbox/Picom settings, actual focus/click-through/fullscreen/layer/recovery/workspace evidence, negative paths, limitations and reproducible commands. Make an explicit host ADR with chosen Rust/X11 managed host, required compositor/WM behavior, public reusable module boundaries and deferred compatibility risks. A00 closes only after independent review and owner acceptance. Foreground+Hide is an allowed **reviewed fallback** for unsolved background behavior, not a silent pass. Focus theft or unrecoverable input capture is never accepted.

## 8. Excluded scope

- Arbitrary per-window sandwich ordering (Vanilla between Window A and Window B) is **A01-W1.1** feasibility work, after stable global layer control and selected-window observation.
- A01 window perching/climbing, selected-window moving/minimizing, character-driven layer policy, persisted settings, autonomous movement, renderer replacement, asset/Spine integration, cognition, sensor surveillance, and future Android/Wayland IPC architecture.
- Unsolicited changes to host Openbox/Picom configuration, external app window placement, or user workspace.
- A general-purpose status daemon, tray dependency, global hotkey, permanent active-window polling, or repeated restack loop.

A01-W1.1 differs from M03 global layers: it seeks relative z-order among selected managed ordinary windows and must establish compatibility with Openbox WM policy, re-raising/focus, window/group lifecycle and input isolation before any guarantee. If infeasible, explicitly fall back to proven Above/Normal/Below. Do not prebuild for it in M03.

## 9. Next action: read-only Codex plan for M03.1

Use a Codex-capable model with **High reasoning effort**. Read AGENTS.md, the mission document, M02 specification/acceptance, the host research and actual current code. Codex should:
- report exact branch/HEAD and dirty state; do not overwrite or stash user changes;
- map existing window atom/property handling, event loop, popup and pointer ownership, and test seams;
- propose narrow file-by-file modifications for **M03.1 only** and exact WM request/verification protocol;
- explain property observation, transition/timeout/failure handling and no-focus safeguards;
- map L01–L13 and deterministic cases to implementation and verification;
- flag deviations/architectural questions; do not change any files, run destructive commands, create branches, commit, push or implement;
- provide an ordered bounded implementation plan for independent ChatGPT review.

Only after the plan is reviewed and approved should the owner authorize M03.1 coding.

## 10. Primary references

- docs/research/studies/R00-M01-nekoai-linux-host-a00-g3.md (completed target-host study)
- https://specifications.freedesktop.org/wm/latest-single/ (EWMH state/fullscreen/workspace)
- https://openbox.org/help/Actions (Openbox layer policy)
- https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html (managed iconification)
