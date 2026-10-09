# Current checkpoint

Updated 2026-10-09 after approval of the [A00-M03.2 design](DesktopRoomie-A00-M03.2-approved-implementation-plan.md) and its [eight-stage implementation roadmap](DesktopRoomie-A00-M03.2-staged-implementation-roadmap.md). This is a context checkpoint, not permission to implement a next mission.

## Accepted foundation

| Mission | Accepted result | Exact evidence |
| --- | --- | --- |
| A00-M01 | Managed transparent procedural body, input shape, focus isolation and event-driven wait/cleanup. | [M01 acceptance](../experiments/desktop-probe/ACCEPTANCE.md), recorded 2026-10-06. Its executable identity is recorded by commit subject, not a full SHA. |
| A00-M02 | Bounded offset-preserving drag, safe pointer completion/cancellation, mouse-only popup and lifecycle recovery. | [Integrated M02 acceptance](../experiments/desktop-probe/ACCEPTANCE-M02.md), recorded 2026-10-07; tested code `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b`. |
| A00-M03.1 | Above / Normal / Below, natural fullscreen precedence/return, preserved focus/input behavior and sustained mixed use. | [M03.1 acceptance](../experiments/desktop-probe/ACCEPTANCE-M03.1.md), recorded 2026-10-08; independent source review, matching CI and owner L01–L13 PASS. |

**Current reviewed executable code revision:** `f8332ec99e876db1d809cd718dc76dff9fd38e27`. Matching CI run and 139-test result are historical evidence in the M03.1 record. The earlier `ca80f6c…` executable was superseded by corrections. Subsequent documentation commits, including D00-M01, are distinct revision identities and do not represent new tested executables. Compare source/Cargo/CI paths against that code SHA when reconciling a later HEAD; see [WORKFLOW](WORKFLOW.md).

## Supported host and evidence boundary

First supported host: **Archcraft / Openbox 3.6.1 / X11 / Picom v13**. M03.1 diagnostics recorded `eDP-2`, 1920×1080 at +0+0, desktop 0, usable workarea (10,48,1900,1022), RENDER 0.11 and SHAPE 1.1. WM/compositor version strings are established host facts, not fresh measurements from that invocation. [ENVIRONMENT](../experiments/desktop-probe/ENVIRONMENT.md) preserves older M01 dual-monitor geometry and a dated M02 reconciliation; those snapshots are not interchangeable.

Host focus/click-through/stacking/fullscreen observations come from the owner. Automated and injected tests establish different evidence; they do not prove physical monitor removal or real X server failure recovery. Other WMs, Wayland and other platforms remain unverified. The probe is the technical foundation, not the complete companion or an accepted completion of all A00.

## Active work and next gate

- **D00-M01 ACCEPTED:** documentation/context hygiene completed. The independent review of docs-only implementation `e3266aeee5a4757f465b90376bcf97838e225888` passed scope, document authority, source ownership, invariants, link/reference checks, cold-start recovery, and matching GitHub CI [run 37807374691](https://github.com/zNe4/DesktopRoomie/actions/runs/37807374691). The owner approved acceptance on 2026-10-08. The final documentation-status commit is separate from the reviewed executable revision.
- **A00-M03.2 DESIGN + A–H STAGED ROADMAP APPROVED (2026-10-09):** the [revised Codex design](DesktopRoomie-A00-M03.2-approved-implementation-plan.md) passed independent architecture review; the owner then approved the [eight bounded implementation stages](DesktopRoomie-A00-M03.2-staged-implementation-roadmap.md) with separately reviewed/tested/accepted checkpoints. M03.2 specifies managed Hide/Show, same-instance X11 control, and **Bring Top** on the existing workspace. **No M03.2 executable work has begun and M03.2-A is not yet authorized.** The next gate is a separate explicit, SHA-pinned stage-A Codex assignment. **Bring Here** and composed **Recover** remain reserved for M03.3. Optional Openbox launch/keybindings remain user-managed and unimplemented.
- **Deferred:** M03.3 same-instance Bring Here / workspace placement plus composed Recover; M03.4 integrated A00 acceptance/host decision; A01 relative-window stacking and companion presentation/behavior. The [completed host study](research/studies/R00-M01-nekoai-linux-host-a00-g3.md) informs A00-G3; additional [R00 studies](research/ProjectVanilla-R00-external-projects-research-roadmap.md) are just-in-time design dependencies, not automatic work or blockers for unrelated missions.

## Meaningful limitations

A covered Below body can make its only menu inaccessible; independent recovery and hide/show are absent. Finite duration provides an exit fallback. Workspace changes suspend/cancel interaction rather than moving the body to follow the user. Monitor identity is retained; removal is an error, not automatic migration.

Layer confirmation establishes readable EWMH flags, not visual stacking or who caused a change. A partial transition can leave Normal; there is no rollback or reassertion loop. Finite operation deadlines assume a responsive X server and do not impose hard transport timeouts on synchronous replies. Brain/Spine, runtime memory/services, animation assets, persistence and cross-device abstractions are not implemented. See [ARCHITECTURE](ARCHITECTURE.md) and [INDEX](INDEX.md) for source boundaries and deeper evidence.
