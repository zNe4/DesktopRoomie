# DesktopRoomie — A00-M01: The first desktop probe

Status: implementation brief, not implemented or validated.
Date: 2026-10-04.
Parent: A00/A01 execution roadmap v0.1.

## Purpose

Prove that a small Rust application can display a transparent, clickable body on the user's desktop without interrupting typing or blocking clicks through its empty area. This is the first executable checkpoint toward the companion.

This mission covers A00-G1 and the initial rendering/input portions of A00-G2. It does not complete A00: dragging, menus, layers, fullscreen behavior, workspace placement, and host selection review follow in subsequent missions.

## Confirmed environment

These facts were supplied by the client, not measured by the assistant:

| Property | Baseline |
| --- | --- |
| Distribution | Arch Linux |
| Window manager/session | Openbox, X11 |
| Compositor | Picom 13 |
| Active display | eDP-2, one monitor |
| Geometry | 1920×1080 at +0+0 |
| Observed focus behavior | Click-to-focus; hover alone does not focus |
| Reported X drivers | modesetting and nvidia |

The driver listing does not establish which GPU renders every surface. Scaling, Picom backend/configuration, Openbox version, panel reservations, and workspace count remain to be recorded locally. None requires a new product-design decision before beginning this mission.

## Candidate for this experiment

Use Rust with x11rb to test the X11 host requirements directly. Its official documentation describes Rust access to X11 and extension feature flags, including Render and Shape. Enable only the features needed for this probe. This is an experiment in desktop integration; choosing it does not commit the final animation renderer or portable core to raw X11.

Keep X11 operations together in a small module. The entry point handles startup, configuration arguments, errors, and shutdown. Avoid creating Brain, physics, model services, generic plugin systems, or a large multi-crate workspace at this stage.

Select a supported crate version against the installed Rust toolchain, record it, and commit Cargo.lock for reproducibility. Do not copy examples from inconsistent library versions. Reference: [x11rb documentation](https://docs.rs/x11rb/latest/x11rb/).

## Ordered minigoals

### M01.1 — Reproducible starting point

Inspect the local repository and preserve existing files and uncommitted changes. Create a working branch if that is the user's normal workflow. The remote inspected for this brief contains README.md and docs/DesktopRoomie-roadmap-v0.1.md; no application code was present at its root.

Create one small Cargo application, document its build/run command, and record the target environment and Rust/Cargo versions. Put the probe under a clearly temporary experimental location, such as experiments/desktop-probe/, so a technical test does not silently become the permanent core architecture. Keep the existing roadmap intact.

**Done:** the executable starts, connects to the current X11 session, and exits with useful diagnostics if that connection fails.

### M01.2 — Transparent body

Create a small, initially stationary, borderless managed window, approximately 160×160 logical pixels, positioned within the current visible screen. Use simple programmatically drawn colored shapes, with genuinely transparent empty space and a partially transparent test patch. No private character artwork is needed.

Discover a suitable alpha-capable visual/render format and matching resources rather than assuming the default root visual has alpha. Report an unsupported configuration clearly. Establish window hints before mapping where relevant. Keep the window manager involved: do not conceal managed-window/focus problems by immediately bypassing it with override-redirect.

**Done:** the desktop is visible through the empty region, the translucent patch blends correctly, and no opaque background rectangle or unwanted window frame appears.

### M01.3 — Mouse input without keyboard activation

Make the pointer input region cover the deliberate test shapes while excluding transparent padding. Configure the body for no keyboard input; do not request activation, take keyboard grabs, or use a global key listener. Log pointer entry, leave, and button events on the body. A click can change its color as visible feedback.

Do not apply machine-wide focus changes to make the test pass. If an application-specific Openbox rule proves necessary, record that dependency and its effect rather than changing unrelated window behavior.

**Done:** the body receives clicks on its interactive portion; clicking through its transparent padding reaches an underlying application; clicking or hovering over the body leaves typing assigned to the previously focused application.

### M01.4 — Recovery and evidence

Provide right-click-on-body to quit for this probe. Also provide an optional finite run duration so the test can exit even if stacking or input is broken. Avoid a custom menu or hotkey framework here. Release resources on ordinary shutdown, show useful errors, and avoid a busy polling loop while the body is idle.

Include a short startup delay option for testing whether mapping itself steals focus: launch the probe with a delay, focus a text editor, and keep typing while it appears. The delay and finite duration are test aids, not product behavior.

**Done:** the checks below have actual pass/fail results from the client's machine, with no claim that passing them completes the later layer/workspace tests.

## Acceptance checklist

| Check | Expected result |
| --- | --- |
| Build | Documented command succeeds using the recorded toolchain/dependency versions. |
| Mapping | The body appears during the delayed-start typing test without taking focus. |
| Transparency | Empty area is transparent and the translucent patch blends; no unwanted frame. |
| Body click | Color/event feedback occurs while keyboard focus remains with the prior application. |
| Hover | Pointer enters/leaves without activation. |
| Empty-region click | An underlying button/text field responds normally. |
| Exit | Right click quits; the finite-duration fallback also works when used separately. |
| Idle | The probe waits for events/timers rather than continuously spinning. Record observed resource behavior; set tighter budgets after a baseline exists. |
| Cleanup | After exit, pointer and normal desktop interaction work as before. |

Use cargo fmt --check and an appropriate compiler/lint check for the small application. The important evidence is the real Openbox/Picom session: a screenshot alone cannot prove keyboard-focus or click-through behavior. Record manual results, preferably with a short demonstration for any failure. Unit tests that merely repeat the probe's configuration constants are unnecessary.

## Return for review

Provide the changed files or repository commit, the launch command, environment notes, and the checklist results. Include exact errors for any failed capability. Keep logs limited to the probe's own events and required display metadata; no unrelated application contents are needed.

The next mission will add intentional dragging and reliable release, followed by explicit layer/workspace tests. The renderer choice becomes more settled only after those tests pass.

## Design addendum: future requests for attention

The client has proposed that Vanilla may later deliberately take window focus to seek attention. Preserve that option as a separate, explicitly user-enabled capability.

Distinguish a visible reaction, coming to the foreground, an attention indication, and actual window activation/keyboard focus. A future request-attention action can ask for one of these; user policy decides what is allowed. The default can remain non-disruptive while a later opt-in mode permits stronger interruption, with appropriate pacing.

A00/A01 still prove that the body can operate without unsolicited activation. This later option does not justify accidental focus theft in the technical probe. Its eventual window-manager support and behavior will need their own test.
