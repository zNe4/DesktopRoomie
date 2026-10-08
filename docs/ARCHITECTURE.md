# Implemented architecture

This describes the Rust/x11rb probe at reviewed executable revision `f8332ec99e876db1d809cd718dc76dff9fd38e27`, retained through the D00-M01 docs-only baseline. It is a responsibility map, not a proposed redesign. [STATUS](STATUS.md) identifies acceptance; [INVARIANTS](INVARIANTS.md) gives review obligations.

## State ownership and boundaries

All source links below are under [`experiments/desktop-probe/src/`](../experiments/desktop-probe/src/).

| Owner | Implemented responsibility |
| --- | --- |
| [main.rs](../experiments/desktop-probe/src/main.rs) | Argument parsing, X11 setup/diagnostics, initial monitor/bounds selection and resource construction; startup, event routing, deadlines and shutdown. `ProbeRuntime` owns interaction, pointer tracker, coalesced/final movement, confirmation/correction state, gesture safety, body availability, popup/generation and layer controller. The connection, managed body and renderer are coordinated alongside it. |
| [geometry.rs](../experiments/desktop-probe/src/geometry.rs) | Pure points/rectangles/sizes, root-space grab offset and targets, checked origin bounds/fit/centering, drag threshold, shared interactive silhouette, popup placement and row hit layout. No X11 ownership. |
| [interaction.rs](../experiments/desktop-probe/src/interaction.rs) | Pure body/opening/menu gesture state and host-action outcomes. Owns press data, offset, freshness/chord suppression and matching-release semantics; defines the canonical five menu items and labels. It does not acquire native resources. |
| [layer.rs](../experiments/desktop-probe/src/layer.rs) | Pure absolute layer transition controller: distinct desired target, last readable observation and pending removal/final phase with one fixed deadline. No pointer ownership or host transport. |

## Native X11 host

| Module | Host responsibility |
| --- | --- |
| [x11/window.rs](../experiments/desktop-probe/src/x11/window.rs) | `ManagedProbeWindow` owns body window/colormap and their release records, non-activating managed-window hints, fixed canvas, requested versus confirmed root origins and bounded movement histories. Position-only configure requests cooperate with the WM; geometry is queried/translated to root space for live reconciliation, visibility/workspace checks and correction. |
| [x11/pointer.rs](../experiments/desktop-probe/src/x11/pointer.rs) | Checked grab/ungrab and root button queries. `PointerCaptureTracker` owns one release obligation: explicit BodyLeft, automatic OpeningRight, or explicit Menu with window/generation. Denial clears acquisition; an unknown acquisition outcome or failed release retains an obligation. |
| [x11/menu.rs](../experiments/desktop-probe/src/x11/menu.rs) | Short-lived popup window, GC and font, measured labels, painting/hits and creation/acquisition generation/sequence gate. Pointer ownership stays in the runtime tracker. Fit refusal can recover without opening; protocol/cleanup failures are reported. |
| [x11/monitors.rs](../experiments/desktop-probe/src/x11/monitors.rs) | RandR 1.5 monitor discovery/identity and subscriptions, root workspace/workarea parsing and checked intersection. Startup picks primary (first active if none); refresh uses its name atom, not new primary status. Only absent workarea falls back to monitor bounds. |
| [x11/state.rs](../experiments/desktop-probe/src/x11/state.rs) | Layer atoms, strict capability/body-property parsing, viewability check and checked root-directed EWMH requests. An absent mapped-body property is unverifiable; present empty flags can mean Normal. Protocol acknowledgement is distinct from WM confirmation. |
| [x11/visual.rs](../experiments/desktop-probe/src/x11/visual.rs) | Finds a depth-32 visual/RENDER format with standard 8-bit ARGB channels. |
| [x11/render.rs](../experiments/desktop-probe/src/x11/render.rs) | `Renderer` owns pixmap/GC and color theme; generates and blits the procedural 160×160 body with premultiplied alpha and a translucent patch. Picom supplies visible compositing. |
| [x11/shape.rs](../experiments/desktop-probe/src/x11/shape.rs) | Builds the body's SHAPE input region from the shared silhouette, excluding transparent padding without removing its visual alpha. |
| [x11/resource.rs](../experiments/desktop-probe/src/x11/resource.rs) | Explicit Owned/Released/Unconfirmed release state and ordered cleanup attempts retaining the first cleanup error. Unconfirmed destruction is not blindly repeated or called successful. |
| [x11/mod.rs](../experiments/desktop-probe/src/x11/mod.rs) | Declares these host modules; no generic platform framework. |

The body is a borderless managed UTILITY window (`override_redirect=false`), with no-input hints, no WM_TAKE_FOCUS and user time zero. The popup is a short-lived override-redirect mouse surface, not a replacement host. No keyboard grab, activation request, fullscreen observer or permanent restacking policy is present.

## Event and deadline flow

Startup validates host geometry/fit, chooses an alpha visual, constructs body/renderer, maps and verifies placement before enabling input. Startup observes the mapped body's layer without changing it. The loop processes bounded batches of up to 64 events, routing lifecycle/layout interruptions, fresh layer observations, popup events and body gestures; it uses one-event lookahead to avoid flushing a coalesced move across an interruption. Ordinary release flushes movement in completion order; cancellation discards pending movement first.

Root coordinates drive drag targets and geometry confirmation; local coordinates drive silhouette and popup hits. Motion is coalesced before position requests. Body availability gates input: unmap/workspace interruption cancels interaction, remap requires live viewability/workspace, geometry and fit validation, and any one-shot bounds correction must be confirmed first. Destruction terminates the run and marks externally destroyed resources appropriately.

After a selecting popup's checked release and destruction, Above/Normal/Below dispatch uses fresh body/capability reads and at most one removal plus one addition. Already matching is a no-op; another selection while pending is busy. Later drag/popup ownership does not stall an existing layer operation. Relevant property notifications trigger fresh reads, not sequence-based layer confirmation. At the fixed one-second deadline there is a final fresh body observation, with no new mutation at expiry. Dismiss preserves pending layers; Quit, expiry and layout/body lifecycle abort them.

The loop checks deadlines even under sustained event traffic, drains buffered replies/events before waiting, flushes, then blocks in socket `poll` until an event or the earliest active deadline. Movement/correction confirmation uses 150 ms. Button-safety queries use 250 ms only during active button gestures (including chord suppression); ordered observations can cancel a missed release without inventing a click or final position. Idle has no permanent safety polling. Optional duration and layer deadlines share the same wait path.

Shutdown cancels interactions and attempts pointer release before popup/renderer/body teardown and connection flush. Cleanup continues after failures; the original fatal error has priority over later cleanup errors. Explicit release records prevent fabricated success. Detailed rationale/tests remain in [M02.5](../experiments/desktop-probe/M02.5-IMPLEMENTATION.md), [M02.6](../experiments/desktop-probe/M02.6-IMPLEMENTATION.md) and [M03.1](../experiments/desktop-probe/M03.1-IMPLEMENTATION.md).

## Deliberate absences

There is no Brain/Spine, runtime memory service, persistent simulation, cross-device abstraction, private character asset integration, autonomous motion or window-relative A01 stacking. Managed hide/show, independent same-instance control/recovery and explicit workspace placement are later gates. These absences do not authorize interfaces, source splitting or architectural expansion; use a bounded approved mission.
