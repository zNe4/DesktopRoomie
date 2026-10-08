# DesktopRoomie — A00 and A01 execution roadmap

Version 0.1 · 2026-10-04 · Planning draft based on the client's answers

Parent: DesktopRoomie roadmap v0.1. This document expands the first adventure and proposes a short technical prelude. It records proposed defaults where the client delegated the choice. It is a roadmap for implementation on the client's machine, not a report of implemented or tested software.

## 1. Product agreement

**A00 — Prove the desktop host:** a deliberately technical demonstration that resolves the platform risks.

**A01 — First visit:** a recognizable, pleasant desktop Vanilla who can be moved, interact physically, perform a small repertoire, and coexist with normal work. A01 must feel like a companion even though its decision-making remains simple.

The first supported environment is the client's current **Openbox on X11** session. The exact distribution/version, compositor, display geometry, and focus settings will be recorded in A00. There is no requirement to switch operating systems or desktops now. bspwm, KDE, and Wayland support remain separate compatibility work, without assuming that success on this target proves support elsewhere.

Rust owns body state, movement/action execution, settings, and the desktop application where practical. Python may help prepare/validate assets. No Python runtime or second language is added solely to satisfy a polyglot checklist. Later model services retain their planned role.

The client implements. The assistant prepares missions, designs interfaces, reviews evidence, and adjusts the plan. We do not begin coding the full adventure before its platform gate is resolved.

## 2. The boundaries we will build

### Capability first; motivation later

Manual controls, scripted demonstrations, and a small idle scheduler will call the same action interface. Later behavior systems can call it without reimplementing movement.

Initial conceptual actions include move to a point, stand/sit, show a sleep pose, begin/end a hold, react to pet/poke, and request a presentation layer. These are design names, not a frozen API.

The executor reports completion, cancellation, or failure. A new command cannot leave two conflicting movement actions active. User dragging overrides autonomous movement; pause and explicit user settings outrank the idle scheduler.

### Separate presentation concepts

| Concept | Meaning in this plan |
| --- | --- |
| Foreground | Draw above ordinary application windows, subject to the tested host policy |
| Background | Draw behind ordinary application windows while remaining part of the desktop scene |
| Hidden | Intentionally not visible or pointer-interactive |
| Keyboard focus | Which application receives typing; the sprite must not take this automatically |
| Pointer interaction | Mouse events on the visible sprite, or continued pointer capture during an intentional drag |
| Workspace | A virtual desktop; distinct from a physical monitor |
| Movement mode | Ground with gravity, or a bounded plane without gravity; independent of presentation layer |

Moving to the background is not moving off-screen, minimizing the character's internal state, or creating another body. Background means behind ordinary windows generally in A01. Placing her precisely behind Firefox but above another selected window is separate, later work.

### Proposed interaction and control defaults

- Left-button drag picks her up after a small movement threshold. Holding keeps her attached to the pointer. Releasing ends the hold reliably, including outside the original sprite region.
- A short left click produces a small poke/reaction. Pet is initially a deliberate right-click-menu action; a stroke gesture can be added after evaluating accidental activation.
- Right click opens a compact menu. Menu actions include Pet, Poke, Pause movement, size preset, movement mode, layer policy, workspace placement, Hide, Reset position, and Quit as the corresponding capability becomes available.
- Sprite hover, movement, animations, layer changes, and workspace changes never request keyboard activation. Pointer interaction should work while typing remains assigned to the previous application. Menu keyboard behavior is explicitly tested; no unexamined toolkit focus default is accepted.
- Pause stops autonomous displacement and falling. Manual repositioning and controls remain available. Resuming reevaluates the current valid position instead of replaying accumulated motion.
- Begin with three size presets, selected after viewing the artwork at the real display scale. Every preset must fit the allowed area.
- The default floor is the usable lower edge of the selected display. The user can move the floor upward within valid limits. Panels/reserved regions are accounted for where reported and verified.
- One selected monitor is the A01 movement area. No cross-monitor roaming. Manual “send to workspace” is included; automatic workspace travel and following the user everywhere are deferred.
- Foreground is the initial placement. A menu can force Foreground or Background. A character-controlled policy allows body layer requests; forced user policy wins. A01 demonstrates those requests with controls or a script.
- Proposed fullscreen default: yield to the fullscreen application and restore the previous presentation afterward without activation. The exact behavior is verified in A00, not assumed from a layer flag.
- A hidden or fully covered sprite must remain recoverable through a launcher/control command independent of right-clicking her. The exact mechanism is selected in A00; a custom global-hotkey system is unnecessary.

These are proposed defaults, adjustable during the first interaction review. The non-negotiable requirements are no unsolicited keyboard focus, reliable user control, persistent settings, and keeping the complete body within valid bounds.

## 3. A00 — Prove the desktop host

**Runnable result:** a small transparent test body with a deliberately plain appearance and controls for the platform features. It is the explicit exception to the companion-feeling requirement.

### A00-G1 — Establish the actual target

| Minigoal | Work | Acceptance evidence |
| --- | --- | --- |
| A00-G1.1 | Record session/WM versions, compositor or its absence, active display geometry/scaling, panels, workspace count, and focus configuration. Do not collect unrelated private desktop data. | A short environment record that can explain later differences. |
| A00-G1.2 | Create the smallest Rust build/run skeleton and a candidate host approach. Compare a second approach only if the first fails a named requirement. | One documented command launches and stops the test body on the user's machine. |

**Dependency:** none. Missing compositor/display facts are gathered here rather than guessed.

### A00-G2 — Prove unobtrusive rendering and input

| Minigoal | Work | Acceptance evidence |
| --- | --- | --- |
| A00-G2.1 | Draw a borderless transparent test image. Verify alpha rendering under the actual compositor configuration; establish a documented supported configuration if necessary. | No opaque rectangle or unwanted frame on the real desktop. |
| A00-G2.2 | Give the image a bounded interactive silhouette and pass input through empty areas. Keep the body non-activating. | Click a control in the transparent part of its bounding rectangle; the underlying app receives it. Type continuously while the body appears, moves, and is hovered. Typing stays in the original app. |
| A00-G2.3 | Prove pointer press/move/release handling and explicit menu dismissal without stuck capture or unintended keyboard activation. | Grab/release the test body, including a release outside its original area, then immediately use another app normally. |

**Dependency:** G1. No polished sprite pipeline is needed to prove these behaviors.

### A00-G3 — Prove placement and select the host

| Minigoal | Work | Acceptance evidence |
| --- | --- | --- |
| A00-G3.1 | Manually switch above/below ordinary windows; test fullscreen, hide/show, and recovery while covered. | A recording or checklist shows actual stacking and focus behavior, including unsupported cases. |
| A00-G3.2 | Move the test body between two virtual desktops, without changing the user's current workspace unexpectedly. Inspect usable bounds. | One instance changes workspace; it does not duplicate or steal activation. |
| A00-G3.3 | Record the selected renderer/host approach, required configuration, known limitations, and reusable boundaries. | A reproducible test build and a short decision record explaining why it meets the target. |

**Dependency:** G2. Layer and workspace tests can share this one executable.

**A00 exit gate:** transparency, reliable pointer handling, non-activation, recovery, and a valid on-screen area work on the target. Manual layering and workspace movement have an explicit pass/fail result.

If layering fails, try one identified fix or alternative with a clear hypothesis. The client has already allowed foreground plus hide as a fallback. Record that reduced scope clearly and retain background switching as pending; do not silently count it as implemented. A workspace failure likewise remains a named limitation for review. Focus theft or an unrecoverable input grab is not an acceptable fallback.

A00 ends when the capability matrix is resolved sufficiently to choose the A01 host. It does not expand into testing every WM or building a universal desktop abstraction.

## 4. A01 — First visit

**Runnable result:** a privately loaded chibi Vanilla who can be picked up, held, released, walk, sit, rest, react, and remain comfortably present while the user works.

The parent roadmap's G1/G2/G3 labels are retained. Each minigoal leaves the current executable runnable. Visual and interaction feedback can trigger small revisions before proceeding.

### A01-G1 — A reliable desktop home

| Minigoal | Implementation outcome | Completion check |
| --- | --- | --- |
| A01-G1.1 — User control | Promote the proven host into a single companion instance with launch, quit, hide/show recovery, and a minimal right-click menu. | Launch twice without creating accidental duplicate companions; recover her while hidden/covered; quit cleanly. |
| A01-G1.2 — A bounded space | Define the selected display's usable area, three provisional sizes, body bounds, and user-defined floor. | Position and size changes never place part of the visible body outside the valid area. A floor too high to fit her is rejected or safely constrained. |
| A01-G1.3 — Layer ownership | Implement the passing foreground/background capabilities, forced-user policy, character-request policy, and fullscreen behavior. | A scripted body request changes layers when allowed; a forced user choice blocks it. Keyboard focus remains unchanged. |
| A01-G1.4 — Workspace placement | Add manual send-to-workspace and a recovery command that can bring the body to the current workspace deliberately. | The user can locate and move the same instance without an involuntary workspace switch. |
| A01-G1.5 — Remember preferences | Persist size, floor, chosen movement mode, layer policy, workspace preference, and safe position. Validate stored values and recover from missing/invalid settings. | Restart restores valid preferences. A removed workspace or changed display geometry results in safe placement rather than an unreachable sprite. |

**Order:** G1.1 → G1.2 → G1.3/G1.4 → G1.5. An A00 fallback applies explicitly to the corresponding minigoal.

**Checkpoint:** a controllable, persistent desktop home using provisional art.

### A01-G2 — A recognizable, animated body

| Minigoal | Implementation outcome | Completion check |
| --- | --- | --- |
| A01-G2.1 — Visual and asset contract | Review the client's uploaded references; fix a small appearance sheet and consistent frame canvas/anchors. Define semantic animation names and necessary timing, loop, bounds, and interruption metadata. | One test animation loads through the same format intended for the private pack. Asset filenames are not hardcoded throughout behavior code. |
| A01-G2.2 — First visual set | Produce and inspect a high-resolution master set for idle/look, walk, sit/rest, and a sleep pose. Generate consistent runtime sizes using a repeatable command-line process when those assets exist. | No visible foot sliding from anchor changes, clipped ears/tail, frame jitter, or unreadable result at the chosen sizes. |
| A01-G2.3 — Interaction performances | Add held, falling, landing, and touch reactions as G3 needs them. Share suitable frames or use simple procedural motion when it looks good; every action does not require a large bespoke sheet. | Movement and animation agree in each interaction; transitions can be interrupted without broken poses or snapping. |
| A01-G2.4 — Pack quality | Check the complete repertoire at every preset and keep private Vanilla assets separate from public code/sample assets. | The private pack can be loaded intentionally; a suitable test pack can still demonstrate the application independently. |

**Dependencies:** G2.1 uses the host proven by A00 and can begin alongside G1. Runtime scaling needs G1.2. G2.3 is interleaved with G3 interaction work.

References are required before producing final artwork. The existing ChatGPT pet may guide the visual discussion, but a new desktop-specific sheet is expected. A static placeholder can unblock code; it cannot satisfy the final companion presentation gate.

Note: A private official chibi Vanilla Spine 2D export is now available (.skel, .atlas, .png) with multiple existing animations. Before designing a custom frame-sheet pipeline, A01-G2.1 should evaluate this asset's Spine runtime/version requirements and enumerate its usable animations, skins/attachments, origin/scale, and bounds. The copyrighted asset files remain private and must not be committed to the public DesktopRoomie repository.

**Checkpoint:** recognizable Vanilla with a small, coherent animation vocabulary.

### A01-G3 — Physical interaction and a little independent life

| Minigoal | Implementation outcome | Completion check |
| --- | --- | --- |
| A01-G3.1 — Pick up and hold | Implement explicit pointer states, grab offset, drag threshold, cancellation of current movement, and reliable release. | Pick her up at different points without a jump to the cursor center. Dragging stays within bounds and ends without a stuck hold. |
| A01-G3.2 — Fall and land | Add modest gravity, a user-defined floor, landing transition, and boundary constraints. Handle pause/resume and floor changes. | Drop from several heights; she lands on the configured floor with no clipping or accumulated-time leap after a pause. |
| A01-G3.3 — Directed movement | Execute move-to, stop, turn, sit, and rest commands through a small developer controller. Report completion/cancellation and validate destinations. | Send her to a point, interrupt by grabbing, and release. The cancelled walk does not unexpectedly resume as a second conflicting action. |
| A01-G3.4 — Plane mode | Offer an optional user-selected bounded 2D plane without gravity, using the same movement executor. It is a desktop-plane illusion, not 3D perspective or real-window navigation. | She can remain where placed and move toward a valid 2D destination. Returning to ground mode resolves her position consistently and keeps her within bounds. |
| A01-G3.5 — Deliberate touch | Connect the click/menu gestures to poke and pet performances, with hover/gaze where assets permit. | Petting and dragging cannot accidentally start each other; the visual response is immediate and coherent. |
| A01-G3.6 — A small idle routine | Add a replaceable, simple scheduler for quiet idles, brief walks, looking, sitting, and a sleep pose. Manual actions and pause override it. | Let her run without developer controls. She varies her behavior and responds to interaction without constantly wandering or restarting actions. |
| A01-G3.7 — Companion acceptance | Tune transitions and defaults during ordinary desktop use; exercise controls, layer policy, workspace placement, settings recovery, and display bounds together. Record resource use on the actual machine. | Complete the acceptance session below and resolve disruptive defects. |

**Order:** G3.1 → G3.2 → G3.3 → G3.4/G3.5 → G3.6 → G3.7. G3 requires G1 controls/bounds and the corresponding G2 assets. Early interactions can use provisional poses while final art is prepared.

Plane mode is a proposed small extension because the client asked for it and it reuses movement capabilities. If it proves larger than that, it becomes a named follow-up minigoal rather than delaying the core interaction loop indefinitely.

The sleep pose here is a body action. Needs-driven sleep, reduced senses, fatigue, hunger, preferences, and meaningful reasons to approach/retreat remain in later adventures. A01's scheduler is intentionally replaceable by those systems.

## 5. Suggested delivery sequence

| Checkpoint | Included work | What the client can try |
| --- | --- | --- |
| C0 | A00 | A technical body demonstrating verified platform capabilities |
| C1 | G1.1–G1.2; G2.1 | A visible body with controls, sizes, and valid space |
| C2 | G3.1–G3.2; first G2.2/G2.3 art | Pick up, hold, fall, and land on a chosen floor |
| C3 | G3.3/G3.5; G2.2/G2.3 | Direct movement, sit/rest, pet, and poke |
| C4 | G1.3–G1.5; G3.4 if retained | Layers, workspaces, plane mode, and restored preferences |
| C5 | G2.4; G3.6–G3.7 | The A01 companion release |

This sequence deliberately interleaves the three parent goals. It prevents finishing a large infrastructure goal before the client can try a satisfying interaction. Each checkpoint receives a bounded programming mission or a short sequence of missions; exact files, libraries, and commands are determined from the repository when that checkpoint starts.

## 6. A01 acceptance session

1. Launch while typing in another application. No focus change, unexplained window frame, or opaque rectangle appears.
2. Click through transparent regions around the sprite and continue normal work. Hover and autonomous movement do not activate the companion.
3. Pick her up, hold her, drag toward every boundary, and release. The complete body remains on-screen and the pointer is released correctly.
4. Change the floor and size. Drop her again and verify the expected landing position. Pause, reposition, and resume.
5. Request walking, sitting, petting, poking, and the sleep pose. Interrupt actions and verify coherent recovery.
6. Try plane mode if retained, then return to ground mode without losing her.
7. Switch foreground/background where supported, force a layer, and issue a conflicting body request. Verify user precedence. Hide her and recover her without clicking the sprite.
8. Move her to another workspace and locate her deliberately. Test fullscreen behavior and return to ordinary windows.
9. Quit/relaunch. Check saved preferences and safe restoration under an altered display/workspace setup.
10. Leave her running during a normal work session. Judge whether she feels pleasantly present. Review observed idle/animation resource use and fix noticeable interference before calling A01 finished.

Use a short repeatable smoke check at each checkpoint and the combined session for release. Focus, input release, bounds, and recovery are correctness gates; visual charm and pacing need the client's direct evaluation. Numeric performance targets are set from A00 measurements before optimizing speculatively.

## 7. Window interaction: preserved follow-up, not forgotten scope

Create an **A01-W extension** after the stable body, with its own acceptance gates:

| Order | Minigoal | Demonstration |
| --- | --- | --- |
| W1 | Observe the geometry and lifecycle of one user-selected ordinary window. | A debug outline tracks movement/resizing and notices minimize/close/workspace changes. |
| W1.1 | **Window-relative stacking (deferred feasibility):** attempt to place Vanilla above one selected ordinary managed window and below another without changing keyboard focus. | With two overlapping test applications, request and verify a bounded between-windows order; record whether Openbox maintains it across raise/focus, minimize/restore, close, and workspace changes, or safely fall back to global layering. |
| W2 | Perch on a selected reachable edge. | Vanilla stays attached while the window moves; if the supporting edge disappears or becomes invalid she returns safely to the floor. |
| W3 | Climb to that edge through an explicit action. | A manually requested climb has valid start/end positions, animation, cancellation, and fallback. |
| W4 | Move a selected window through an explicit permitted action. | A controlled test window moves as intended without trying to fight WM policy or affecting unrelated windows. |
| W5 | Request selected window operations with a matching performance. | A tested minimize/other operation accompanies the intended animation. Close needs its own explicit interaction design. |

W1.1 is a **z-order** capability, not geometric perching (W2) or automatically reading applications. Global Above/Normal/Below in A00-M03 is a prerequisite, but does not imply arbitrary interleaving among managed windows. An X11 relative-restack request is subject to Openbox policy and may not persist when the user raises or focuses another window. Research the relevant EWMH/X11 request path and real Openbox results just in time, including target identity, transient/group relationships, disappearing targets, conflicting layer policies, input/focus isolation, and event-driven reconciliation. Avoid constant restacking, undocumented WM tricks, and surveillance of unrelated windows. A stable sandwich is an aspiration until independently verified; fall back visibly to proven global layers if unsupported.

These are future goals, not promises that all windows expose usable titlebars or controls. Geometric perching and asking the WM to move a window are different capabilities. Literal titlebar-button clicking is an additional theme/application-dependent experiment, not a required implementation of semantic window actions. No autonomous manipulation is introduced just because the technical capability exists.

The future “she dislikes this video, retreats behind Firefox, then returns when hungry” scenario connects three later inputs: perception of content, a preference/need-driven decision, and the layer capability proved here. A01 can demonstrate the final layer action without pretending the earlier two systems exist.

## 8. Technical grounding and unresolved facts

Official documentation establishes available mechanisms, not successful behavior on the client's installation:

- Openbox's [per-application documentation](https://openbox.org/help/Applications) shows above/below layers and virtual-desktop placement. It supports choosing the current session as a first test target.
- The [EWMH specification](https://specifications.freedesktop.org/wm/latest-single/) defines stacking states, workspace information, frame extents, and move/resize requests. These are ingredients for experiments, not a universal guarantee of window control. Its above/below guidance favors user preferences; automatic layer changes should be an explicit user-enabled companion feature.
- The [ICCCM focus model](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html) distinguishes keyboard-focus policies. The host must be configured and tested to preserve the client's typing focus.
- The [X Shape extension](https://xorg.freedesktop.org/archive/X11R7.7/doc/xextproto/shape.html) provides independent input-region shaping. Transparency alone does not establish the input behavior we want.

Before the first coding mission, obtain:

1. The current distro and Openbox version, compositor name or absence, and focus policy.
2. Active monitor count, resolution/scaling, and any panel/dock reserving space. A01 will use one chosen monitor even if more are connected.
3. The reference images when starting G2; they do not block A00.

The A00-G1/G2 probe and integrated A00-M02 acceptance are complete. The next active assignment is **A00-M03.1**, as specified by [A00-M03: Placement, Recovery and Host Selection](DesktopRoomie-A00-M03-placement-recovery-host-selection.md); its first gate is a read-only Codex plan and independent review. A01-W1.1 remains deferred. No general architecture rewrite or complete sprite production is needed for M03.
