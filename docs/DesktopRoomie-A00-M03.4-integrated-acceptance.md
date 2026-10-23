# A00-M03.4 — integrated A00 acceptance

**Status: overall A00 technical acceptance OWNER-APPROVED on 2026-10-23; M03.4 closeout decision recorded under C3.** The owner accepted the original A00-G1/G2/G3 requirements as satisfied for the recorded supported target, with the disclosed limitations below. C1 prepared this evidence consolidation, C2 supplied focused corrections, and ChatGPT independently approved both C2 documents for their reviewed scope, as reported in the owner's C3 assignment. A00-M03.4-C3 documentation changes were independently reviewed and approved by ChatGPT on 2026-10-23. Final documentation publication and matching CI verification remain pending. The companion [host architecture decision](DesktopRoomie-A00-M03.4-host-architecture-decision.md) records the owner-ratified existing host. This closeout adds no host testing, implementation, publication authority or A01 authorization.

## 1. Revision and verification identities

| Identity | Exact revision / evidence |
| --- | --- |
| Repository and branch | `zNe4/DesktopRoomie`, `m03`; local root remains `/home/zzzne4/Desktop/Utils/ProjectVanilla`. |
| Accepted executable source | **`f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863`**, M03.3 F-T3; unchanged during Stage G native acceptance. This is a source revision, not a newly measured binary hash. |
| Matching executable CI | [39688537222](https://github.com/zNe4/DesktopRoomie/actions/runs/39688537222): successful formatting, build, all-target Clippy with warnings denied, **760 passed / 0 failed**. |
| Stage G starting documentation revision | `ceedb0aec261531279a76f6dbf93950ac8837a20`; [CI 39690049162](https://github.com/zNe4/DesktopRoomie/actions/runs/39690049162). Owner-native G0–G4 acceptance was recorded on 2026-10-22 against the frozen executable above. |
| C1 starting documentation HEAD | **`f15b9075f7f2b13c4a03649fbc201ca3753a9fdd`**, the final M03.3 Stage G documentation commit. |
| Final M03.3 documentation CI | [39863304379](https://github.com/zNe4/DesktopRoomie/actions/runs/39863304379): SUCCESS at `f15b9075f7f2b13c4a03649fbc201ca3753a9fdd`, with the same 760 tests passing and formatting/build/Clippy successful. This is distinct from the matching executable CI and from owner-native acceptance. |
| C1/C2 reviewed documents | Uncommitted additions on that HEAD; ChatGPT independently approved both C2 versions for their reviewed scope. No reviewed-document commit SHA was assigned. |
| C3 documentation closeout | Based on the same `f15b9075f7f2b13c4a03649fbc201ca3753a9fdd` HEAD, with no executable changes. These local documentation changes have no publication SHA or new CI result; C3 was independently approved by ChatGPT on 2026-10-23. |

C1 preflight found a clean tracked/untracked tree, the expected local branch/HEAD, and live `origin` branch `m03` at the same SHA. The complete accepted-executable-to-HEAD path diff contained only documentation changes; Rust source, Cargo manifest/lockfile and CI definitions were unchanged. GitHub job summaries and logs were read during C1 for runs **39688537222** and **39863304379**, confirming their recorded revisions, successful checks and 760-test results. Reading existing CI is not a new CI execution. Both logs record Rust/Cargo 1.99.1; older environment records identify their own historical 1.99.0 toolchain. The workflow follows stable Rust rather than a pinned toolchain.

[Stage G](DesktopRoomie-A00-M03.3-stage-G-acceptance.md) separately records owner-executed G0 local fmt/build/Clippy/tests passing. No Cargo command, diagnostic launch, native desktop check or host configuration change was performed for C1.

C3 preflight confirmed the expected repository, branch and local/live-remote HEAD, exactly the two untracked C2 documents and no tracked changes. Source, Cargo manifest/lockfile and workflow definitions still matched F-T3. C3 uses documentation-only verification; no fresh Cargo, CI or native result is claimed.

## 2. Evidence ledger and provenance

The original contract is [A00 in the A00/A01 plan](DesktopRoomie-A00-A01-plan-v0.1.md#3-a00--prove-the-desktop-host), refined by the [M03 mission contract](DesktopRoomie-A00-M03-placement-recovery-host-selection.md). Earlier dated “pending” language in preserved reports describes its original checkpoint; the later signed evidence supplies the corresponding disposition. No historical report is changed by this consolidation.

| Record | Accepted identity and contribution |
| --- | --- |
| [Environment](../experiments/desktop-probe/ENVIRONMENT.md) and [M01 acceptance](../experiments/desktop-probe/ACCEPTANCE.md) | Environment recorded 2026-10-05, M01 verified 2026-10-06. M01 identifies its executable by commit subject, `feat(desktop-probe): add exit controls and record acceptance results`, rather than a full SHA. Native alpha, borderless management, silhouette input, focus and event-wait/cleanup foundation. Do not assign it the later executable SHA. |
| [Integrated M02](../experiments/desktop-probe/ACCEPTANCE-M02.md) | `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b`, accepted 2026-10-07; 112-test matching CI and owner T01–T26 with explicitly controlled T16/T22/T23 evidence. Offset-preserving bounded drag, release/cancellation, mouse-only popup, cleanup and idle observations. |
| [M03.1](../experiments/desktop-probe/ACCEPTANCE-M03.1.md) | Corrected accepted `f8332ec99e876db1d809cd718dc76dff9fd38e27`, 2026-10-08, 139-test CI and owner L01–L13 PASS. Above/Normal/Below, natural fullscreen coverage/return, interaction/focus regressions. The earlier candidate is not the accepted revision. |
| [M03.2 integrated H acceptance](../experiments/desktop-probe/ACCEPTANCE-M03.2.md) and [Openbox evidence](host/OPENBOX.md) | Final H-A accepted 2026-10-18 at `a46e432c1f97b03c576737b1fc0ac35d7f90c158`, 453-test CI. Hide/Show, singleton/control transport and lifecycle. Earlier F/G native gates retain their own revisions, **not new H-SHA passes**: F `ccd01915bef13850067391f70f1524362a6440dc`, G3 `80d7df4aa1524122bd7c232f5aa09a4e39c8dd9f`. |
| [M03.3 Stage D](../experiments/desktop-probe/M03.3-D-ACCEPTANCE.md) | D4.1 `4c67da0b09ffe7b69605ac4a415d256acf8abd23`, accepted 2026-10-20, 677-test CI. Actual public Bring Here native evidence, preserving effective layer and genuine minimization. Original D4 was source-review blocked. |
| [M03.3 Stage E](../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md) | E4 `aa00f8e94aff1cf2ef1ff6a6cecf734f822d5603`, accepted 2026-10-21, 737-test CI. Public composed Recover; qualified native N01–N19, including genuine Busy and later admission. The invalid first N13 shell harness is not an application failure or a pass; the corrected bounded harness supplies that result. |
| [RF current closeout](DesktopRoomie-A00-RF-modularization-plan.md#current-closeout-2026-10-21) | RF2d `df5faa6ec17da464e1eeacfbb3c785688f60ea36`, 737-test CI and seven owner-reported native regression categories on 2026-10-21. RF1/RF2a–d are closed; RF2e/f remain deferred. |
| [Stage F coverage audit](DesktopRoomie-A00-M03.3-stage-F-coverage-audit.md) and [Stage F closeout](DesktopRoomie-A00-M03.3-stage-F-closeout.md) | Historical audit at `0b0f1b649460f0150adfccfbea17fedde010dd93` found composed coverage gaps, no established production defect. Four accepted test-only checkpoints added 23 identities, losing none: 737→760 at F-T3. Lifecycle/Closing, late-event turnover and local-layer contention gaps closed at deterministic scope. No new native Stage-F acceptance. |
| [Stage G acceptance](DesktopRoomie-A00-M03.3-stage-G-acceptance.md) | Owner-approved G0–G4 on 2026-10-22 on unchanged F-T3: integrated Here/Recover, interaction, genuine Busy, original-workspace Show, singleton and lifecycle at the exercised single-monitor Openbox/X11/Picom scope. Its explicit unexercised matrix remains binding. |

### Supplemental fullscreen observation

During M03.4 planning **before C1**, the owner reported a supplemental fullscreen **PASS**: **fullscreen behavior still worked as expected on October 22, 2026**. Codex did not newly test fullscreen in C1, C2 or C3. This remains a supplemental owner-reported observation. No new log, exact executable/binary identity, application/procedure detail or independent verification was supplied for it. It is not added to Stage G's exercised checks: Stage G explicitly did **not** exercise fullscreen precedence. Historical M03.1 L07/L08 remain the direct native visual evidence for natural coverage and return; M03.2 G3 H25 separately records owner acceptance and focus/state snapshots, whose log did not itself establish visual overlap.

## 3. Original A00 requirement mapping

The owner approved original A00-G1/G2/G3, including all eight minigoals below, as satisfied for the recorded Openbox/X11/Picom target on 2026-10-23. The mapping retains each earlier evidence identity and scope; overall acceptance does not convert untested configurations or injected cases into native passes. G3.3 is closed by the ratified host decision and integrated acceptance, with C3's documentation changes independently approved by ChatGPT on 2026-10-23.

| Original requirement | Accepted evidence and integrated disposition |
| --- | --- |
| **A00-G1.1 — actual target record:** session/WM, compositor, geometry/scaling, panels, workspaces, focus | M01 environment plus dated M02 reconciliation; later M03.2 H and Stage G host snapshots. Archcraft/Arch, Openbox 3.6.1, X11, Picom v13, Polybar/Plank observations, five desktops and click-to-focus are recorded at their respective dates. The original contract requires a recorded target configuration, including geometry/scaling, but not a complete Picom configuration. Pixel geometries are recorded; a separate display scaling factor is **not specified**. On 2026-10-23 the owner accepted the [untested scaling qualification](#7-supported-host-envelope-and-qualifications) without additional host tests. Do not infer 1× scaling, a particular backend or scaling compatibility. |
| **A00-G1.2 — smallest Rust skeleton and candidate host, documented launch/stop** | M01 build/run/exit evidence and the reproducible commands below. The existing Rust/x11rb managed host met the tested capabilities; no second approach was required by the conditional comparison rule. Quit and finite duration are the present checked ordinary stop paths. |
| **A00-G2.1 — borderless alpha rendering under the compositor** | M01 checklist 3 native visual PASS: transparent padding, blended translucent patch, crisp body, no frame. Later input/visual regressions and Stage G preserve the exercised host; compositor replacement or arbitrary Picom configurations are not verified. |
| **A00-G2.2 — bounded silhouette, empty-area click-through, non-activation** | M01 input/hover evidence; M02 T01/T02/T10; M03.1 L01/L09/L10; Stage D/E interactions and Stage G G2-A/D. Actual continued typing and physical underlying-app clicks complement metadata. The translucent patch is intentionally interactive. |
| **A00-G2.3 — pointer press/move/release and explicit menu dismissal** | M02 T03–T21/T24/T26 and accepted controlled T16/T23; Stage G G2-B/C/D. Grab offset, release outside the original body, neutral cancellation, consumed outside dismissal and subsequent normal input are accepted at their recorded scopes. Denied grabs and real server loss have different evidence types. |
| **A00-G3.1 — manual layers, fullscreen, hide/show, recovery while covered** | M03.1 L02–L08 native layering/fullscreen; M03.2 H and earlier F/G Hide/Show/Bring Top gates; public Recover Stage E and Stage G. CLI works independently of the exposed menu. The supplemental fullscreen report above is separately qualified. Above is an effective WM layer, not absolute stacking or fullscreen precedence. |
| **A00-G3.2 — same instance across two desktops, unchanged user workspace, usable bounds** | Public Stage-D N01–N09/N14–N16 and Stage-G G1-B–I demonstrate same-body relocation between virtual desktops, preserved Here layer/minimization and focus/current-desktop isolation. Retained-monitor/workarea checks and deterministic invalid-layout tests supply bounds evidence. Timed real workarea drift and hotplug are not native passes. |
| **A00-G3.3 — host decision, configuration, limitations, reusable boundaries and reproducibility** | This acceptance record, the owner-ratified [host ADR](DesktopRoomie-A00-M03.4-host-architecture-decision.md), [implemented architecture](ARCHITECTURE.md), [invariants](INVARIANTS.md) and [Openbox guide](host/OPENBOX.md) record the selected existing approach. ChatGPT approved the C2 documents for their reviewed scope; **owner signoff is recorded on 2026-10-23**. C3 documentation changes were independently approved by ChatGPT on 2026-10-23; publication and matching CI verification remain pending. |

## 4. Capability and evidence matrix

Native and deterministic columns are separate proof sources. A limitation or unexercised case is not converted into a pass by appearing beside a passing row.

| Capability | Accepted native behavior | Deterministic / injected evidence | Known limits, optional or deferred checks |
| --- | --- | --- | --- |
| Rendering and silhouette | M01 composited alpha/frame/input checks; later padding interaction including Stage G G2-D. | Shared geometry/input-region and post-map synchronization coverage. | Tested Picom environment only; no new compositor/backend/scaling matrix. |
| Keyboard-focus isolation | Actual typing in M02, M03.1, D/E and G2-A; no observed focus theft/user-workspace switch in exercised commands. | No-input hints and nonactivating request envelopes; no keyboard grab/activation path. | Hints and `_NET_ACTIVE_WINDOW` snapshots alone cannot prove typing continuity on a new host/configuration. |
| Pointer and popup safety | M02 drag/release/right-cancel/dismiss/expiry; Stage G popup/held-drag recovery and later usable input. | Capture denials/unknown acquisition, release debt, cleanup failures, stale/chord input and generation gates. | No exhaustive physical grab-denial or server-failure races; startup input is not universally disabled until Ready (optional refinement deferred). |
| Valid selected-monitor placement | M02 bounded edge/panel drag and T09 release on a temporary second monitor without body migration; D/E retained-monitor observations; G stable single-monitor geometry. | Checked fit/coordinates, retained name-atom identity, removed-monitor error, one-shot verified correction and strict destination agreement. | Physical monitor removal/hotplug and deliberately out-of-bounds/different-workarea corrections were not fully natively exercised. |
| Above / Normal / Below | M03.1 L02–L06 and later retained-layer tests, including G1. | Absolute layer predicates, malformed/conflicting flags, support checks, finite removal/addition and failure cases. | No exact Normal-peer ordering or arbitrary selected-window sandwich guarantee; no restack loop/rollback. |
| Fullscreen coexistence | M03.1 Firefox F11 naturally covers Above body; exit naturally returns it. M03.2 G3 H25 has its own qualified acceptance. Supplemental 2026-10-22 owner report above. | Layer protocol does not request activation or implement fullscreen surveillance. | **Not exercised in Stage G.** No guarantee over all fullscreen apps, exclusive modes or other WMs. |
| Managed Hide / Show | M03.2 local/off-workspace HIDDEN transitions; Stage G G3-C1–C3 preserves original workspace/layer and restores interaction on return. | Coherent ICCCM/EWMH classification, input fencing, dispatch guards, geometry and deferred restoration. | Openbox Iconic/unmapped/no-HIDDEN elsewhere is not genuine minimization. Remote Show may succeed with placement/input deferred; Show cannot raise an already visible covered Below body. |
| Independent Bring Top | M03.2 G3 confirms not-minimized + Above on existing body desktop; Stage G G3-B confirms later command use. | Show→Above composition, qualified remote layer reader, fixed cutoffs and confirmed-restoration Partial. | Does not relocate to caller workspace or establish absolute/fullscreen precedence. Off-workspace input/placement remain deferred. |
| Explicit Bring Here | D4.1 public command and G1 visible/all-layer and genuinely minimized relocation. | Frozen destination capture, strict workarea/viewport/count/monitor checks, single send/correction, proof/history and refusal/error paths. | Genuine minimization stays minimized; geometry/input may be deferred. No automatic workspace following, physical carrying across workspaces or monitor migration. |
| Composed Recover | E4 and G1/G2: same body moves to frozen caller workspace, restores when needed, becomes Above and usable without activation. | One-way Here→Top, fresh current proof, preserved history/consumed authority, fixed phase/total deadlines and fatal paths. | Not every Partial or OutcomeUnknown branch or phase-boundary failure was physically reproduced. No resend/rollback on uncertainty. |
| Singleton, concurrency and lifecycle | M03.2 and G: duplicate exit 4, no-owner exit 3, Quit/expiry/relaunch and SIGKILL cleanup/reacquisition. G3-A: one Success, seven Busy, then fresh commands succeed. | Owner epoch/marker/correlation, all command pairs, selection/control/body loss, Closing errors, local-layer contention and late-event turnover. | SIGKILL server-side resource release is not application checked cleanup or a real X-server disconnection test. Busy evidence is not exhaustive scheduling coverage. |
| Truthful results and event-driven operation | M03.2 H21 includes one native exit-8 OutcomeUnknown at expiry. M02 T25 and H26 record no tick delta in bounded idle samples; D/E passive snapshots and G lifecycle are separately scoped. | Lost replies, dispatch uncertainty, current proof versus history, no retries, fixed deadlines and >64-event helper/service sequences. | Stage G did not exercise reply loss/all Partial paths or native deadline storms. Tick measurements are process-only at counter resolution; passive snapshots do not prove no polling. Full `run_probe` under arbitrary traffic is not executed by injected harnesses. |

M03.2 **H14** remains an unobserved alternate off-workspace representation; **H23** remains conditional/not exercised because Polybar IPC did not change `_NET_WORKAREA`; **H28** optional Openbox shortcut smoke was not exercised. Optional shortcuts are not a prerequisite for independent CLI correctness. The [F audit](DesktopRoomie-A00-M03.3-stage-F-coverage-audit.md) and [F closeout](DesktopRoomie-A00-M03.3-stage-F-closeout.md) qualify scheduler and injected evidence; Stage F's coverage-gap G1–G3 labels are distinct from Stage G's native G0–G4 labels.

## 5. Integrated architectural invariants

The accepted [invariants](INVARIANTS.md) and [source ownership map](ARCHITECTURE.md) remain review obligations:

1. **One body and explicit authority.** One managed fixed 160×160 procedural body; one cooperative owner/control endpoint, one `ControlWindow.active` operation slot and one `ProbeRuntime` layer controller. Local/remote adapters borrow those owners; no queue, duplicate body or nested caller is added.
2. **Focus and input isolation.** No-input hints, no `WM_TAKE_FOCUS`, user time zero, no keyboard grab/focus/activation request or unexpected user-desktop switch. Transparent padding remains click-through; the translucent patch remains interactive. Popup outside dismissal consumes its gesture without replay.
3. **Capture and cleanup truth.** Body/popup pointer owners cannot coexist. Checked acquisition/release and unresolved release debt govern transitions; cancel stale interaction before teardown. Attempt remaining cleanup after failure, preserve the original error and distinguish Released from Unconfirmed destruction.
4. **Checked geometry and retained identity.** Root coordinates govern movement/confirmation; local coordinates govern hits. Fit the full canvas within selected-monitor/workarea bounds. Retain monitor name-atom identity; removal is an error. Here/Recover require strict complete destination evidence, absent/all-zero viewport policy and two-read agreement, without silently borrowing legacy workarea fallback.
5. **Observed evidence governs success.** A checked X request or event is not proof of WM geometry, visibility, desktop or layer success. Here preserves effective layer and genuine minimization. Recover clears current proof at handoff, retains only real confirmed relocation/restoration history and rebuilds all final postconditions. No-op/acknowledgement/correction alone cannot justify Partial.
6. **Bounded authority.** Caller destination is frozen before discovery; identity, operation deadlines and consumed correction attempts survive handoff and events. At most one desktop request and one correction for Here/Recover, with a full 150 ms correction confirmation interval; no retry/rollback or new mutation after its cutoff. Required safety cleanup and bounded final read-only observations remain allowed.
7. **Event/lifecycle coordination.** Main-loop scheduling services admitted work before sleep and checks deadlines through bounded event batches. Input is re-enabled only through applicable fresh visibility/geometry/fence checks. No permanent idle polling, automatic following or fullscreen reassertion loop. Clean Closing, actual OwnershipLost and native failure remain distinct; one terminal attempt per admitted operation, cleared slot and honest caller uncertainty.
8. **Reuse without speculative infrastructure.** Pure geometry/interaction/layer/control logic stays separate from X11 resources and runtime coordination. RF2e/f remain deferred. Public engine code contains synthetic content; private character assets, simulation, persistence and Brain/Spine/services are outside A00.

## 6. Reproducible commands

Run from the repository root. These are reference procedures, **not C1/C2/C3 Cargo or native execution results**. First verify the source identity; an empty path diff means unchanged definitions, not a new native test or a byte-identical binary.

```sh
git branch --show-current
git rev-parse HEAD
git status --short --branch
git diff f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863 HEAD -- experiments/desktop-probe/src experiments/desktop-probe/Cargo.toml experiments/desktop-probe/Cargo.lock .github/workflows

cargo build --manifest-path experiments/desktop-probe/Cargo.toml
cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check
cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path experiments/desktop-probe/Cargo.toml

cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --diagnose
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --delay 2 --duration 30
```

Diagnostics and rendering require access to the actual X11 session; visible alpha additionally needs the compositor. In a second terminal in that same session, use the actual binary produced by the build (the path below assumes Cargo's default target directory). Each control invocation takes one command and does not create an owner when none exists:

```sh
PROBE_BIN="$PWD/experiments/desktop-probe/target/debug/desktop-probe"
"$PROBE_BIN" --help
"$PROBE_BIN" --hide
"$PROBE_BIN" --show
"$PROBE_BIN" --bring-top
"$PROBE_BIN" --bring-here
"$PROBE_BIN" --recover
```

These commands are alternatives for deliberate checks, not an automated acceptance sequence. Hide/Show preserve body workspace and effective layer; Top establishes Above there; Here preserves genuine minimization/layer while relocating to the caller-frozen desktop; Recover composes Here→Top. Control flags are mutually exclusive and cannot combine with owner delay/duration or diagnose. There is no remote `--quit` or remote `--layer` flag. Right-click/release opens Above / Normal / Below / Dismiss / Quit; matching left press/release selects a row. Quit or configured duration gives checked ordinary shutdown; forced termination is a separate lifecycle case.

| Operation | Owner budget | Caller budget |
| --- | --- | --- |
| Hide | 1000 ms | 4000 ms |
| Show | 2500 ms | 4000 ms |
| Bring Top | 3500 ms; visibility cutoff at admission +2500 ms, layer ≤1000 ms capped by owner deadline | 5000 ms |
| Bring Here | 3500 ms | 5000 ms |
| Recover | 7000 ms total; Here ≤3500 ms, Top visibility ≤2500 ms from validated handoff, layer ≤1000 ms, all capped by total | 9000 ms |

Budgets assume responsive synchronous X11 replies; they are not independent hard transport timeouts. Exit meanings in [protocol source](../experiments/desktop-probe/src/control/protocol.rs): **0** Success, **1** local failure, **2** usage, **3** no instance, **4** duplicate launch, **5** Busy/Starting/Closing, **6** protocol failure, **7** operation failed, **8** OutcomeUnknown, **9** Partial. A nonzero expected no-owner/Busy result is not a native test failure by itself. Inspect diagnostics; Unknown does not authorize resend or imply the owner's terminal result.

## 7. Supported-host envelope and qualifications

The accepted first host is **Archcraft/Arch Linux, Openbox 3.6.1, X11 and running Picom v13**; historical Picom revision `d87a5ba` is recorded in M01/M02, not freshly measured in Stage G. Required facilities and the Openbox-specific HIDDEN behavior are detailed in the [ADR](DesktopRoomie-A00-M03.4-host-architecture-decision.md) and [host guide](host/OPENBOX.md). No custom shortcut, special new Openbox application rule or dotfile edit is established as required by this evidence. Exact Picom backend/configuration and a separate scaling factor are not preserved in the reviewed records. A complete Picom configuration is not an original A00 requirement. The owner accepted the following qualification on 2026-10-23 without requiring additional real-host tests:

> Display scaling was not explicitly tested or independently verified during A00. The accepted native results establish correct behavior for the recorded pixel-coordinate configurations, but do not establish compatibility with arbitrary scaling factors, mixed-DPI monitors, or other display-scaling arrangements. This limitation is accepted for A00 closure; further validation may be undertaken when A01 requires it.

No scaling value or compatibility claim is inferred. Supplemental owner-native metadata collection remains optional future work and was not performed in C1, C2 or C3.

Environment snapshots are dated, not interchangeable: M01 had eDP-2 plus HDMI-1-0; M02 T09 temporarily used a second display but ended on one eDP-2; M03.2 H and Stage D used eDP-2 plus DP-1-0 at 1920×1080 each, with E4 owner-observed retention; Stage G used **one eDP-2 at 1920×1080 +0+0 and five desktops**. Do not attribute the older dual-monitor root workarea `10,48,3820,1022` to Stage G. This is a selected-monitor probe, not cross-monitor roaming or mixed-DPI support.

The following are **not completely physically verified** by this package: live monitor hotplug/removal; forced workarea or viewport/count changes; real X-server disconnection; every WM refusal/malformed reply; every Partial/OutcomeUnknown branch; exhaustive deadline/scheduling interleavings; other WMs or Wayland. Stage G additionally did not exercise fullscreen, compositor/WM replacement or terminal-reply loss. Historical native H21 Unknown, fullscreen and multi-monitor evidence stay attached to their own records. None fills all corresponding fault/compatibility cells.

Known behavior remains explicit: Below can cover the only menu; Show preserves effective layer rather than peer order; Bring Top/Recover Above does not defeat fullscreen; ordinary workspace switches cancel/suspend interaction rather than carry the body with the pointer or automatically follow the user. Strict Here/Recover reject sticky/all-desktop placement and unsupported nonzero viewport models. Synchronous replies can stall beyond logical deadlines. X11 private control is cooperative same-session IPC, not a security boundary against other X clients. Real connection-loss cleanup is best effort, without fabricated server acknowledgements.

A01 animation/assets, character movement, gravity, floor/size settings, autonomous behavior, persistence and physical carrying across workspaces are not A00 deliverables. Existing technical drag and explicit CLI relocation do not claim those companion behaviors.

## 8. Owner signoff and documentation closeout

The owner's C3 assignment formally records **overall A00 technical acceptance on 2026-10-23**, following ChatGPT's independent approval of both C2 documentation drafts for their reviewed scope. C1 evidence consolidation and C2 focused corrections remain the reviewed provenance; no commit SHA exists for those local versions. The owner's C4 assignment reports independent ChatGPT approval of the C3 documentation changes on 2026-10-23. Final documentation publication and matching CI verification remain pending.

Original A00-G1/G2/G3 are satisfied for the supported target: recorded environment, reproducible Rust host, transparency, pointer recovery, non-activation, independent recovery, valid bounds, manual layers and same-instance workspace placement. The owner accepts the scaling qualification and all disclosed native/deterministic/compatibility boundaries in sections 2–7; no additional real-host tests are required for A00 closure. Historical accepted reports and failed candidates remain unchanged. A00 proves the technical host, not the complete desktop companion.

| Signoff field | Current disposition |
| --- | --- |
| Independent integrated document review | **C2 documents approved by ChatGPT for their reviewed scope**, as reported by the owner; uncommitted versions, no reviewed-document SHA. **C3 documentation changes independently reviewed and approved by ChatGPT on 2026-10-23.** |
| Executable accepted for A00 | `f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863`; [CI 39688537222](https://github.com/zNe4/DesktopRoomie/actions/runs/39688537222), 760 tests and all checks PASS; existing M03.3 native acceptance remains distinct |
| G1.1 metadata disposition | **Accepted with the explicit untested display-scaling qualification** on 2026-10-23; no guessed scaling factor, complete Picom configuration requirement or new host test |
| Additional native evidence for A00 closure | **Not required**; optional further validation may be considered when separately authorized A01 work requires it |
| Owner, acceptance date and decision | **OWNER-APPROVED — overall A00 technical acceptance, 2026-10-23**, recorded from the owner's C3 assignment |
| Accepted limitations / follow-up authorization | All disclosed boundaries in sections 2–7 retained and accepted for A00; no new implementation or compatibility work authorized |
| Publication revision / authority | **C4 authorizes this five-file documentation commit/push only; final documentation publication and matching CI verification remain pending. No publication SHA or CI outcome is asserted here.** |
| Next adventure | **A01 requires separate owner authorization**, including its monitor/workspace and presentation/behavior decisions |
