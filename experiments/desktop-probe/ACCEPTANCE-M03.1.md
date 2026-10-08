# A00-M03.1 verification and pending owner acceptance

Decision: **implementation ready for independent review; owner acceptance NOT RUN**.

Evidence recorded 2026-10-08. Scope is M03.1 only. This record does not mark M03.1 or A00 accepted and does not authorize a later mission.

## Revision identity and environment

- Starting branch: `main`; starting HEAD: `2968b7520041fbc4d5c605668e8fd6473643a932`.
- Starting working tree: clean, unchanged from the read-only planning pass. No intervening owner edits were present.
- Ending branch/HEAD: unchanged. Tested implementation: **uncommitted working-tree changes on that HEAD**, including new `src/layer.rs` and `src/x11/state.rs`. HEAD alone does not identify the tested executable.
- Final changes: five modified source files (`src/geometry.rs`, `src/interaction.rs`, `src/main.rs`, `src/x11/menu.rs`, `src/x11/mod.rs`), two new source files, this report, and `M03.1-IMPLEMENTATION.md`.
- No commit, push, branch creation, staging, or discard was performed. No dependency/lockfile/CI workflow change was made.
- Local toolchain: `rustc 1.99.0 (b940084d7 2026-09-28)`; `cargo 1.99.0 (5f94df478 2026-08-27)`.
- Current target-host environment snapshot: **NOT RUN**. The established target is Openbox 3.6.1 / X11 / Picom v13 on Archcraft; historical M02 values are not a fresh observation of this implementation.
- GitHub CI for the new implementation: **NOT RUN / not published**. Record an exact executable commit and corresponding CI run after owner-authorized publication. Do not substitute M02 CI evidence.
- Independent review of the implemented diff: **PENDING**. The owner supplied approval of the plan with the three corrections implemented here.

## Local automated verification

Commands executed from the repository root under the desktoproomie-verify procedure:

| Exact command | Actual result |
| --- | --- |
| `cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check` | PASS — exit 0 |
| `cargo build --manifest-path experiments/desktop-probe/Cargo.toml` | PASS — exit 0 |
| `cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings` | PASS — exit 0, no warnings |
| `cargo test --manifest-path experiments/desktop-probe/Cargo.toml` | PASS — exit 0; 137 passed, 0 failed, 0 ignored, 0 measured, 0 filtered out |
| `git diff --check` | PASS — exit 0, no whitespace diagnostics |

`cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml` was applied during implementation. Earlier development test runs passed at 121 and 137 tests; the final verification above is the 137-test suite after the final Rust edits. Cargo commands sharing the target directory briefly waited on Cargo's normal build lock; all completed successfully.

The complete tracked diff and new source/document files were reviewed for scope and M02 regressions. New-file whitespace was checked separately against `/dev/null` because `git diff --check` does not include untracked files. No graphical process was launched. No real-host layer/fullscreen/focus result is inferred from these automated checks.

## Controlled / injected evidence

All rows below were exercised by the passing unit suite. They are synthetic property, clock, event, and host callback tests, not live WM refusal or X server failure observations.

| Area | Evidence |
| --- | --- |
| Absolute transition rules | All target/observed combinations, Conflict, idempotence, opposite removal, no redundant addition after a matching removal, maximum one removal plus one addition |
| Property/capability parsing | Present empty and unrelated atoms; Above/Below/Conflict; absent mapped state rejected; wrong type/format, truncation, inconsistent payload and oversized data rejected; absent/missing support prevents mutation |
| Transport | Root destination, propagation false, both substructure masks, body client ID, format 32, absolute action and source 1 payload |
| Confirmation | Checked mutation followed by fresh read; immediate completion; relevant stale notifications reread current truth; unrelated notifications cannot confirm; no layer sequence gate |
| Fixed deadline | Sustained event fixtures, no extension during phases, final read success/mismatch, no addition at expiry, expiry during support query, no timer or resurrection after termination |
| Later ownership | A newly started drag and newly acquired popup leave addition enabled; interaction state and capture owner remain unchanged |
| Menu | All five row outcomes on matching release; layer-row mismatch/outside/wrong-button/chord rejection; canonical rendering labels match hits; larger menu fits corners or refuses invalid fit |
| Busy and cleanup | Selecting-popup checked release/destruction precedes dispatch; later layer selection is busy without replacing the original operation; failed release/destruction prevents dispatch |
| Failure/lifecycle | Injected read/support/send/check/post-send-read failures; no rollback; primary error survives later cleanup failures; Dismiss preserves pending work; Quit/duration/layout/body lifecycle terminate it |
| M02 regressions | All 112 original cases retain intent, including pointer freshness, release debt, movement/bounds, popup lifecycle and cleanup; Dismiss/Quit indices updated and selection coverage extended |

## Owner real-host instructions

Use the actual target Openbox/X11/Picom session. Before testing, record the executable revision and dirty status, WM/compositor versions, relevant rules, monitor/workarea/workspace facts, and exact launch command. Keep a portion of the body exposed during Below/Normal tests and use a finite duration when testing covered states. Independent recovery is M03.2 scope.

Suggested commands below are instructions, **not commands executed for this report**:

```sh
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --diagnose
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --delay 2 --duration 30
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --duration 120
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --duration 5
```

Right-click and release on the body to open Above / Normal / Below / Dismiss / Quit. Activate an item with a matching left press/release in its row. An outside-left dismissal consumes that click without replay.

| ID | Status | Owner procedure and evidence required |
| --- | --- | --- |
| L01 | NOT RUN | Delayed launch while typing in another application; record unchanged startup appearance and continued typing focus. |
| L02 | NOT RUN | Select Above against two partly overlapping ordinary windows; record actual coverage, body state diagnostic and released menu capture. |
| L03 | NOT RUN | Select Normal, then focus/raise ordinary windows; record neither layer flag and actual normal-layer ordering. No fixed relative position is promised. |
| L04 | NOT RUN | Select Below; ordinary windows should cover overlap. Keep an exposed body area for menu access; record actual ordering. |
| L05 | NOT RUN | Cycle Above → Below → Normal → Above repeatedly; record final flags, stability, one body and no activation. |
| L06 | NOT RUN | Select an already established layer again; record no-op diagnostic, no extra mutation, no unexpected flicker/capture/focus transfer. |
| L07 | NOT RUN | Establish Above and close the popup; enter an ordinary application's actual fullscreen mode. Record whether focused fullscreen naturally covers the body. Maximization alone is insufficient. |
| L08 | NOT RUN | Exit fullscreen without another layer request; record natural return to the previous Above relationship and continued application usability, or the exact WM limitation. |
| L09 | NOT RUN | Type continuously while performing several menu/layer changes; record the application receiving characters. `_NET_ACTIVE_WINDOW` alone is insufficient. |
| L10 | NOT RUN | After layer changes, test offset-preserving drag, release outside body, right-cancel, transparent-padding click-through and menu dismiss. Verify bounds, no stale action, no click replay and subsequent normal desktop input. |
| L11 | NOT RUN | Open the five-row popup at corners/edges and with a narrow usable area; record complete fit or safe refusal without retained capture. Do not change unrelated WM settings to force a pass. |
| L12 | NOT RUN | Let duration expire with the popup open and, if reproducible, a pending layer request. Record clean exit, exit status and usable input. A synthetic pending-state test is already covered above; do not relabel it real-host evidence. |
| L13 | NOT RUN | Perform at least 20 mixed layer/menu/drag operations in one process; record count and absence of stale ownership, orphan popup/body or control degradation. Check Quit and return to idle socket waiting. |

Optional one-shot property snapshots of the body and chosen fullscreen application can support these observations; the probe contains no fullscreen observer or active-window polling. A short idle CPU/RSS observation can supplement L13 without adding permanent telemetry.

For every row record actual outcome, focus observation method, exact command, environment differences and limitations. Do not copy expected results into an observed-results column. Unexpected fullscreen behavior is architectural evidence, not permission to add a raising/restacking loop.

## Remaining limitations and next gate

- Actual Openbox layering, fullscreen coverage/return, focus isolation, click-through and resource behavior on this build remain unverified by the owner.
- State confirmation proves current property flags, not visual stacking or the causal source of a WM change. Late changes after timeout are observed without retry.
- The one-second WM confirmation deadline assumes a responsive X server; synchronous X11 checks/replies have no new hard transport timeout.
- A partial transition can leave Normal if removal succeeds and addition fails. Diagnostics retain the readable result; there is no rollback.
- Below may cover the only menu control. Independent recovery remains deliberately deferred; finite duration is available.

Automated verification passed. Independent implemented-diff review, matching published-revision CI evidence and owner L01–L13 acceptance are pending. M03.1 is not yet accepted. Do not advance to M03.2 or other missions.
