# Current checkpoint

Updated 2026-10-10 after independent review, matching CI and owner acceptance of A00-M03.2 stages A–D. The [approved technical design](DesktopRoomie-A00-M03.2-approved-implementation-plan.md) and [A–H staged roadmap](DesktopRoomie-A00-M03.2-staged-implementation-roadmap.md) remain the implementation baseline. This is a context checkpoint, not permission to implement a next mission.

## Accepted foundation

| Mission | Accepted result | Exact evidence |
| --- | --- | --- |
| A00-M01 | Managed transparent procedural body, input shape, focus isolation and event-driven wait/cleanup. | [M01 acceptance](../experiments/desktop-probe/ACCEPTANCE.md), recorded 2026-10-06. Its executable identity is recorded by commit subject, not a full SHA. |
| A00-M02 | Bounded offset-preserving drag, safe pointer completion/cancellation, mouse-only popup and lifecycle recovery. | [Integrated M02 acceptance](../experiments/desktop-probe/ACCEPTANCE-M02.md), recorded 2026-10-07; tested code `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b`. |
| A00-M03.1 | Above / Normal / Below, natural fullscreen precedence/return, preserved focus/input behavior and sustained mixed use. | [M03.1 acceptance](../experiments/desktop-probe/ACCEPTANCE-M03.1.md), recorded 2026-10-08; independent source review, matching CI and owner L01–L13 PASS. |

**Accepted M03.1 executable code revision:** `f8332ec99e876db1d809cd718dc76dff9fd38e27`. Matching CI run and 139-test result are historical evidence in the M03.1 record. The earlier `ca80f6c…` executable was superseded by corrections. Subsequent documentation commits, including D00-M01, are distinct revision identities and do not represent new tested executables. See [WORKFLOW](WORKFLOW.md) for revision/evidence discipline; the latest accepted executable is identified below.

## A00-M03.2 — accepted stages A–D

**Latest accepted executable revision:** `f4ed63c1dfddbbcdf324141c93e3c26f0c39fa47`. This documentation checkpoint and later documentation-only commits are separate revision identities, not newly tested executables. Acceptance below is reported by the owner assignment, distinguishing independent ChatGPT source/CI review from owner real-host observations. Test counts belong to the exact historical CI revisions; no fresh Rust or desktop run is claimed here.

| Stage | Accepted executable SHA | Matching successful CI | Accepted scope and evidence |
| --- | --- | --- | --- |
| A — pure foundations | `07fc142b68dc77cd77b8f20651a2f21bba244d75` | [38012276595](https://github.com/zNe4/DesktopRoomie/actions/runs/38012276595), 163 tests | Command/exit types, wire codecs, owner descriptor, caller marker, request/terminal response, strict pure visibility classification, confirmation and correlation rules. Independent review/CI and owner acceptance; pure-stage evidence does not imply host visibility operations. |
| B — ownership/lifecycle | `f3f10d6f172990067473bf478d2c12d42b7979e4` | [38014813584](https://github.com/zNe4/DesktopRoomie/actions/runs/38014813584), 196 tests | Never-mapped InputOnly control endpoint, per-screen cooperative selection, genuine server timestamp, guarded arbitration, Starting/Ready/Closing, duplicate prevention, ordered cleanup and runtime control-loss failure. Independent review/CI and owner real-Openbox acceptance, including duplicate launches, delayed appearance, no typing-focus theft and clean relaunch after normal exit or forced termination. |
| C — independent transport | `f0465fe8df5f412ae8925330898d674c27c20356` | [38030443563](https://github.com/zNe4/DesktopRoomie/actions/runs/38030443563), 220 tests | Second-process discovery, validated InputOnly caller reply endpoint with immutable marker, direct private ClientMessage request/reply, correlation, bounded caller deadline, lifecycle/Busy/peer disappearance, truthful terminal responses and exit mapping. Independent review/CI and owner real-Openbox acceptance passed. |
| D — observation/input fence | `f4ed63c1dfddbbcdf324141c93e3c26f0c39fa47` | [38070108185](https://github.com/zNe4/DesktopRoomie/actions/runs/38070108185), 247 tests | Strict native ICCCM/EWMH observation, complete property decoding/capability validation, coherent workspace/minimization classification, inactive checked iconify/restore helpers, checked empty/restored input SHAPE, transition pointer-release debt, gesture/menu/movement invalidation, input sequence freshness and safe restoration predicates. Accepted after the correction described below. |

**Stage D acceptance history:** initial candidate `889c8ac9df5850fabbe5ef5148c0bbcb3cc95914` passed its first source/CI review, but owner Openbox acceptance exposed a transparent-padding click-through failure. The corrected executable above adds bounded post-map reassertion of the original interactive silhouette. Its independent source review and matching correction CI passed; the owner confirmed genuine transparent-padding click-through and reported all other Stage D real-host regression checks successful. The application-side correction resolves the observed host behavior; the exact Openbox-internal root cause was not conclusively established. The original failed host gate is not superseded into a historical pass.

**Functional boundary:** Hide, Show and Bring Top are parsed and transported to the same owner, but a Ready owner still returns `Failed / Unsupported / Preflight`, exit code **7**. No remote visibility or layer mutation is enabled. Stage D deliberately did not activate those operations, and the optional startup refinement of disabling native body input universally until readiness was deferred.

**Next gate:** Stage E (Show first) requires a separate bounded assignment and **has not started**. Stages E/F/G/H are unimplemented; A–D acceptance is not acceptance of the entire M03.2 mission or the integrated H01–H28 host matrix. Final integrated evidence/`ACCEPTANCE-M03.2.md` remains reserved for H. M03.3 Bring Here / composed Recover remains excluded. This checkpoint authorizes no remaining stage.

## Supported host and evidence boundary

First supported host: **Archcraft / Openbox 3.6.1 / X11 / Picom v13**. M03.1 diagnostics recorded `eDP-2`, 1920×1080 at +0+0, desktop 0, usable workarea (10,48,1900,1022), RENDER 0.11 and SHAPE 1.1. WM/compositor version strings are established host facts, not fresh measurements from that invocation. [ENVIRONMENT](../experiments/desktop-probe/ENVIRONMENT.md) preserves older M01 dual-monitor geometry and a dated M02 reconciliation; those snapshots are not interchangeable.

Host focus/click-through/stacking/fullscreen observations come from the owner. Automated and injected tests establish different evidence; they do not prove physical monitor removal or real X server failure recovery. Other WMs, Wayland and other platforms remain unverified. The probe is the technical foundation, not the complete companion or an accepted completion of all A00.

## Active work and next gate

- **D00-M01 ACCEPTED:** documentation/context hygiene completed. The independent review of docs-only implementation `e3266aeee5a4757f465b90376bcf97838e225888` passed scope, document authority, source ownership, invariants, link/reference checks, cold-start recovery, and matching GitHub CI [run 37807374691](https://github.com/zNe4/DesktopRoomie/actions/runs/37807374691). The owner approved acceptance on 2026-10-08. The final documentation-status commit is separate from the reviewed executable revision.
- **A00-M03.2 A–D ACCEPTED:** exact executable, CI, independent review and owner evidence are recorded above. The approved design and A–H allocation remain in force. **E is the next separate implementation gate; it has not started.** Optional Openbox shortcuts remain user-managed; recovery operations are not yet enabled.
- **Deferred:** M03.3 same-instance Bring Here / workspace placement plus composed Recover; M03.4 integrated A00 acceptance/host decision; A01 relative-window stacking and companion presentation/behavior. The [completed host study](research/studies/R00-M01-nekoai-linux-host-a00-g3.md) informs A00-G3; additional [R00 studies](research/ProjectVanilla-R00-external-projects-research-roadmap.md) are just-in-time design dependencies, not automatic work or blockers for unrelated missions.

## Meaningful limitations

A covered Below body can make its only menu inaccessible; independent control transport exists, but actual recovery and hide/show operations are absent. Finite duration provides an exit fallback. Workspace changes suspend/cancel interaction rather than moving the body to follow the user. Monitor identity is retained; removal is an error, not automatic migration.

Layer confirmation establishes readable EWMH flags, not visual stacking or who caused a change. A partial transition can leave Normal; there is no rollback or reassertion loop. Finite operation deadlines assume a responsive X server and do not impose hard transport timeouts on synchronous replies. Brain/Spine, runtime memory/services, animation assets, persistence and cross-device abstractions are not implemented. See [ARCHITECTURE](ARCHITECTURE.md) and [INDEX](INDEX.md) for source boundaries and deeper evidence.
