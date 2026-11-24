# DesktopRoomie — A01-C1 architecture and implementation plan

**Mission:** A01-C1-P, after R00-M13-P4-D closeout  
**Prepared:** 2026-11-22  
**Document status:** Architecture handoff / implementation plan; **not an implementation, native-test, asset-use, or publication authorization**  
**Proposed repository location:** `docs/DesktopRoomie-A01-C1-architecture-plan.md`  
**Public baseline:** `791a0636e7336d1df91efc5cb069b3790277213c` (`main` P4-D documentation closeout)

> **Owner direction:** Favor cohesive, independently testable modules with explicit ownership and minimal dependencies. Use conventional Rust crate organization (`src/lib.rs` for reusable APIs, a thin `src/main.rs` for the executable), but migrate incrementally. Neither maximizing nor minimizing the number of files/crates is itself a goal. Never broaden an approved change into a speculative refactor.

## 1. Accepted evidence and boundaries

**A00 host (preserve):** Frozen accepted F-T3 executable `f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863`, 760 tests and owner-native X11/Openbox/Picom acceptance. The accepted `experiments/desktop-probe` tree is reported identical at the public P4-D documentation HEAD. It provides one managed, transparent, nonactivating UTILITY window, one cooperative per-screen owner and independent CLI, checked X11 resources, exact SHAPE input, pointer capture/release, drag, menu, focus isolation, Above/Normal/Below, Hide/Show/Bring Top/Bring Here/Recover, finite deadlines and truthful placement evidence.

**Private renderer (preserve):** Accepted P4-D1-B source derivative, manifest SHA-256 `bd63117ffde028648c14f4f7e2fc32b3f677b40f8e3fb4d57939cbd785c893c4`, patch SHA-256 `524b5a7bc5e9e303ca54a81f047c802697ecc4a5f3f49f5122bfeafa3c719548`. P4-D acceptance covers persistent Vulkan resources, clip/direction control, hard cuts, clip-change phase zero, walking-direction phase preservation, front-right idle and specified walking sequence, and owner-reported native lifecycle. Retained partial telemetry: 59.79 application FPS, 16.637 ms median, 17.770 ms p95, stable resource counts, zero diagnostic readbacks. It does **not** prove actual scanout, native device loss, transparent ARGB X11 presentation, or integrated desktop safety. Back-facing idle loop seam remains qualified; look/sit/rest/sleep/held/falling/touch capabilities are not assumed accepted.

**Target only:** Arch Linux/Archcraft, X11, Openbox 3.6.1, Picom v13, Kitty; retain selected-monitor identity and existing qualified scaling limitations. No Wayland, KDE, hotplug, mixed-DPI or universal-WM claims.

**Repository and content separation:** `DesktopRoomie` public and independently buildable/runnable without Spine; `DesktopRoomie-Spine` separate initially private repository/derivative depending **on** DesktopRoomie, never the reverse. Preserve ignored `extensions/spine` relative symlink arrangement; no submodule, new mandatory optional path dependency, plugin ABI, IPC or separate renderer daemon in C1. No NEKOPARA/Vanilla assets in public repository; synthetic public fixtures. Runtime/license/attribution and private asset permissions remain unresolved and require separate authorization.

References: `AGENTS.md`, `docs/WORKFLOW.md`, `docs/STATUS.md`, `docs/ARCHITECTURE.md`, `docs/INVARIANTS.md`, `docs/DesktopRoomie-A00-A01-plan-v0.1.md`, `docs/DesktopRoomie-A00-M03.4-host-architecture-decision.md`, `docs/renderer/R00-M13-P4-D/P4-D-ACCEPTANCE.md`, `docs/research/R00-M12-spine-backend-separation-decision.md`.

## 2. C1 deliverable and explicit non-goals

A01-C1 combines **G1.1** (one launchable/recoverable companion and minimal menu), **G1.2** (selected-monitor bounds, three provisional size presets, checked floor), **G2.1** (small renderer-neutral presentation contract), and **initial G2.2** (the accepted animated Vanilla shown through the accepted A00 host with alpha, stable anchor, valid input coverage and focus safety).

**C1 success:** One actual managed X11 body, animated Vanilla with verified safe input and correct geometry, original independent CLI and controls still working, plus standalone procedural fixture without Spine. Aim for 60 FPS under tested conditions; report frame pacing and host responsiveness separately.

**Not C1:** Autonomous scheduler, gravity/falling/landing or character drag enhancements beyond preserved A00 behavior; new general behavior engine; additional unverified Vanilla clips; Hina/glTF; character switching; cognition/LLM; multi-body; persistent user preferences; global keyboard hooks; a new window system or a second event loop. Keep later A01 C2–C8 sequencing intact.

## 3. Architecture principle and target ownership

The **one real A00 host** remains authoritative for the window, server connection/event consumer, lifecycle, workspace/layer control, geometry/visibility proof, pointer and popup ownership, input fences, deadlines, and cleanup. Presentation/backend code must never independently move, restack, focus, map, resize or destroy the body, consume its native events, open a second companion owner, or alter v2 command semantics.

| Area | Ownership | Boundary |
|---|---|---|
| Application/host coordinator | Owner admission/Starting/Ready/Closing, dispatch, event loop, fatal finalizer, scheduling | Drives presentation and X11 adapters through explicit APIs |
| X11 native modules | Connection, managed window, ARGB visual, pixmap or surface target, SHAPE, WM, resource cleanup | No knowledge of Spine clips |
| Pure geometry/control/interaction | Root/local transforms, drag offsets, bounds, gestures, layer and v2 operation proof | Remain backend-neutral and deterministic |
| Public presentation contract | Semantic actions, orientation, clock sample, committed extent, transform/anchor, frame ID, hit data, lifecycle | No native event loop or character-specific code |
| Private Spine adapter | Spine pose, action/facing capability mapping, persistent wgpu state, geometry, selected output transport | Depends on public API; no X11 ownership |
| Future 3D backend | Independent renderer satisfying the same narrow contract after actual 3D feasibility | No proactive implementation during C1 |

**Conventional Rust target layout, evolved only as justified:**

```text
experiments/desktop-probe/
  Cargo.toml
  src/
    main.rs                # thin public executable: CLI args -> library dispatch
    lib.rs                 # minimal reusable entry and explicit public API
    host/                  # only extract cohesive owner/scheduling code when necessary
    presentation/          # G2.1 public contracts and synthetic fixture (C1-B)
    geometry.rs            # existing pure checked geometry
    interaction.rs         # existing pure pointer/gesture policy
    layer.rs               # existing pure layer controller
    control/               # existing v2 progress/codec, one authority
    x11/                   # existing native resource and protocol modules
    probe_tests/           # preserve meaningful test identities and source coverage
```

This is a **target organization**, not permission to create all listed directories or move everything now. Keep `main.rs` as a binary and `lib.rs` as the library root. **Do not adopt R1's `[lib] path = "src/main.rs"` as the permanent architecture.** Exact extraction of the large root and private-module references must be small and independently reviewed. Do not duplicate the production loop across library and binary or add `include!` shims as a way to avoid ownership review.

## 4. C1-A — first bounded implementation mission (after explicit owner authorization)

**Goal:** Establish a conventional library/binary boundary while preserving the *entire existing procedural behavior*; do not integrate GPU/Spine yet.

1. Verify branch, HEAD, local/remote baseline and clean working tree; abort on mismatch or unrelated owner changes. Read the accepted source and workflow.
2. Propose the smallest source move/extraction that leaves a thin `src/main.rs` binary and a reusable `src/lib.rs` dispatcher, preserving all existing module ownership. Use Cargo conventional library+binary targets; retain executable name and CLI invocation. Do not create extra crates without justification.
3. Preserve Help/Diagnose/Owner/Control observable outputs, error texts and exit codes, owner admission/lifecycle and independent same-session CLI. The existing `main()` uses `process::exit` on several branches: replacing it with a reusable return status requires explicit equivalence checks, including paths where destructor behavior might change. Do not silently change terminal cleanup semantics.
4. Preserve all 760 test identities, fixtures, resource/error sequencing and especially the **two source-sensitive tests** in `src/probe_tests/here_service_tests.rs` currently reading `include_str!("../main.rs")`. Redirect them to the **actual production source** containing `run_probe` and the owner operation service, keeping their substantive checks; do not point them at a thin wrapper or replace them with vacuous assertions. If the extraction would require broad test restructuring, stop and report a revised smaller approach.
5. Keep public API minimal (e.g., a CLI dispatcher returning a status/exit code) and leave the later `BodyFactory`/`BodyPresenter` interface **unimplemented** until C1-B contract review. Do not add runtime, wgpu, private asset, XCB feature or window-size dependencies in C1-A.
6. Verify locked build, formatting, all-target Clippy and full existing test suite; also check actual public binary behavior for Help and error exit codes without launching a native owner if not authorized. Report exact diff and test results for independent ChatGPT review.

**C1-A gate:** public binary still runs in its accepted standalone mode, no v2 command or native behavior changes, test/source assertions intact. No automatic advancement to C1-B. Any code publication requires separate branch/base and commit/push authority under `docs/WORKFLOW.md`; no direct push to `main`.

## 5. C1-B — neutral presentation contract and synthetic fixture (later mission)

After C1-A review and owner approval, specify/implement the narrow public factory/presenter APIs rather than a generic plugin system.

**Factory:** lazily invoked only by the admitted Owner, after selection ownership (and optional startup delay); supplies an initial validated body extent/capabilities before creation, then attaches to a **lifetime-bound** host presentation target after body creation. Help/Diagnose/Control must bypass the factory entirely. The public procedural fixture uses the same host entry; private executable supplies its own adapter after separate approval.

**Presenter:** accepts host-owned action/facing, active-time sample, extent/generation; stages a candidate frame; reports capabilities, pixels or presentation submission identity, bounds and safe candidate hit coverage; can redraw without time advance, suspend/resume and perform checked shutdown. Explicitly model evaluated → admitted/staged → submitted → transport-completed → host-committed; **none means confirmed physical display scanout**. Keep output-specific transport objects outside shared action/geometry API. Do not expose a native event receiver or mutable window authority.

**Identity:** frame ID + body/viewport/placement generation; stale completions cannot publish or reopen input. One checked transform for rendered pixels, anchor, bounds and hit coverage, including reflection, scale, letterbox, Y inversion and rounding. Explicit encoded premultiplied alpha and (on byte path) pixel format, stride and row-padding rules. Preserve P4-D hard cuts, time-zero clip changes, walking-direction phase and capped visible delta; nonaccepted actions remain capability-gated fallbacks/pending.

**Scheduling:** preserve A00's single control-first event/deadline loop and bounded 64-event batches, wait/drain ordering and fresh error handling. Add only an optional frame deadline, at most one bounded frame step per turn, explicit missed-slot skipping and inactive-time suspension. No blocking frame pipeline may starve owner requests, pointer release or deadlines; no independent permanently busy loop.

## 6. Pixel transport — explicitly unresolved until synthetic proof

**Candidate A: Direct Vulkan WSI onto the *existing* managed depth-32 X11 window.** Potential benefit: no GPU-to-CPU readback. Requires proof of live XCB handle and screen-index lifetimes, supported visual/queue/alpha and Picom behavior, compositing of premultiplied pixels, window map/unmap/resize lifecycle, and especially host-service stalls around acquire/present. The accepted P4-D preview uses an opaque surface/matte, not the A00 ARGB body. Current public `RustConnection` has no direct native XCB pointer. A single owner-side `x11rb::xcb_ffi::XCBConnection` is a *candidate* requiring a focused concrete-connection audit, optional unsafe/linkage decision and verification of buffered events/sequence handling, polling and checked control operations. It cannot introduce a second event receiver or owner.

**Candidate B: Persistent offscreen Vulkan, asynchronous bounded staging, checked X11 pixmap upload/blit.** Potential benefit: retain accepted `RustConnection`, ARGB pixmap/GC and existing host path. Requires proof of actual server pixel layout/endian/stride/request limits, PMA swizzle, bounded reused staging buffers, frame-size-specific memory costs, stale in-flight work and resource stability. The accepted blocking diagnostic `snapshot()` is **not** a live 60 FPS transport. Expose repaint from the last safely committed pixmap; never claim a checked `CopyArea` means Picom displayed that frame.

**Decision rule:** do not preselect A or B. Prepare a separately authorized **synthetic-only**, same-XID, same-host native proof measuring both under identical scenarios. Failure of either candidate must not undermine the public procedural runtime. Reject any path that loses pointer safety, focus, checked ownership, control responsiveness or correct alpha/SHAPE behavior, even if its FPS is higher. Do not use character assets to decide transport.

## 7. Animated input/presentation consistency — safety invariant

The server SHAPE input region is **not automatically synchronized** to Vulkan presents or Picom composition. A frame's return from `present()`, an X11 checked request, and elapsed time cannot alone identify precisely what Picom may still show.

For every *new, unowned* pointer press at time `t`, require:

```text
EffectiveInputShape(t) ⊆ intersection of eligible interactive-visible regions
                        of every frame/placement that could still be displayed at t.
```

Coordinates must refer to the **actual native input window**, including managed-window/reparenting effects. An unknown or blank display state contributes an empty region. Exclude transparent texels and noninteractive decorations, not just outside-of-triangle geometry. Native SHAPE must never be wider than the safe coverage; rejecting a click later in Rust does **not** undo swallowing it at the X server.

Before publishing a frame that invalidates coverage, checked-narrow/empty SHAPE first. Expand only when all potentially displayed states justify it. Distinguish visualization progress from host input readiness. An indefinitely empty shape is safe but **not acceptable for an interactive C1 milestone**. Require useful interaction coverage and explicit evidence for any old-frame retirement criterion. If the synchronization model cannot safely provide it, stop the transport proof for architecture review.

**Gestures:** An admitted mouse gesture retains its original pointer owner, grab offset, event freshness and completion/release obligation. Ordinary playback does **not** pause just because a button is held. Changes affect *new* gesture admission; release routing must remain safe after coverage narrows to empty. Visibility/resize/lifecycle fencing still cancels and settles ownership. Native proof must include automatic right-opening capture, explicit popup/left captures, stale input and no invisible swallowed clicks.

## 8. C1-C — initial private Vanilla integration (later mission)

Keep the standalone P4-D preview intact as a regression entry. Build a private executable that depends on an approved public library revision and supplies a thin Spine presentation adapter. Use only separately authorized private source/assets and license-compatible runtime use. The adapter maps the accepted P4-D `AnimationController` and persistent `GpuRenderSession` to host intents/frame metadata; do **not** copy the preview's own event loop/window owner into the host.

Initial accepted capability set: front-right idle and the scripted front-right/front-left/back-right walks, with hard cuts and preserved walk phase. Back-facing idle seam remains qualified. Back-left, look/sit/rest/sleep/touch and future interaction performances must be listed as unsupported/unverified until separately reviewed. A real foot/ground anchor is **not** established by the accepted geometry; use a reproducible owner-approved pack-specific anchor, validated against the same transform as pixels and hit coverage. Use a conservative fixed full-animation envelope plus per-frame containment check; never refit each pose and create visible bouncing.

Native proof and source review gate C1-C: same XID/one owner, correct alpha and PMA colors, stable anchor, safe animated input, proper map/Expose/recovery, no focus theft or desktop switching, no unbounded GPU resource growth, responsive host, and truthful frame-pacing measurements.

## 9. C1-D — G1.2 size presets and floor (later mission)

After safe fixed-extent integration, introduce **three provisional, review-approved client-size presets** and a stable anchor. Choose real pixel sizes from art/display evidence rather than inventing them. Whole-client fit against selected-monitor × verified workarea remains conservative. A configured floor `F`, local ground-anchor height `a_y`, client height `H` and root usable area `U` must obey:

```text
U.top + a_y <= F <= U.bottom - H + a_y
```

Reject impossible floors/presets **before** mutation; define root integer/pixel rounding explicitly. A floor here is validated geometry, **not gravity** (G3.2).

**Resize transaction:** allow only local/viewable/settled state and exclude active gesture, capture/release debt, popup, pending movement/correction/map/SHAPE restoration, layer or control operation. Before mutation: mark resize busy, fence input, invalidate old frame generation. Update fixed-size WM hints, issue a single bounded WM-cooperative resize/move, then freshly verify client extent, root origin, retained monitor/workarea/current workspace and whole-client fit. Prepare/commit new presentation and safe input only after convergence. A refusal/timeout/partial change must leave input fenced unless a safe state is freshly verified; no unbounded reassertion, guessed success or automatic retry.

**Here/Recover:** Do not change v2 wire protocol. All strict destination/readiness proofs use one committed client extent; resizing is excluded for the entire active operation and one-way handoff. Preserve frozen caller desktop, same-body identity, effective layer, genuine minimization, one correction/150 ms allowance and original operation deadlines. A changed or unreadable extent invalidates proof but grants no new authority.

## 10. Synthetic native transport proof — future, independently authorized

One transport candidate per process launch, same managed body XID, recorded X11/Openbox/Picom target, **synthetic content only**. Start with 160×160 and one separately approved larger proof extent. For each path, run three trials of 30 s steady-state plus 30 s simultaneous control/pointer traffic. Record exact GPU, driver, refresh rate, screen/visual, Picom configuration where available, source/build hashes and all actual test conditions.

**Images/inputs:** transparent holes/padding, opaque/translucent known PMA swatches over dark/light windows, noninteractive visible effects, moving edges, disjoint hit lobes, frame markers, changing SHAPE regions, a truly usable safe hit area, Kitty typing and actual underlying click receipts. Include manual assessment and deterministic synthetic failure injection, recorded separately. Test Expose, remap, minimize/restore, workspace leave/return, popup/drag/release, resize/refusal, Hide/Show/Here/Recover under load, Quit, duration, duplicate owner and relaunch. Check resource release and no stale-generation frame reactivation.

**Proposed quantitative gate for a verified 60 Hz display (review/approve before experiment):** >=59 useful host frame commits/second per active trial, p95 frame interval <=20 ms, p99 <=33.4 ms, p99 input/control dispatch latency <=33.4 ms, worst observed host-service gap <=100 ms under defined responsive load. Also report p50/p95/p99/worst transport stalls, CPU/GPU timing, dropped/stale frames, staging depth and memory. These are *proposed acceptance thresholds*, not measured facts; host commits do not prove display scanout. Explicit external trial deadline mitigates any blocking Vulkan call.

**Immediate abort:** focus theft, swallowed click in an invisible region, stuck pointer release, stale frame/input admission, unexpected owner/XID change, unbounded resource growth, unsafe remap, or unacceptable host-control stall. A sound old-frame visibility/input model is required even if all finite manual observations pass.

## 11. Review gates and staged work

| Gate | Next deliverable | Must be independently approved before subsequent work |
|---|---|---|
| **C1-P** (this document) | Architecture and modularity decision, mission boundaries, risks | Owner/ChatGPT acknowledge the agreed scope; open questions stay open |
| **C1-A** | Conventional public binary/library boundary only | Full standalone A00 checks, exact diff review, no behavior regression |
| **C1-B** | Small public presentation contract + synthetic procedural compatibility | Deterministic frame/input/clock/ownership tests and independent review |
| **Transport proof** | Separately authorized synthetic native comparison | Evidence-based A/B selection, alpha and input safety, host latency |
| **C1-C** | Accepted P4-D private fixed-size Vanilla adapter on A00 host | Private/public build checks, source review, owner-native alpha/input/pacing acceptance |
| **C1-D** | Three sizes, floor, safe resize transaction | Pure+native geometry/SHAPE/Here/Recover regressions, owner acceptance |
| **C1 closeout** | Evidence and progress-site update in authorized scope | Independent docs review, owner acceptance and separate publication authority |

Keep these A/B/C/D labels consistent with the **latest Codex R1**; these are A01-C1 slices, not historical R00-M13-P4 checkpoints. New Codex sessions should handle one bounded approved slice at a time; do not automatically advance. The local progress site should be updated only after reviewed acceptance in an approved documentation scope, never silently during implementation.

## 12. Non-regression test checklist

- **Public:** all 760 accepted tests and test identities; build, fmt, all-target Clippy, locked dependencies; Help/Diagnose/Owner/Control CLI, exact status/exit semantics, source-sensitive fatal and wait-order checks.
- **Owner/input:** one owner and control operation, Busy/no queue, Starting/Ready/Closing, focus/typing, procedural transparent patch and click-through, left/right/menu capture debt, stale chord/release suppression, fresh SHAPE evidence, gesture cancellation and checked resource cleanup.
- **WM/placement:** managed UTILITY no-activation window, Above/Normal/Below, natural fullscreen yielding, Hide/Show/Bring Top/Bring Here/Recover, remote frozen workspace, retention of monitor identity, genuine minimized versus ordinary off-workspace Iconic, deadline and correction allowance, correct Partial/OutcomeUnknown classification, no implicit desktop switch.
- **Presentation:** accepted P4-D extension/preview/S2-C suites, accepted script identity and hard-cut policy, frame IDs and generation rejection, buffer/alpha correctness, scale/anchor/region parity, no clipped extremities, resource stability; distinguish native measurement and simulated errors.
- **Manual acceptance:** owner on actual Arch Linux/X11/Openbox/Picom desktop with Kitty; real typing and clicking, scrolling beneath transparent padding, dragging and popup, layer/fullscreen/workspace transitions, launch/quit/duplicate/relaunch and performance under input traffic.

Run the following for **public source changes**, plus task-specific checks:

```sh
cargo build --locked --manifest-path experiments/desktop-probe/Cargo.toml
cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check
cargo clippy --locked --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path experiments/desktop-probe/Cargo.toml
```

The private extension/preview/S2-C checks and native asset tests remain separate, exact-source and separately authorized. Report checks **actually run**, not inferred from prior CI.

## 13. Current decisions / unresolved approvals

**Agreed direction:** modular architecture, conventional `src/lib.rs` + thin `src/main.rs`, one accepted A00 owner, backend-neutral presentation, public/private dependency separation, phased acceptance, no speculative refactor. This document records the owner's modularity preference and ChatGPT recommendation; it is not itself a Git commit or owner-native acceptance.

**Still requiring design/proof approval:** exact API and connection-lifetime types, whether direct XCB is viable, chosen GPU/X11 transport, useful nonempty animated SHAPE policy and old-frame retirement evidence, quantitative proof thresholds, specific preset pixel extents, anchor source and floor/clamp UX, private runtime licensing/use and character asset inspection, branch/publish permission. Unsafe or uncertain cases fail closed and return for review.

**Current action:** place this document in the local public repository **without staging, committing or pushing** unless separately authorized. Open a **fresh Codex session** for C1-A only when the owner explicitly authorizes the bounded implementation assignment below. Codex must first report any architectural blocker; passing the plan's gate does not authorize C1-B.

## Appendix — paste-ready prompt for the next Codex implementation session

> **DesktopRoomie A01-C1-A — bounded implementation assignment.** Read `AGENTS.md`, `docs/WORKFLOW.md`, `docs/STATUS.md`, `docs/ARCHITECTURE.md`, `docs/INVARIANTS.md`, and `docs/DesktopRoomie-A01-C1-architecture-plan.md`, especially §§3–4 and §§11–12. Target public baseline `791a0636e7336d1df91efc5cb069b3790277213c` unless the owner supplies a reviewed updated SHA; first inspect current `git status`, HEAD and `origin/main`. Implement **only C1-A**: a conventional `src/lib.rs` public dispatcher and thin `src/main.rs` binary, preserving existing procedural behavior, CLI, owner event loop, exit statuses and all 760 tests, including source-sensitive fatal/wait-order assertions. Prefer the smallest maintainable extraction and stop if a broader structural move or altered shutdown semantics become necessary. Do **not** implement presentation, Spine, wgpu, XCB, window resizing, transport proofs or private assets. No branch creation, commit, push, merge, site edit, permission/asset change or native GPU test is authorized by this text alone. Run applicable locked build/fmt/Clippy/tests, provide complete source diff and verification results, and stop for independent ChatGPT review and owner approval. **Do not advance to C1-B.**

