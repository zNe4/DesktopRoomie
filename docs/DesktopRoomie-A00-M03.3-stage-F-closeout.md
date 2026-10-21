# A00-M03.3 Stage F — deterministic test-hardening closeout

**Status (2026-10-21): COMPLETED and independently reviewed.** Accepted Stage-F executable: `f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863` on `m03`; [matching CI 39688537222](https://github.com/zNe4/DesktopRoomie/actions/runs/39688537222) passed **760 tests / 0 failures** plus format, build and all-target Clippy. This is **deterministic source-and-CI acceptance only**. No new owner-native Stage-F host test was performed; **Stage G integrated M03.3 native acceptance remains pending, separately authorized and not implied by this record**.

## 1. Source and evidence identity

The original [F-P read-only audit](DesktopRoomie-A00-M03.3-stage-F-coverage-audit.md) was performed against a **737-test** source baseline at docs HEAD `0b0f1b649460f0150adfccfbea17fedde010dd93`. Its G1–G3 findings were gaps in composed deterministic test coverage, **not demonstrated production defects**. The historical audit (including proposals, pinned links and its original state) remains unchanged.

Previously accepted public Recover feature/native testing belongs to frozen E4 `aa00f8e94aff1cf2ef1ff6a6cecf734f822d5603` ([CI 39529051360](https://github.com/zNe4/DesktopRoomie/actions/runs/39529051360), 737 tests; [qualified N01–N19 evidence](../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md)). RF2d later provided seven qualified native regression categories, recorded in the [RF closeout](DesktopRoomie-A00-RF-modularization-plan.md#rf2d-owner-native-regression-evidence). None is relabelled as native Stage-F acceptance.

## 2. Accepted checkpoint chain

| Checkpoint | Reviewed executable commit | Matching successful GitHub CI | Total tests / new identities | Test-only scope |
| --- | --- | --- | --- | --- |
| F-T1a | [`a9d6f5d86b2c621aa821f82c56fb080b5b995653`](https://github.com/zNe4/DesktopRoomie/commit/a9d6f5d86b2c621aa821f82c56fb080b5b995653) | [39683061384](https://github.com/zNe4/DesktopRoomie/actions/runs/39683061384) | 741 (+4) | SelectionClear / DestroyNotify across eight Recover snapshots; true or stale ownership, shutdown, one reply attempt. |
| F-T1b | [`394d58eeb8fc88a8b2905f74645d60ab8aa93c94`](https://github.com/zNe4/DesktopRoomie/commit/394d58eeb8fc88a8b2905f74645d60ab8aa93c94) | [39684549706](https://github.com/zNe4/DesktopRoomie/actions/runs/39684549706) | 747 (+6) | Closing query/publication failures enter Recover fatal policy; original-error and history priority; successful replacement-owner distinction. |
| F-T2 | [`03a11c0b1683ef91a77778cf709fc670611bf91f`](https://github.com/zNe4/DesktopRoomie/commit/03a11c0b1683ef91a77778cf709fc670611bf91f) | [39686683004](https://github.com/zNe4/DesktopRoomie/actions/runs/39686683004) | 754 (+7) | Late event → fresh successor admission, no inherited proof/authority; 64-event boundary, body destruction, replay/correlation. |
| F-T3 | [`f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863`](https://github.com/zNe4/DesktopRoomie/commit/f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863) | [39688537222](https://github.com/zNe4/DesktopRoomie/actions/runs/39688537222) | 760 (+6) | Remote/local layer Busy at Here/Recover phases, menu cleanup-before-dispatch, pending controller, post-terminal reuse. |

Independent review compared individual passing test names between the consecutive matching CI logs. From **737 → 741 → 747 → 754 → 760**, there were **23 new tests, zero lost or renamed existing identities**. The four commits were limited to existing test modules: `experiments/desktop-probe/src/probe_tests/here_service_tests.rs`, `src/probe_tests/layer_runtime_tests.rs` and `src/x11/control/transport_tests.rs`. **There were no production Rust, protocol, Cargo, workflow or host-configuration changes.** The test changes are reviewable from the four linked commits; the final executable SHA is distinct from this subsequent documentation commit.

## 3. G1–G3 disposition

- **G1 — Recover lifecycle and Closing errors: CLOSED at deterministic integration-test scope.** F-T1a joins actual `ControlWindow::handle_event_with` loss/destruction handling to eight production-driven Recover snapshots. F-T1b joins `closing_with` query/write failures to `finish_probe_run_with` and actual placement fatal helpers. Tests preserve original-error priority, genuine historical progress, cleared current proof, ownership-loss distinction and a single terminal attempt.
- **G2 — Delayed-event turnover: CLOSED at deterministic integration-test scope.** F-T2 exercises production event observation, bounded draining, owner service/transport admission, delayed property/map/unmap/configure events, body Destroy handling and successor correlation. Stale events may demand fresh reads; they cannot transfer success evidence, retry/correction authority or deadlines. Test-owned event driving is not the full native scheduler.
- **G3 — Local layer contention: CLOSED at deterministic integration-test scope.** F-T3 covers all eight Recover phase snapshots and standalone Here, reachable Above/Normal/Below menu gestures, actual release/destruction-before-Busy sequencing, cleanup errors, pending single-controller Busy admission, and subsequent legitimate local/remote reuse. Invalid popup/fence combinations are not invented.

Other requirements from the [Stage F §11 contract](DesktopRoomie-A00-M03.3-staged-implementation-roadmap.md#11-stage-f--lifecycle-concurrency-and-deadline-hardening) already had substantial baseline deterministic coverage (remote command pairs, identity/epoch, destination drift, dispatch uncertainty, input cleanup, fixed deadlines and >64-event load, OutcomeUnknown/terminal errors and full M03.2 regressions). Stage F did not recreate those matrices or claim every physical failure interleaving was exercised.

## 4. Architecture and invariants preserved

- `ControlWindow.active` remains the sole owner-operation slot; `ProbeRuntime.layers` is still the sole `LayerController`. There is no alternate event scheduler, command queue, new protocol, retry service, body or resource owner.
- Here→Top Recover handoff is one-way with frozen destination, original request/epoch and fixed 7000 ms owner deadline. Current success evidence is rebuilt, confirmed history is preserved, consumed correction authority never replenishes, and at most one terminal send is attempted.
- Existing local/remote layer gating, pointer/popup cleanup, input-fence safety, focus/workspace isolation and M03.2 Hide/Show/Bring Top contracts remain non-regression obligations.
- RF1/RF2a–d module architecture remains accepted. RF2e (layer-service extraction) and RF2f (caller-lifecycle extraction) remain **deferred**; no new production refactor is justified by the Stage-F tests.

## 5. Residual Stage G native and scheduler evidence

Stage F did **not** execute the complete `run_probe` RustConnection event loop under arbitrary native X11 traffic; test-owned event sequences and 64-event draining are deterministic helper evidence. Native X replies still lack hard synchronous transport timeouts.

Separate owner-native verification is required for typing focus/current desktop preservation, actual pointer grabs and popup releases, transparency/click-through, drag, stack/fullscreen behavior, genuine minimized/off-workspace representations, and compositor/WM behavior. Real X-server or peer disappearance, refused/malformed WM transitions, destination/workarea/viewport/monitor drift, hot-plug/removal, terminal reply loss, caller OutcomeUnknown and race/deadline interleavings were **not all physically reproduced**. A fake's recorded terminal-send attempt is not proof of delivery. Earlier monitor-retention observation did not constitute a hot-plug test.

For Stage G, label each real-host check **PASS / FAIL / NOT EXERCISED / CONDITIONAL**, with exact environment and source SHA; preserve injected-only evidence separately. Native refusal or destructive disconnection scenarios must not be fabricated as passes or tested without safe, explicit authorization.

## 6. Next gate — Stage G (not yet accepted)

Per the [approved Stage G roadmap](DesktopRoomie-A00-M03.3-staged-implementation-roadmap.md#12-stage-g--integrated-m033-acceptance), separately authorize: freeze the exact executable SHA (currently `f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863` if unchanged), perform local fmt/build/Clippy/tests, independently review source and matching CI, complete owner-native **Arch Linux / X11 / Openbox** acceptance, record actual observations and qualifications, and obtain explicit owner sign-off.

**This closeout does not authorize Stage G, M03.4, A01, fault injection or user host configuration changes.** Consult [STATUS](STATUS.md) and [INDEX](INDEX.md) for current lifecycle pointers.
