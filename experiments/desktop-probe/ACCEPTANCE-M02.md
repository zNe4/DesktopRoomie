# A00-M02 integrated acceptance

Decision: initially Pending review

Evidence recorded: 2026-10-07, for A00-M02.7-C. Owner observations and supervising-review/CI facts below were supplied by the owner in the assignment. Local commands and source inspection were performed by Codex. Codex did not run graphical tests during this documentation mission.

## Revision identity

- Tested code revision R: `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b`.
- CI-tested SHA: `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b`.
- Local baseline: clean working tree on `m2.7b`, HEAD exactly R.
- Final documentation/review revision: **to be filled after publication and independent final review**.

Before preparing evidence, `git diff R -- experiments/desktop-probe/src experiments/desktop-probe/Cargo.toml experiments/desktop-probe/Cargo.lock .github/workflows` was empty (using the full SHA for R). Rust source, Cargo manifest/lockfile, and workflow therefore corresponded to R. Documentation may be committed after R without changing executable source; that documentation revision must be recorded separately rather than replacing the tested-code identity. No commit or push was made in this mission.

## Target environment

Owner-reported rediscovery on the real target desktop:

| Property | Observed value |
| --- | --- |
| System | Archcraft rolling; Linux `7.2.8-arch1-1` |
| Session | `XDG_SESSION_TYPE=x11`, `DISPLAY=:0` |
| Window manager | Openbox 3.6.1; process active |
| Compositor | Picom v13, revision `d87a5ba`; process active |
| Panel | Polybar active |
| rustc | `1.99.0 (b940084d7 2026-09-28)` |
| cargo | `1.99.0 (5f94df478 2026-08-27)` |
| Final monitor snapshot | One active monitor: `eDP-2`, primary, 1920×1080 at +0+0; 144 Hz active mode |
| Final root geometry | 1920×1080 |
| EWMH desktops | 5; current desktop 0 |
| Final `_NET_WORKAREA` | x=10, y=48, w=1900, h=1022 |
| Final active window | Previously focused application, not the probe |
| Probe diagnostics | Root depth 24 bpp; RENDER v0.11; SHAPE v1.1; 32-bit TrueColor alpha-capable candidates found |
| Diagnostic selection | Primary monitor `eDP-2`; final usable area x=10, y=48, w=1900, h=1022 |

The desktop configuration changed during acceptance. A secondary monitor was temporarily connected for T09; the owner later disconnected it and returned to the final single-monitor snapshot. That snapshot does not describe the entire run. The temporary monitor's identity/mode was not supplied for this session; historical `ENVIRONMENT.md` monitor values are not substituted for it.

`ENVIRONMENT.md` preserves M01's 2026-10-05 facts, including a secondary monitor, 2944×1280 root, and selected usable area 10,48,1910,1032. These are historical observations, not the final M02 snapshot. A dated reconciliation note links this record.

Focus evidence includes actual continued typing (T01/T10); active-window metadata is supporting evidence. No new direct input-focus query or statement about application-specific Openbox rules was supplied for this integrated session. Historical no-input hints remain documented; no new rule dependency is inferred.

## Integrated implementation mechanisms

R retains the managed fixed 160×160 procedural body, transparent input padding and translucent interactive patch. The startup-selected monitor identity supplies checked bounds. Movement uses position-only `ConfigureWindow` x/y requests, with root-space geometry confirmation and bounded correction. The initiating root-coordinate grab offset is preserved.

Body-left gestures use explicit pointer capture. Opening-right gestures track the server's automatic capture: normal unchorded completion ends that ownership locally; cancellation uses checked release. The popup obtains its own explicit pointer capture in one checked acquisition attempt. Body and popup ownership do not coexist. Cleanup attempts release before destruction, preserves fatal errors, and retains unacknowledged obligations. Non-activating hints preserve typing focus; the mouse-only menu has Dismiss and Quit, without keyboard capture/navigation or activation requests.

The origin limits use `max_x = usable.x + usable.width - body.width` and `max_y = usable.y + usable.height - body.height`, with checked fit/arithmetic and the 160×160 canvas. Earlier T08 logs included x=1760, y=910, and y=48 while configuration/workarea differed. Those logs are not attributed to the later workarea: the final 10,48,1900,1022 snapshot would yield maxima (1750,910). Earlier logs do not establish one common geometry snapshot or a contradiction with the final snapshot.

Implementation detail remains in [M02.5-IMPLEMENTATION.md](M02.5-IMPLEMENTATION.md) and [M02.6-IMPLEMENTATION.md](M02.6-IMPLEMENTATION.md).

## Local Codex verification

Executed from the repository root at R on 2026-10-07 under the desktoproomie-verify procedure, as explicitly required by this assignment. Local toolchain: rustc `1.99.0 (b940084d7 2026-09-28)`, cargo `1.99.0 (5f94df478 2026-08-27)`.

| Exact command | Actual result |
| --- | --- |
| `cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check` | PASS — exit 0 |
| `cargo build --manifest-path experiments/desktop-probe/Cargo.toml` | PASS — exit 0 |
| `cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings` | PASS — exit 0 |
| `cargo test --manifest-path experiments/desktop-probe/Cargo.toml` | PASS — exit 0; 112 passed, 0 failed, 0 ignored, 0 measured, 0 filtered out |
| `git diff --check` before documentation | PASS — exit 0 |

Final documentation whitespace/diff review is recorded below. These checks do not substitute for real-desktop acceptance.

The specification lists normal `cargo run`, `--delay 2 --duration 30`, `--duration 5`, and `--diagnose` launch variants with this manifest. The owner supplied scenario results and diagnostics, but not the exact command line for every manual run; those variants are reference commands, not newly asserted execution evidence.

## GitHub clean-environment CI

Workflow: `.github/workflows/desktop-probe-ci.yml`, inspected locally at R. It runs on `ubuntu-24.04`, installs stable Rust with rustfmt/clippy, records revision/toolchain, and executes the same four exact Cargo commands listed above. The observed run toolchain was rustc 1.99.0 / cargo 1.99.0; the workflow uses the stable channel rather than a version pin.

| Milestone | Exact checked SHA | Run | Owner-supplied result |
| --- | --- | --- | --- |
| M02.7-A workflow establishment | `99036b843a2b0fb9823f6419377c841c20de8743` | [37578695433](https://github.com/zNe4/DesktopRoomie/actions/runs/37578695433) | SUCCESS; 112 tests passed |
| Final frozen implementation R | `d8dc1abef8b6a8d9395342b234f2f0beb912bc7b` | [37579752824](https://github.com/zNe4/DesktopRoomie/actions/runs/37579752824) | SUCCESS; Format PASS; Build PASS; Clippy `-D warnings` PASS; Tests PASS — 112 passed, 0 failed |

Final acceptance relies on run **37579752824 for R**, not only the earlier workflow-establishment run. CI outcomes are recorded from the owner's supplied evidence and supervising review; Codex did not independently fetch the remote run in this mission.

## Independent ChatGPT review

The owner reports that M02.7-A's workflow was independently reviewed and approved. The supervising ChatGPT independently reviewed R's B1 source diff and exact GitHub CI run and approved R for integrated manual acceptance. No review ID or timestamp was supplied. These approvals preceded integrated acceptance; they do not replace the pending independent final review of this record.

## Owner integrated T01–T26 matrix

All real-host rows record the owner's observations at frozen revision R. Controlled rows record Codex's deterministic reruns approved as evidence by the supervising review. PASS here describes the row's evidence; the final mission decision remains pending review.

| ID | Status | Method | Actual observed result | Evidence type / limitations |
| --- | --- | --- | --- | --- |
| T01 | PASS | Delayed launch while typing | Typing in the previously focused application continued through mapping; no focus theft. | Owner, real-host; exact launch command not supplied. |
| T02 | PASS | Click transparent padding before/after moving | Underlying controls received clicks; padding started no gesture; transparency/compositor behavior remained correct. | Owner, real-host. |
| T03 | PASS | Small-jitter body click and valid release | Exactly one toggle; no drag; prior application retained focus. | Owner, real-host. |
| T04 | PASS | Release pending click outside shape | No toggle or movement; normal pointer use resumed. | Owner, real-host. |
| T05 | PASS | Drag from multiple body points and translucent patch | Grab offset stayed stable; no jump to pointer center. | Owner, real-host. |
| T06 | PASS | Cross threshold, return, release | Gesture remained a drag after returning; no click toggle. | Owner, real-host. |
| T07 | PASS | Fast drag across another application/outside original footprint | Release completed correctly; subsequent application click/scroll worked normally. | Owner, real-host. |
| T08 | PASS | Drag to edges/corners/panel bounds | Body remained correctly bounded; logs included requested origins (1760,910), (1760,48) and verified map (1760,910). | Owner, real-host; earlier logs used different configuration/workarea from final snapshot. |
| T09 | PASS | Move pointer to temporary secondary monitor during gesture, release | Pointer reached other monitor; body did not migrate; release completed, capture ended, body stayed on selected monitor. | Owner, real-host; secondary monitor later disconnected. Not N/A. |
| T10 | PASS | Type while holding and moving | Typing remained in the previously focused application. | Owner, real-host. |
| T11 | PASS | Stationary left hold ≥10 seconds, ordinary release | No timer-driven release; ordinary release worked afterward. | Owner, real-host; no more precise duration supplied. |
| T12 | PASS | Middle/wheel input during left hold | No action or incorrect gesture termination. | Owner, real-host. |
| T13 | PASS | Right-cancel pending and dragging gestures | Both cancelled cleanly; no stale action; subsequent interaction worked. | Owner, real-host. |
| T14 | PASS | Duration expiry without input | Automatic termination occurred correctly; desktop input remained usable. | Owner, real-host. |
| T15 | PASS | Duration expiry while left held/dragging | Capture released; desktop input recovered immediately. | Owner, real-host; no measured recovery latency supplied. |
| T16 | PASS — CONTROLLED/INJECTED | Exact denial-test reruns and source assertions | Denials created no false owner/drag/menu, made one attempt, recovered usable state and attempted correct popup cleanup. | Codex, controlled/injected; 3 passing reruns; no live denial race. |
| T17 | PASS | Popup at all four corners, each edge midpoint, and anchors at sprite/canvas border at display edges/corners | Entire popup readable and fitted correctly; typing focus preserved; opening remained correct. | Owner, real-host. |
| T18 | PASS | Dismiss item, fresh right dismissal, outside left dismissal | All worked; first outside click only dismissed; subsequent fresh application input worked. | Owner, real-host. |
| T19 | PASS | Press item, release elsewhere or over other item | Neither mismatch activated an item; menu remained usable. | Owner, real-host. |
| T20 | PASS | Quit and duration expiry with menu open | Cleanup left no orphan popup or stuck capture. | Owner, real-host. |
| T21 | PASS | Workspace changes during held gestures, dragging, open menu | Interaction cancelled correctly; character did not follow to other workspace; returning to launch workspace left it usable. | Owner, real-host. |
| T22 | PASS — CONTROLLED/INJECTED | Exact layout/bounds/reconciliation reruns and source assertions | Identity, invalid-fit errors, interruption cancellation, placement gating and checked correction/refusal paths passed. | Codex, controlled/injected; 9 passing reruns; physical unplug/reconfiguration not claimed as row evidence. |
| T23 | PASS — CONTROLLED/INJECTED | Exact lifecycle/error/cleanup reruns and source assertions | Interruption, resource cleanup ordering, retained release obligations and original error preservation passed. | Codex, controlled/injected; 10 passing reruns; no real X server disconnection. |
| T24 | PASS | At least 20 drag/release/menu/dismiss cycles in one process | No accumulating jumps, stale actions, orphan popup, stuck capture or degradation. | Owner, real-host; exact cycle count beyond required minimum not supplied. |
| T25 | PASS | 90.26-second idle resource observation after interactions | 19 samples: RSS 3880 KiB throughout, state SN, wait channel poll_s; no additional CPU ticks detected. | Owner, real-host; process-only sampling and kernel-counter limits detailed below. |
| T26 | PASS | Normal menu Quit and shell exit check | Quit selected; probe exited cleanly; shell exit status 0; desktop input remained usable. | Owner, real-host. |

Owner-supplied runtime excerpts for T08:

```text
[INPUT] Drag released at requested origin (1760, 910)
[INPUT] Drag released at requested origin (1760, 48)
[PLACEMENT] Verified map at (1760, 910)
```

Owner-supplied normal Quit evidence for T26:

```text
[MENU] Quit selected
  Probe exited cleanly.
Probe exit status: 0
```

## Controlled failure evidence

Executed locally at R on 2026-10-07 after inspecting the source assertions. Every requested test name matched; no naming discrepancy was found. Each fully qualified name below was run independently using:

```sh
cargo test --manifest-path experiments/desktop-probe/Cargo.toml <fully-qualified-test-name> -- --exact
```

Each invocation exited 0 and reported **1 passed, 0 failed, 0 ignored, 0 measured, 111 filtered out**. T16: 3 invocations; T22: 9; T23: 10. The two tests shared by T22/T23 were rerun for each row: 22 invocations, 20 distinct tests. These results are separate from the full 112-test suite.

### T16 — Capture acquisition denial

Source assertions and execution confirm that SUCCESS is the only tested acquisition status that establishes active ownership. ALREADY_GRABBED, INVALID_TIME, NOT_VIEWABLE, and FROZEN each make exactly one attempt and leave no owner or false MenuOpen state. Body denial returns to Idle and permits a fresh press; popup denial invokes teardown once, clears popup resources, and restores the input gate without an ungrab for capture never acquired. This is deterministic injected evidence, not a real X server raced for ownership.

| Exact test | Source file under `experiments/desktop-probe/` | Rerun result |
| --- | --- | --- |
| `interaction::tests::test_denied_grab_recovers_to_idle` | `src/interaction.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `x11::pointer::menu_tests::explicit_success_and_every_denial_make_one_attempt_without_contradictory_owners` | `src/x11/pointer.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `menu_runtime_tests::popup_acquisition_success_and_each_denial_have_single_attempt_and_checked_teardown` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |

### T22 — Usable-area/display/layout changes

Source assertions and execution confirm stable selected-monitor identity across primary changes and an error on removal; absent workarea is distinct from malformed/truncated data; empty, overflowing, too-small, or unsupported wire-coordinate bounds are rejected. Relevant workspace/workarea events interrupt pending movement, and body/popup lifecycle events trigger neutral cancellation. Unavailable bodies defer placement, remap correction is issued once and must be verified before input resumes, and refused correction or movement failure preserves the error while cleanup is attempted. These fixtures and callbacks do not establish physical unplug/reconfiguration.

| Exact test | Source file under `experiments/desktop-probe/` | Rerun result |
| --- | --- | --- |
| `x11::monitors::tests::primary_change_preserves_identity_and_removal_is_an_error` | `src/x11/monitors.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `x11::monitors::tests::absent_and_malformed_workarea_are_distinct` | `src/x11/monitors.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `x11::monitors::tests::intersections_are_checked_and_never_expand_empty_or_invalid_fit` | `src/x11/monitors.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `geometry::tests::test_body_too_large_and_empty_region` | `src/geometry.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `tests::workspace_workarea_and_root_geometry_are_relevant_but_unrelated_properties_are_not` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `tests::unavailable_body_defers_placement_and_verified_remap_requires_correction_first` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `tests::refused_correction_fails_without_retry_and_movement_failure_still_cleans_up` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `tests::unsupported_x11_coordinate_range_is_rejected_before_placement` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `menu_runtime_tests::layout_and_window_lifecycle_identify_interruptions_before_refresh` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |

### T23 — Lifecycle/host failure and original error preservation

Source assertions and execution confirm interruption classification and discarded coalesced movement while ordinary release remains ordered for completion; unavailable bodies cannot move or accept input. Simulated external popup destruction skips window destruction but still frees GC/font resources. Corrective-move failure attempts ungrab before renderer/window destruction and retains the primary error; popup acquisition/release/destroy failures likewise retain the original fatal diagnostic and explicit release obligations. Failed release still attempts popup destruction, acknowledged retry clears ownership, acknowledged cleanup is idempotent, and unacknowledged resource destruction cannot be relabeled successful or blindly repeated. This is controlled deterministic evidence, not an actual disconnection from a failing live X server.

| Exact test | Source file under `experiments/desktop-probe/` | Rerun result |
| --- | --- | --- |
| `tests::interruption_discards_coalesced_move_and_release_preserves_flush_order` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `tests::unavailable_body_defers_placement_and_verified_remap_requires_correction_first` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `menu_runtime_tests::layout_and_window_lifecycle_identify_interruptions_before_refresh` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `x11::menu::tests::external_destruction_skips_window_but_releases_independent_resources` | `src/x11/menu.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `tests::failed_corrective_move_preserves_primary_error_and_attempts_release_before_destruction` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `menu_runtime_tests::fatal_error_keeps_priority_over_popup_release_and_destroy_failures` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `menu_runtime_tests::acquisition_protocol_failure_retains_resources_for_primary_error_cleanup` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `menu_runtime_tests::failed_release_still_destroys_popup_and_cleanup_retry_retains_owner_until_ack` | `src/main.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `x11::pointer::tests::failed_release_retains_obligation_and_confirmed_retry_is_idempotent` | `src/x11/pointer.rs` | PASS — 1 passed, 0 failed; exit 0 |
| `x11::resource::tests::cleanup_is_idempotent_and_does_not_claim_unacknowledged_success` | `src/x11/resource.rs` | PASS — 1 passed, 0 failed; exit 0 |

## T25 idle/resource observations

Owner-performed observation after interactions: PID **823333**, command **desktop-probe**, actual interval **90.26 seconds**. The owner sampled process data using `ps` every 5 seconds (19 samples), and calculated interval CPU from the change in `/proc` CPU counters with `CLK_TCK=100`.

| Measurement | Owner-reported result |
| --- | --- |
| RSS in all 19 samples | 3880 KiB, stable |
| Process state in all samples | SN |
| Wait channel in all samples | poll_s |
| Lifetime `ps %CPU` | Declined from 1.7 to 0.6 |
| Interval CPU_ticks | 0 |
| CLK_TCK | 100 |
| Interval CPU percent of one core | 0.0000 |
| One-tick resolution | 0.0111 percentage points |

No additional CPU ticks were detected during the 90.26-second interval at the kernel counter resolution. This does not prove literally zero wakeups or zero CPU use. Lifetime `ps %CPU` is distinct from the interval counter calculation. Stable RSS and consistently sleeping in `poll_s` support the intended event-driven idle behavior during this observation. Sampling covers this process only, not X server/compositor memory or CPU. Raw sample files and the exact measurement shell script were not supplied; the values above are the owner's reported measurements.

## Known limitations / untested cases

- T16 uses deterministic denial statuses, not a live X server ownership race.
- T22 uses controlled layout/usable-area fixtures and failures; physical monitor unplug/reconfiguration is not the evidence for that row. The owner's later secondary-monitor disconnection does not establish a separate tested live failure sequence.
- T23 uses deterministic lifecycle and host-error injection, not an actual X server disconnection. After real connection loss, cleanup can only be best effort; unit results do not prove server acknowledgement.
- T25 does not measure X server/compositor resources or count wakeups, and sub-tick CPU use is below the reported counter resolution.
- Compatibility outside the recorded Openbox/X11/Picom target is not established.
- The final snapshot had one active monitor; T09 occurred with a temporarily connected secondary monitor. Earlier bounds logs cannot be assigned the later workarea.
- Exact commands for each owner manual run, an integrated direct input-focus query, and application-specific Openbox-rule details were not supplied. Continued typing is actual focus evidence; no missing details have been invented.

These limitations are disclosed for independent review. The specification permits controlled evidence for T16/T22/T23, and the supervising review explicitly approved that method for this acceptance. No stuck capture, focus theft, invalid placement or broken quit/recovery was reported. This record does not approve later A00/A01 roadmap work.

Final documentation checks: `git diff --check` passed (exit 0). Because the new acceptance file was untracked, `git diff --no-index --check /dev/null experiments/desktop-probe/ACCEPTANCE-M02.md` was also run: no whitespace diagnostics; exit 1 reflects the new-file difference under `--no-index`, not an exit-0 pass. The complete new file and tracked environment-note diff were reviewed for factual accuracy. Final source/Cargo/workflow comparison against R remained empty; historical acceptance records were unchanged. No graphical tests were rerun for documentation edits.

## Final decision

Pending independent final review
