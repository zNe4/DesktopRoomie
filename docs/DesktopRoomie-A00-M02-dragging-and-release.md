# DesktopRoomie — A00-M02: Dragging, release, and pointer recovery

> Lifecycle note (2026-10-08): ready-to-implement wording below describes the original brief, not current progress. Integrated M02 was subsequently accepted; see [M02 acceptance](../experiments/desktop-probe/ACCEPTANCE-M02.md) and [current checkpoint](STATUS.md). The original contract is preserved.

Version 0.1 · 2026-10-06 · Detailed implementation roadmap

**Status:** ready to implement; none of M02 is claimed implemented or tested by this document.

**Parent:** `docs/DesktopRoomie-A00-A01-plan-v0.1.md`.

**Prerequisite:** A00-M01 approved on the recorded Arch Linux / Openbox / X11 / Picom target.

**Reviewed starting commit:** [`23b13f74a0a3b4a1e8ee50a582717967b7e69403`](https://github.com/zNe4/DesktopRoomie/commit/23b13f74a0a3b4a1e8ee50a582717967b7e69403).

**Roles:** the client implements locally; the assistant specifies, reviews, and revises. This roadmap creates no repository changes by itself.

## 1. The result we are building

At the end of M02, the existing plain test body can be picked up with the left mouse button, moved around the selected display, and released anywhere the pointer can reach. It does not jump to the pointer center or become unreachable. A small mouse-only menu can be opened, dismissed, and used to quit. Every interaction returns pointer control to the desktop and preserves the application's typing focus.

The body remains stationary where released. Gravity, falling, walking, artwork, and autonomous decisions belong to later work. This is still the A00 technical demonstration, not the A01 companion release.

**Roadmap mapping:** this mission completes A00-G2.3, including its menu-dismissal requirement, and rechecks G2.1/G2.2 while the window moves. It does not complete A00-G3 or the whole A00 adventure. It supplies evidence for A01-G3.1 without prematurely implementing that companion minigoal.

### Included and deferred work

| Included in M02 | Remains for later missions |
| --- | --- |
| Click/drag distinction and stable grab offset | Physics, floor configuration, falling, and landing |
| Intentional pointer capture and reliable release | Autonomous movement, idle routines, and needs |
| Movement of the existing managed body | Climbing, perching, or moving other applications |
| Bounds on one selected monitor | Cross-monitor travel and user-selectable monitor UI |
| Cancellation and safe failure handling | Presentation layers, fullscreen policy, and explicit workspace placement |
| Small Dismiss/Quit menu without keyboard activation | Full settings menu, pause, sizes, persistence, and launcher recovery |
| Evidence from the real desktop | General support for bspwm, KDE, Wayland, or other hosts |

Adding the menu here is deliberate: A00-G2.3 asks us to prove both dragging and explicit menu dismissal. Its two actions exercise the same pointer ownership and cleanup mechanisms. A full companion menu remains separate.

## 2. Preserve the approved baseline

Continue in `experiments/desktop-probe/`. Keep the 160×160 procedural body, transparent padding, translucent patch, validated ARGB format, managed borderless window, input shape, and documented launch commands.

M01 already provides event-based socket waiting, `--delay`, `--duration`, no-input focus hints, and checked ordinary resource cleanup. Extend those mechanisms rather than replacing them with a polling loop or a new application framework.

The current environment record includes an external monitor, although the initial planning baseline had one display. M02 uses one selected display: the startup primary monitor, currently `eDP-2`. An extra monitor is useful for testing a release beyond the allowed body area, but is not a body-travel destination.

The recorded usable rectangle is `x=10, y=48, width=1910, height=1032`; rediscover the actual values at runtime. Neither those values nor the initial position are constants to copy into movement code.

**Invariant throughout this mission:** preserve no keyboard grabs, no global key listener, no `SetInputFocus`, and no activation request. Body movement or menu use must not acquire typing focus. Do not alter machine-wide Openbox focus settings to make a test pass.

Keep existing documentation. Add a separate `ACCEPTANCE-M02.md` so M01 evidence remains distinguishable from M02 evidence. Update shared environment facts only when they have actually changed.

## 3. Interaction agreement

These defaults make the first implementation concrete. They can be tuned after trying the result; changing them does not require rebuilding the whole architecture.

### Gestures and priorities

| Gesture | Required behavior |
| --- | --- |
| Left press on the circle or translucent patch | Begin a pending click, record the pointer offset, and acquire pointer capture. Do not move or change color yet. |
| Move less than 4 root pixels from the press | Remain a pending click. Small hand jitter does not move the body. |
| Reach at least 4 root pixels from the press | Become a drag. Move using the original grab offset and clamp the requested position. |
| Left release before becoming a drag | Release capture; toggle color once only if the release is still on an interactive shape. Otherwise cancel the click. |
| Left release after becoming a drag | Apply the final bounded target from the release position, release capture, and leave the body there. Do not toggle color. |
| Right click while idle | Open the small menu after the right-button release. This replaces M01's direct right-click quit. |
| Right press during a pending left click or drag | Cancel that interaction and release capture; do not open a menu or quit. Require a fresh gesture before starting again. |
| Middle button or wheel while idle | Log if useful; do not move, recolor, or open anything. |
| Middle button or wheel during a left gesture | Ignore the action. Releasing those buttons does not end the left gesture. |
| Outside click while the menu is open | Dismiss the menu, consuming that complete dismissing click; the next click works normally in the underlying application. |
| Duration expiry or ordinary quit | Cancel any interaction, release capture, close the menu, and perform checked cleanup. |

The drag threshold is based on distance from the original press, not distance from the previous motion event. Use squared distance `dx*dx + dy*dy >= 16` with a sufficiently wide signed type. Once a gesture becomes a drag, returning to the press position does not turn it back into a click.

The translucent patch is intentionally draggable too: it is already part of the proven interactive region. Transparent padding cannot start a gesture. During capture, release can arrive outside the body; that must still finish the gesture.

Opening the menu on release prevents its opening right press from accidentally selecting an item. Keyboard menu navigation and Escape dismissal are deferred; Escape must not be implemented with a global listener or keyboard grab.

### Minimal interaction state

Use an explicit enum or equivalent small state model. Suggested names describe responsibilities rather than a mandatory API:

| State | Information it owns | Exit conditions |
| --- | --- | --- |
| Idle | No active gesture or pointer ownership | A fresh valid body press |
| LeftPressed | Press root position/time, original offset, capture ownership | Threshold reached, matching release, cancellation |
| Dragging | Original offset, latest target, capture ownership | Matching left release or cancellation |
| RightPressed | Pending menu-opening gesture and its button | Matching release on the body or cancellation |
| MenuOpen | Popup geometry and pointer ownership | Item gesture, outside-dismiss gesture, cancellation |
| MenuPressed | Pressed item or outside-dismiss marker, matching button | Matching release, cancellation |

Represent cancellation as one operation returning to a neutral state; it need not become a permanent extra state. If a button is still physically down after cancellation, suppress stale events until a fresh press can start a new interaction. Do not let a late release toggle color or open a menu.

**One owner at a time:** body dragging and menu capture never coexist. Avoid independent booleans such as `is_dragging`, `menu_open`, and `has_grab` that can disagree. A small capture record can track whether a server grab was successfully acquired and whether release is still required.

## 4. Coordinates and valid space

### Coordinate contract

Keep three quantities explicit:

1. Pointer position in root/screen coordinates.
2. Actual body client-window origin in those same root coordinates.
3. Local coordinates used to hit-test the rendered shapes.

On press, derive `grab_offset = pointer_root - actual_body_origin`. On motion or release, derive `requested_origin = pointer_root - grab_offset`. Perform all calculations in signed coordinates before converting at the X11 boundary.

For example, if the actual origin is `(885, 484)` and the press is at `(955, 539)`, the offset is `(70, 55)`. A later pointer position `(1200, 700)` requests origin `(1130, 645)` before bounds are applied. The grabbed point remains under the pointer while unconstrained. At a boundary the body stops; the pointer remains free to travel farther.

Do not continually replace the offset as the window moves. Do not use event-local coordinates as if they were root coordinates. Determine the actual mapped origin before accepting the first drag: the WM may have adjusted the requested launch position.

### Bounds contract

For usable area `(left, top, width, height)` and body size `(body_w, body_h)`, valid body origins are:

```text
left <= x <= left + width  - body_w
top  <= y <= top  + height - body_h
```

Treat rectangle right/bottom edges as exclusive when testing containment. Clamp the entire 160×160 client canvas for this probe, including transparent padding. Later sprite bounds can be more precise; there is no need to introduce that asset contract here.

Use checked or widened arithmetic. A negative monitor origin is valid. An empty usable region or a region smaller than the body is an unsupported placement, not a reason to underflow an unsigned subtraction or return an invalid clamp interval. Report it and avoid leaving an active interaction.

The current layout helper silently falls back in several situations. Retain ordinary supported fallbacks where appropriate, but distinguish a genuinely invalid selected-monitor/workarea intersection for drag bounds. Do not silently expand the allowed drag area across the whole root because monitor discovery failed. A diagnostic-only run can still describe the failure.

Freeze the selected monitor identity for this run rather than reselecting whichever monitor becomes primary on every motion. On a relevant monitor/workarea change, cancel the gesture or menu, refresh bounds, and reconcile position. If the selected monitor disappears or the body no longer fits, exit cleanly with a useful diagnostic; automatic monitor migration is later scope.

Changing the user's workspace is not a request to follow them. Cancel capture/menu when the workspace changes, leave workspace policy alone, and refresh appropriate bounds when the body is visible again. M02 does not add a send-to-workspace action.

## 5. X11 host approach and error contract

### Pointer capture

Use an explicit, short-lived `GrabPointer` for a deliberate gesture. Start with the mapped body as the grab window, `owner_events = false`, motion and button events selected, asynchronous pointer and keyboard modes, and no pointer confinement. Check the reply status; a request that was sent is not necessarily a successful grab.

A denied grab is a recoverable failed interaction: report its status, return to neutral, do not move or toggle, and remain usable. A protocol/connection error is a fatal probe error: release owned input where possible, attempt cleanup, and exit nonzero. Neither path waits indefinitely for a release that it can no longer receive.

Use a valid event timestamp when acquiring capture. Normal completion can release using the completing event's time; cancellation without an appropriate event uses the protocol's current-time facility. Centralize release and check its request result. Avoid confusing ordinary boundary crossing with capture loss.

A held gesture has a low-frequency safety check, provisionally every 250 ms: inspect the initiating button's current state and cancel if its release was missed. This is a timer active only during a pending/held button gesture, not a permanent idle pointer poll. A stationary hold must remain a hold; elapsed time alone is not grounds for dropping the body.

### Managed-window movement

Keep the body managed. Put movement requests in the X11 host module, not in the pure gesture logic.

The first candidate is checked `ConfigureWindow` requests containing position only, with a verified position/gravity contract for the borderless Openbox target. Confirm the resulting root-space position. If that contract is not reliable, make one focused experiment with `_NET_MOVERESIZE_WINDOW`, checking support and its gravity/position flags. Use the application source indication and leave unspecified fields unset. Record the chosen mechanism.

Do not start a WM-owned interactive `_NET_WM_MOVERESIZE` operation while also holding the application's drag capture. M02 needs the application to own gesture completion and bounds. Do not add automatic raising or restacking to each move; layer policy remains later work.

Track requested and confirmed positions separately. Use configuration notifications and, when needed, root-coordinate translation to reconcile actual geometry. A normal notification after reparenting may use a different coordinate space from a synthetic one; do not treat every `ConfigureNotify.x/y` as root-space coordinates.

If Openbox refuses or materially changes a requested move, report the observation and cancel rather than repeatedly fighting the WM. Reject unexpected body size changes for this fixed-size probe or reconcile them safely; do not retain a stale hit mask and bounds calculation.

### Event loop and shutdown

Keep event-based waiting. The next wait deadline is the earliest of the finite-duration deadline and any active interaction safety timer. When idle with no finite duration, no periodic safety timer is armed. Menu hover reacts to events; it does not need continuous querying.

Check deadlines between bounded event batches so a burst of motion cannot postpone the duration fallback indefinitely. Coalesce consecutive motion events where useful, preserving button/lifecycle ordering. Flush the latest pending position before handling a release. Do not replay every queued motion as an expensive full repaint or synchronous round trip.

Process duration expiry, right-cancel, lifecycle changes, and errors through a common cancellation/cleanup path. Release capture before destroying its target; close the popup and release its resources too. Attempt every cleanup step even if an earlier step fails. Report the failure and use a nonzero exit on unsuccessful ordinary cleanup. Error-path cleanup remains best effort after reporting the original error.

Logging should record press, drag start, drag end/cancellation reason, capture status, final requested/confirmed geometry, menu open/dismiss, and cleanup results. Avoid printing every motion at normal verbosity. Log only probe events and required display metadata.

## 6. Ordered implementation minigoals

Each minigoal ends with the current probe runnable. Commit after its stated checks pass. Implement and review in order; unfinished later features do not prevent committing an earlier, clearly bounded checkpoint.

| Order | Minigoal | Runnable checkpoint |
| --- | --- | --- |
| M02.1 | Coordinate and bounds foundation | Existing probe still works; actual geometry and safe target calculations are available |
| M02.2 | Pending gestures and capture ownership | Clicks complete on release; pressing outside the body still passes through |
| M02.3 | Bounded dragging and final release | Body follows the held pointer without a jump and remains where released |
| M02.4 | WM confirmation and event efficiency | Confirmed geometry agrees with motion; movement leaves no trails or request backlog |
| M02.5 | Cancellation and host changes | Interrupted interactions return input control and cannot leave a stale hold |
| M02.6 | Small mouse-only menu | Menu opens, dismisses, and quits without activation or stuck capture |
| M02.7 | Combined acceptance and evidence | M02 has an honest, reproducible pass/fail record on the target desktop |

### M02.1 — Establish actual coordinates and safe bounds

**Purpose:** avoid building dragging on guessed placement or unsigned coordinate arithmetic.

1. Check the starting checkout and local changes. Preserve existing work; use the client's normal branch workflow.
2. Record the approved M01 baseline and rerun its short smoke checks before modifying interaction behavior.
3. Add small pure types/helpers for root points, body origin/size, grab offset, valid-origin range, and clamping. Reuse `Rect` where sensible; keep X11 IDs and connection types out of this logic.
4. Obtain the mapped body's actual root origin and size. Log requested versus actual startup placement once.
5. Validate the selected usable region and body fit. Keep body bounds separate from input silhouette.
6. Add meaningful geometry tests: exact four edges, all corners, a negative origin, a panel-reduced region, body exactly filling the area, and body too large/empty region.

**Done:** M01 still behaves as before, and target positions can be calculated without root/local confusion or invalid arithmetic. No dragging is claimed yet.

**Suggested commit:**

```text
refactor(desktop-probe): establish root coordinates and bounded body placement
```

### M02.2 — Separate clicks from held gestures

**Purpose:** establish input ownership before adding movement.

1. Add the small interaction state model and the 4-pixel threshold calculation.
2. On left press, record origin, root press position, offset, and time; acquire capture and inspect its reply status.
3. Move the color toggle from `ButtonPress` to a completed, valid left click on `ButtonRelease`.
4. Receive motion/release outside the original body area. At this checkpoint, exceeding the threshold can cancel the click without moving; full dragging comes next.
5. Ignore unrelated buttons as actions. A right press cancels an active left gesture; while idle keep M01's direct right-click quit until the menu checkpoint replaces it.
6. Centralize capture release for ordinary completion, cancellation, duration expiry, and shutdown.
7. Test meaningful transitions using a small host-action seam: denied acquisition, a matched release, irrelevant button release, and cancellation must not cause movement/toggle or duplicate ownership.

**Done:** a left click toggles once on release; release outside the body safely cancels; no capture remains afterward; the probe stays usable after acquisition failure. Transparent padding and typing focus still behave as in M01.

**Suggested commit:**

```text
feat(desktop-probe): distinguish clicks from captured pointer gestures
```

### M02.3 — Implement dragging and reliable release

**Purpose:** deliver the first new visible capability using the foundations above.

1. Promote a pending left gesture to Dragging when root displacement reaches the threshold.
2. Request origin from the preserved grab offset, clamping the whole canvas before sending each move.
3. Send position-only host requests; preserve dimensions, hints, visual, and shape.
4. On matching release, calculate the final target from that release event, then release capture and return to Idle.
5. Keep the body at the final confirmed location. No gravity, snap-to-floor, or restart of movement follows.
6. Try presses at several parts of the circle and at the translucent patch. Try fast movement and release outside the original footprint.
7. Verify that a gesture that became a drag never produces click color feedback, even if it returns to its starting point.

**Done:** dragging begins without a pointer-center jump, respects bounds, works across unrelated windows, and ends on left release. The pointer can move beyond the selected monitor while the body stops at its boundary.

**Suggested commit:**

```text
feat(desktop-probe): add bounded dragging with stable grab offsets
```

### M02.4 — Confirm movement and preserve desktop behavior

**Purpose:** ensure requested movement is actually what the WM displays.

1. Handle body configuration notifications and reconcile actual root position/size.
2. Confirm the chosen movement mechanism at the center and each boundary; document any required per-application host configuration.
3. If there is a persistent offset or rejected request, diagnose the coordinate/gravity contract before changing interaction math.
4. Coalesce motion sensibly; retain the final motion/release order and check deadlines between event batches.
5. Repaint on needed exposures rather than regenerating the body for every move. Recheck composited transparency and input shape at the new location.
6. Keep logging bounded and preserve idle socket waiting after the gesture ends.

**Done:** requested/actual positions agree within the explicitly observed host behavior; repeated dragging does not produce trails, flicker, a lasting backlog, unintended raising, or focus changes. A mismatch is recorded and handled, not hidden.

**Suggested commit:**

```text
fix(desktop-probe): reconcile managed movement and coalesce pointer updates
```

### M02.5 — Recover from interruption and changing bounds

**Purpose:** make dragging safe to interrupt instead of treating the happy path as the whole feature.

1. Route right-cancel, duration expiry, confirmed loss of the initiating button, unmap/destroy, workspace change, and fatal host errors through cancellation.
2. Add the interaction-only button-state safety timer. Do not cancel on an ordinary `LeaveNotify` or merely because the user holds still.
3. Subscribe to relevant RandR and root-property changes for geometry/workarea/workspace updates. Keep the chosen monitor identity stable.
4. On a relevant update, cancel first, refresh bounds, and request one safe reconciliation if placement remains supported. If the monitor disappears or no valid fit remains, exit with a clear diagnostic.
5. Make cleanup idempotent locally. Late events cannot complete an already cancelled click or revive a drag.
6. Test failed movement/release through the narrow host-action seam if practical; a deterministic injected failure must still attempt release/cleanup and produce the right outcome.

**Done:** cancellation restores normal desktop pointer use and preserves typing focus. Duration expiry still terminates a stationary hold. No lifetime-long mouse capture remains after the body becomes unavailable.

**Suggested commit:**

```text
fix(desktop-probe): release pointer capture on cancellation and host changes
```

### M02.6 — Prove a small menu and explicit dismissal

**Purpose:** finish A00-G2.3 using the same proven ownership rules.

1. Replace idle right-click quit with right-click-to-open after release; update help and run instructions at the same time.
2. Create a small, readable mouse-only popup with exactly two actions: **Dismiss** and **Quit**. Choose the simplest adequate label rendering; no UI toolkit, theme engine, or general menu framework is required.
3. Place the entire popup inside the selected usable area, flipping its anchor near edges. If it cannot fit, report and return to Idle without capture.
4. Keep the body managed. A short-lived override-redirect popup is permitted specifically for this menu experiment, following the ICCCM popup convention; it does not replace the body's host or focus test.
5. Never set focus or grab the keyboard. Map the popup and obtain its short-lived pointer capture as one checked operation; on acquisition failure, close it immediately and remain usable.
6. Route captured pointer coordinates to menu hit testing. Select on a matching press/release in the same row. A release outside the originally pressed item cancels that selection.
7. Dismiss on the Dismiss item, a fresh right click, or an outside click. Consume an outside-dismiss gesture through its matching release, then ungrab. Do not synthesize or replay a click into another application.
8. Quit cancels input, closes/frees popup resources, and uses normal checked body/renderer cleanup. Duration expiry and relevant lifecycle changes also close the menu.
9. Test opening/dismissing at all corners and repeatedly after dragging. Menu events must never become body drag events.

**Done:** menu interaction preserves typing focus; its first outside click dismisses and the next click reaches the underlying app; no invisible popup or pointer capture survives dismissal. Right-click no longer quits directly, but Quit remains readily available.

**Suggested commit:**

```text
feat(desktop-probe): add a non-activating pointer menu with reliable dismissal
```

### M02.7 — Verify the integrated mission

**Purpose:** close M02 on evidence from the executable, not on the existence of its code.

1. Run the compiler, formatting, lints, and meaningful pure-logic/transition tests listed below.
2. Complete the manual matrix on the recorded Openbox/Picom target. Re-run M01 smoke checks after the final implementation change.
3. Add `experiments/desktop-probe/ACCEPTANCE-M02.md` with actual outcomes, exact launch commands, tested revision, relevant environment differences, observed timings/resources, and named limitations.
4. Update `ENVIRONMENT.md` and help text only where behavior/facts changed. Reference the new report without rewriting M01 history.
5. Check that all successful shutdown routes say only what the executable knows, such as “Probe exited cleanly.” Manual pass/fail status belongs in the acceptance record.
6. Request review of the final commit. Mark M02 complete only after required checks pass and findings are resolved.

**Done:** the combined body/drag/menu loop works, known failures are explicit, and the evidence permits review without guessing how a check was performed.

**Suggested commit:**

```text
test(desktop-probe): record dragging and pointer recovery acceptance
```

## 7. Verification plan

### Automated checks

From the repository root:

```bash
cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check
cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path experiments/desktop-probe/Cargo.toml
cargo build --manifest-path experiments/desktop-probe/Cargo.toml
```

Test calculations and transitions that can fail meaningfully: bounds, negative coordinates, threshold crossing, click-after-drag suppression, release-button matching, failed capture, cancellation, and popup placement/selection. Use a small fake host outcome where necessary. Do not write tests that simply assert configuration constants or duplicate the implementation's lines.

Automated tests cannot establish real Openbox focus preservation, managed movement, or compositor behavior. Those remain manual acceptance gates.

### Launch commands

```bash
# Normal interactive run: right-click opens the menu after M02.6.
cargo run --manifest-path experiments/desktop-probe/Cargo.toml

# Delayed mapping while typing in another application.
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --delay 2 --duration 30

# Recovery deadline: use separately with no input, while held, and with menu open.
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --duration 5

# Preserve environment diagnostics.
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --diagnose
```

Keep a terminal-based stop path available during development. Ordinary `--duration` is the intentional fallback when pointer interaction is broken. Ctrl+C/process termination is additional development recovery, not evidence that explicit gesture cleanup works.

### Manual acceptance matrix

Use Pass / Fail / Not tested for each applicable row. Record steps and observed result, rather than copying the expected result into the evidence column. If the secondary monitor is unavailable, mark that row Not applicable with a reason; test release outside the body elsewhere on the same display.

| ID | Test | Required observation |
| --- | --- | --- |
| T01 | Delayed launch while typing | Existing application continues receiving typing as the body maps. |
| T02 | Transparent-padding click before/after moving | Underlying control receives the click; probe does not begin a gesture. |
| T03 | Body click with small jitter | One toggle on valid release, no movement, no keyboard activation. |
| T04 | Release a pending click outside its shape | No toggle; normal pointer control resumes. |
| T05 | Drag from multiple points and the translucent patch | Preserved offset; no jump to pointer center. |
| T06 | Cross the threshold, return, then release | Remains a drag; no click toggle. |
| T07 | Fast drag across another app and outside original footprint | Final release finishes correctly; subsequent app click/scroll works. |
| T08 | Drag toward all edges/corners and reserved panel space | Whole canvas remains in valid selected-display bounds. |
| T09 | Move pointer onto another monitor while dragging, then release | Body stays on selected display; capture ends even though pointer is elsewhere. |
| T10 | Type while holding/moving the body | Typing remains in the previously focused application. |
| T11 | Stationary left hold for at least 10 seconds | No timer-driven accidental drop; ordinary release still works. |
| T12 | Middle/wheel input during left hold | No action; unrelated release does not end the left gesture. |
| T13 | Right-cancel during left gesture | Gesture ends without click/menu/quit; stale releases cause no action. |
| T14 | Duration expires without input | Exits near its deadline; no periodic idle wake loop. |
| T15 | Duration expires while left held | Releases capture and exits; desktop input works immediately afterward. |
| T16 | Capture acquisition denied | No false Dragging state, movement, or stuck capture; later gesture works. |
| T17 | Open menu near every display corner | Readable popup fits entirely; typing focus is preserved. |
| T18 | Dismiss item, right dismissal, and outside dismissal | Popup/capture gone; outside dismiss consumes one click, next app click works. |
| T19 | Press menu item and release elsewhere | No accidental activation; menu remains safely dismissible. |
| T20 | Quit item and duration expiry with menu open | Popup/body/resources are released; no lingering capture. |
| T21 | Workspace change during a gesture/menu | Interaction cancels without initiating a workspace switch or follow behavior. |
| T22 | Usable-area/display change | Interaction cancels; safe placement is reconciled or unsupported fit exits clearly. |
| T23 | Body unmap/destroy or injected host error | No stuck hold; original error remains visible; cleanup is attempted. |
| T24 | Repeat drag/release/menu cycles at least 20 times | No accumulating jumps, stale state, orphan popup, or unreleased capture. |
| T25 | Leave idle after interactions | Event-based waiting resumes; record observed CPU and resident memory. |
| T26 | Ordinary exit after successful interactions | Checked cleanup succeeds, exit status is zero, normal desktop input resumes. |

T16 can use a small host failure injection rather than racing another app. Do not install a permanent global grab or modify unrelated WM rules just to create this test. T22/T23 can use a controlled probe test hook where reproducing the lifecycle safely is difficult; clearly distinguish injected and real-host evidence. Do not claim a physical unplug/reconfiguration test was performed if it was simulated.

Focus evidence should include observed typing and, where available, a direct input-focus observation. `_NET_ACTIVE_WINDOW` is useful supporting metadata but is not a complete substitute for verifying which application receives characters. The report should say whether any application-specific Openbox rule was required.

For T25, record the measurement method and observation interval. A rounded “0.0%” is a reading, not proof of zero memory or zero CPU work. Do not add permanent telemetry or global desktop monitoring for this experiment.

## 8. Repository organization and review handoff

Suggested boundaries, subject to what makes the current code simplest:

| Location | Responsibility |
| --- | --- |
| `src/interaction.rs` | Plain gesture state/transitions and requested actions |
| `src/geometry.rs` or existing small module | Root coordinates, offsets, fitting and clamping |
| `src/x11/window.rs` | Body hints, managed movement, actual geometry, window cleanup |
| `src/x11/pointer.rs` | Checked acquisition/release and interaction-only button-state observation |
| `src/x11/menu.rs` | Short-lived popup, geometry, painting, and popup resources |
| `src/x11/monitors.rs` | Selected display/workarea query and validation |
| `src/main.rs` | Configuration, event/timer routing, lifecycle, errors, and shutdown |
| `ACCEPTANCE-M02.md` | Actual verification record |

These are suggested boundaries, not a demand to create every file or a generic host trait. Keep helpers small; do not turn this experiment into a framework rewrite. A seam for a few host outcomes is enough for the relevant transition tests.

Rust owns these runtime interactions. A Python service, LLM, TTS backend, or extra language has no necessary job in M02. Polyglot architecture remains purposeful and will grow when a later capability requires it.

For each review, provide the commit, the minigoal ID, exact build/run commands, checks actually performed, and any observed failure. Do not mark the whole mission complete after its first successful drag.

For the final report, use this structure:

```markdown
# A00-M02 acceptance
Tested revision: <exact commit SHA>
Environment: <versions and relevant changes from ENVIRONMENT.md>
Movement mechanism: <chosen request path and coordinate/gravity contract>
Openbox rules required: <none, or exact scoped dependency>
Commands and automated results: <actual commands/results>
Manual matrix: <ID, status, steps, observed result>
Idle/resource observations: <method, interval, measured values>
Known limitations and untested cases: <explicit list>
Decision: <pending review / passed after review / blocked with reasons>
```

If the report is committed with the code it verifies, record the tested pre-document code SHA plus the final review commit; do not invent a self-referential commit SHA in advance. The review can identify the final revision after publication.

## 9. Start here: M02.1 only

The first programming assignment is **M02.1**, not all seven minigoals at once:

> Start from the approved desktop probe. Preserve the M01 behavior and local changes. Introduce signed root-coordinate and bounded-placement helpers, verify the mapped body's actual geometry, and add meaningful geometry tests. Keep the executable runnable. Do not implement dragging, pointer menus, physics, layer switching, or a new application architecture in this first commit. Return the commit and the geometry/build smoke-check results for review.

When that passes, M02.2 turns the safe coordinate groundwork into a held gesture; M02.3 turns that gesture into movement. The later minigoals close the host, interruption, and menu paths around that same capability.

## 10. Exit gate and decisions we can revisit

**M02 passes when:** bounded dragging, release outside the body, cancellation, mouse-only menu dismissal/quit, non-activation, baseline click-through/transparency, event-based idle waiting, and checked ordinary cleanup all work on the recorded target. Required evidence is present and all remaining limitations are named and reviewed.

Missing nonessential physical hardware tests can be explicitly accepted as limitations; unresolved stuck capture, focus theft, invalid placement, or broken quit/recovery are blockers. A passing unit suite alone cannot close those gates.

Then prepare a separate mission for A00-G3: deliberate above/below placement, fullscreen behavior, hide/show recovery, workspace movement, and the host decision record. M02 does not silently count those as passed.

Revisit threshold, popup presentation, cancellation feedback, motion batching, and file boundaries after the client tries them. Revisit the movement mechanism if actual WM evidence contradicts the candidate. If the menu grows beyond two actions or requires a framework, keep it bounded and split that work into a named follow-up; update the parent mapping so G2.3's unfinished portion remains visible.

## 11. Technical references

These references establish available mechanisms. They do not certify behavior on this installation. The interaction policy, minigoals, and acceptance procedures above are project design decisions.

- [x11rb 0.14.0 GrabPointer API](https://docs.rs/x11rb/0.14.0/x11rb/protocol/xproto/fn.grab_pointer.html): request/reply API and the need to inspect acquisition status.
- [x11rb 0.14.0 UngrabPointer API](https://docs.rs/x11rb/0.14.0/x11rb/protocol/xproto/fn.ungrab_pointer.html): explicit pointer release.
- [X11 protocol](https://www.x.org/releases/X11R7.7/doc/xproto/x11protocol.html), sections GrabPointer, UngrabPointer, QueryPointer, and TranslateCoordinates: pointer ownership, timestamps, button-state observation, and coordinate conversion.
- [ICCCM](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html), sections Input Focus, Pop-up Windows, Reparenting, and Window Move: focus model, short-lived popup convention, and managed-window geometry.
- [EWMH](https://specifications.freedesktop.org/wm/latest-single/), sections `_NET_MOVERESIZE_WINDOW`, `_NET_WM_MOVERESIZE`, and Window Geometry: the alternate programmatic move request and its distinction from a WM-owned interactive operation.
- [Linux poll manual](https://man7.org/linux/man-pages/man2/poll.2.html): socket readiness and timeout waiting, already used by the M01 probe.

The stable future benefit is a body that can be held and released safely. A01 can add performances and physics around that tested capability; a later brain can decide when to move without taking responsibility for X11 pointer cleanup.
