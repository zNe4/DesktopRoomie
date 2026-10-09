# DesktopRoomie — A00-M03: Placement, Recovery and Host Selection

Status: **M03.1 accepted** on the target Openbox/X11/Picom host. Client-approved recovery separation and host-integration policy were clarified 2026-10-09. M03.2 remains a read-only design gate; no M03.2 or M03.3 implementation is authorized.
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
| M03.2 | Managed hide/show and independent same-instance control; explicit **Bring Top** within the body's existing workspace. | No orphan grab, safe hidden/covered control, no focus theft, duplicate/stale instance handling and verified Above/visible outcome within that workspace. |
| M03.3 | Same-instance workspace placement and destination workarea reconciliation; explicit **Bring Here** plus combined recovery composed from Bring Here and Bring Top. | Valid bounded placement, no change of _NET_CURRENT_DESKTOP, preserved layer for Bring Here, tested invalid/racy destinations and partial composite outcomes. |
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

**Approved client semantics (2026-10-09): recovery has independent, composable operations.** M03.2 owns visibility, an independently addressable control channel and **Bring Top on the body's existing workspace**. M03.3 alone owns explicit placement on the user's current workspace (**Bring Here**) and their eventual combination. Do not smuggle workspace movement into M03.2 under a generic Recover name.

Introduce managed iconification through `WM_CHANGE_STATE` / `IconicState`, followed by `MapWindow` restoration and actual-state confirmation. Cancel any gesture/popup and release ownership before hiding; never set `_NET_WM_STATE_HIDDEN` manually or withdraw the managed body as the default. Consider `SKIP_TASKBAR` / `SKIP_PAGER` only after real target-host observation.

Keep a dedicated never-mapped X11 control window, potentially `InputOnly`, independent of body visibility. Establish per-display/screen single-instance discovery with a private selection such as `_DESKTOPROOMIE_INSTANCE_S0`. A second invocation can query its owner and send a direct private `ClientMessage`; the read-only plan must resolve versioned request/reply framing, request IDs, acknowledgment versus observed final outcome, deadlines, ownership races, stale/dead clients, duplicate-launch behavior and same-X-session trust limitations. Private X11 discovery is not a security boundary against another X11 client.

**Candidate commands and contracts (spellings not yet fixed):**

| Command | M03.2 behavior |
| --- | --- |
| `--hide` | Safely iconify the managed body; leave the same process and independent control channel available. |
| `--show` | Restore a hidden/iconified body; do not implicitly change workspace or chosen presentation layer. |
| `--bring-top` | Restore visibility if necessary and request/confirm **Above** using the existing M03.1 controller; do not move the body to the caller's current workspace or activate it. |
| `--layer above\|normal\|below` | Optional remote access to the existing absolute layer operation, if narrowly justified in the plan; never reimplement the layer state machine. |

If `--bring-top` acts on a body on another workspace, it must not report that the body is visible **to the caller on the current workspace**. Distinguish success on the body's workspace from not being present here. Do not automatically change `_NET_CURRENT_DESKTOP` or `_NET_WM_DESKTOP`. Commands received from a second invocation must settle to a truthful confirmed/failure outcome; successful transport alone is not completion.

The read-only plan must also resolve iconification/restore ordering relative to pending layers, drag, popup, geometry correction, lifecycle events and shutdown without a focus steal or orphan grab. M03.2 must have its own detailed plan review, deterministic failure tests, manual acceptance and stop gate before coding.

## 6. M03.3 — preliminary design boundary (not yet implementation authority)

**Bring Here** moves **the same instance** to the caller/user's current workspace while preserving its existing Above/Normal/Below policy. Request `_NET_WM_DESKTOP` on the body, never change `_NET_CURRENT_DESKTOP` to transport the user. Validate the target index against `_NET_NUMBER_OF_DESKTOPS`, read that target's `_NET_WORKAREA` rectangle, intersect with the selected monitor and checked body fit, and reconcile body position before enabling input. Observe the effective destination desktop and resulting geometry; reject unsupported/stale destinations, changed desktop count, invalid workarea or WM refusal. The M03.3 plan must decide explicitly how an iconified body is treated; do not assume a hidden instance becomes viewable just because its workspace changed.

**Combined Recover** is an explicit **composition** of Bring Here and Bring Top, not a third recovery mechanism. Reuse the same-instance control channel and existing layer controller. Define execution order, confirmation and the honest **partial-success** result if one operation succeeds and the other fails. Candidate user commands are `--bring-here` and `--recover`; final syntax remains design-gated. M03.3 owner checks cover two real workspaces, no focus theft or user-workspace switch, no duplicate, preserved layer on Bring Here, destination workarea bounds, explicit combined recovery and partial failure/injected races.

### Native host integration, not a duplicate window-manager service

**Approved policy (2026-10-09): prefer reliable host facilities over reimplementing them.** DesktopRoomie owns correctness-critical lifecycle/state, second-instance request handling and truthful result reporting; the desktop environment may own optional keyboard shortcuts, launchers, menus and autostart. Do not implement global key capture, a custom shortcut daemon, a tray manager or silent edits to a user's dotfiles just to make A00 recovery convenient. Command-line control must work independently of any optional shortcut setup.

The target-specific guide is [Openbox host integration](host/OPENBOX.md). It distinguishes required configuration from optional conveniences and describes user-controlled edits under `~/.config/openbox/`. No user configuration is changed by this documentation decision; all proposed shortcut commands are future candidates until implemented and accepted. Other hosts can have separate integration guides later.

## 7. M03.4 — integrated evidence and exit gates

Reuse all accepted M02 contracts and record the combined implementation's exact executable SHA, final docs SHA separately if needed, local fmt/build/clippy/tests, corresponding clean-environment CI, target Openbox/Picom settings, actual focus/click-through/fullscreen/layer/recovery/workspace evidence, negative paths, limitations and reproducible commands. Make an explicit host ADR with chosen Rust/X11 managed host, required compositor/WM behavior, public reusable module boundaries and deferred compatibility risks. A00 closes only after independent review and owner acceptance. Foreground+Hide is an allowed **reviewed fallback** for unsolved background behavior, not a silent pass. Focus theft or unrecoverable input capture is never accepted.

## 8. Excluded scope

- Arbitrary per-window sandwich ordering (Vanilla between Window A and Window B) is **A01-W1.1** feasibility work, after stable global layer control and selected-window observation.
- A01 window perching/climbing, selected-window moving/minimizing, character-driven layer policy, persisted settings, autonomous movement, renderer replacement, asset/Spine integration, cognition, sensor surveillance, and future Android/Wayland IPC architecture.
- Unsolicited changes to host Openbox/Picom configuration, external app window placement, or user workspace. Optional manual Openbox shortcut/launcher setup belongs in the host guide, not automatic probe code.
- A general-purpose status daemon, tray dependency, global hotkey, permanent active-window polling, or repeated restack loop.

A01-W1.1 differs from M03 global layers: it seeks relative z-order among selected managed ordinary windows and must establish compatibility with Openbox WM policy, re-raising/focus, window/group lifecycle and input isolation before any guarantee. If infeasible, explicitly fall back to proven Above/Normal/Below. Do not prebuild for it in M03.

## 9. Next action after accepted M03.1 and D00-M01

M03.1 planning, implementation, source review, CI and owner L01–L13 acceptance are complete. The D00-M01 documentation/context checkpoint is also accepted; follow `AGENTS.md` and `docs/STATUS.md` for current state, preserving earlier acceptance evidence and technical contracts.

The next work is a **read-only M03.2 design and code-seam audit**, using a Codex-capable model with High reasoning effort. Review this document's approved client clarification, accepted invariants, existing layer and pointer ownership, the host study and the actual code. Propose only managed hide/show, independently addressable same-instance control and Bring Top **without workspace movement**. Resolve request/reply confirmation, duplicate/stale owner races, lifecycle/focus/pointer safety and real-host tests. Present the plan for independent review; do not implement M03.2 or M03.3 until explicitly authorized.

## 10. Primary references

- docs/research/studies/R00-M01-nekoai-linux-host-a00-g3.md (completed target-host study)
- https://specifications.freedesktop.org/wm/latest-single/ (EWMH state/fullscreen/workspace)
- https://openbox.org/help/Actions (Openbox layer policy)
- https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html (managed iconification)
