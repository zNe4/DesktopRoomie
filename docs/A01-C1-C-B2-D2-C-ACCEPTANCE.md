# A01-C1-C-B2-D2-C — Headless Vulkan acceptance record

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-24**

Independent ChatGPT source review of D2-B-R2 and D2-C-A-R2, review of owner-executed N1, N2 and N3 evidence, and D2-C-F documentation review passed. The owner explicitly granted technical acceptance of D2-B/D2-C on 2026-11-24. Git publication remains pending and owner-controlled; D3 requires separate authorization.

## 1. Frozen identities and review provenance

Branch `a01`; published HEAD and live remote `refs/heads/a01` verified as `4bfe899ed0f3dd18e721b6e86b5d53dd85c24b43` (D2-A-R1 supervisor baseline). The index has no staged changes. The reviewed implementation remains a local unpublished candidate over that HEAD.

| Reviewed artifact | SHA-256 |
| --- | --- |
| Accepted D2-B-R2 cumulative patch | `c3cd1dbbae5f0121ecd3ba0be1a7ef1e9c73911aac39ef394054d8e753768e0a` |
| Reviewed D2-C-A-R2 cumulative patch against published HEAD | `860fb24d315bd7539e78831ee7e5547c040f16c4bf3acda223fc3a42685d0c7d` |
| D2-C-A-R2 source review archive | `fd463844490a2fc96d40aece04075c609e49d694f6c0c4a08240cb1924232941` |
| Frozen 187-file source manifest (`candidate-source.sha256`) | `7229a36791c9506eb11b1c191b731a5e8b4f87c71a3883c3fcb02ebf2a3e2cd0` |
| Release acceptance executable (`roomie-worker-acceptance`) | `9db8a51387486b929588e674d179809e5a8f2ce5cd5505a318134d0a386e71bd` |
| Release worker (`roomie-worker-vulkan`) | `aec5c2d4b142888f11a34c80db4f23889013942f152904f98a89126a156d8625` |
| Independent accepted CPU fixture (`presentation/synthetic.rs`) | `9bcca46707113b45f897e2ec688e4ff2467efb14d4416335a8d5729238ebb099` |

Codex independently rehashed all 187 working-tree files against the reviewed manifest before editing, checked all 419 review-archive content entries and its 187 archived source files, verified both current release binaries, and verified all seven native archives below. Every native archive retains the same frozen source manifest and binary identities; its regular-file contents match the retained extraction. Private archives and raw logs remain private. Public identities here permit evidence correlation without publishing machine-specific paths.

D2-C-F edits only this new record, [STATUS](STATUS.md), [ARCHITECTURE](ARCHITECTURE.md) and [INDEX](INDEX.md). Of the original 187 files, three existing documents change and **184 remain byte-identical**; the new record makes 188 candidate files. All previously reviewed implementation changes remain included. Before/after manifests, incremental and cumulative patches, their hashes, separate scratch patch replays, link/reference checks, whitespace checks and the full Git state are retained in the private D2-C-F review package.

### Independent source review history

| Checkpoint | Correction and review boundary |
| --- | --- |
| D1 and D2-A-R1 | Previously accepted pure handoff and published process supervisor. Their earlier acceptance identities remain separate. |
| D2-B initial / R1 | Independent review required explicit native acceptance/completion proof, normal-return native-error observation, checked unmap and callback handling, full correlated settlement evidence, first-cause versus later-fault authority, and one protected release path. R1 added those corrections and deterministic regressions. Its behavior that could finish an invalidated terminal prefix required R2 correction; the earlier candidate is not the final accepted contract. |
| D2-B-R2 | Invalidated partial Frame/JobFailed/rejection or Stopped permanently abandons transport, permits no further writes and retains uncertainty. Ordinary late faults with still-valid settlement retain separate treatment. R2 also made intentional-release context unwind-safe and blocked retry/destruction after interrupted release. Forty additional worker identities preserved all R1 identities. Independent ChatGPT source review passed; the owner-approved patch identity is frozen above. |
| D2-C-A initial / R1 | Review required fail-closed stress correctness and a per-job ledger, exact workload-specific telemetry reconciliation, expected initialization-rejection classification, complete map/callback/timed-operation validation, and precise observation windows/cohorts with no catch-up pacing. R1 added 15 worker identities per configuration, distinct local withdrawal and independently observed lifecycle timings. It preserved native settlement and wire contracts. |
| D2-C-A-R2 | The original failed N1-A observation motivated a fixed exact-only freshness allowance and separate original-reference reporting, described in section 2. Eight GPU-free regression identities per worker configuration preserved R1 behavior and all prior test identities. Independent ChatGPT source/patch review passed for the frozen R2 candidate. |
| N1 / N2 / N3 | The owner executed the frozen release candidate. Independent ChatGPT inspection of the actual archived evidence passed, as reported by the owner for this assignment. Cargo execution belongs to Codex's recorded verification, not to ChatGPT's independent inspection. |
| D2-C-F | Independent ChatGPT documentation review PASS; the owner explicitly approved D2-B/D2-C technical acceptance on 2026-11-24. Git publication remains pending. |

## 2. Historical failure and fixed R2 policy

The original **failed R1 N1-A observation**, retained privately under evidence identity `run.ibS7aAaL`, remains a failure. On llvmpipe, the first submission wait was **127.485213 ms**, beyond the original **100 ms** freshness budget. One operation physically settled, mapped/unmapped and became obsolete; **no exact frame was accepted and no pixel mismatch was established**. Controlled resource release, Stopped, successful exit and child reaping succeeded. This historical operation is excluded from the R2 totals below. Its retained files match the frozen R2 review copy and remain unchanged.

R2 changes only exact-workload freshness to **1,000 ms**, supplied consistently to D1 and the supervisor. Stress, cancel, cancel-before, unsupported and init-failure keep **100 ms**. All workloads keep initialization 5,000 ms, escalation 2,500 ms, graceful shutdown 500 ms and exit observation 500 ms. There are no adaptive retries, extra native prewarming, discarded failed warmup frames or backend fallbacks. Exact still validates 25 warmup and 100 measured frames; its longer correctness allowance is separate from performance readiness.

The original **100 ms reference** remains independently reported for warmup, measured and total owner end-to-end observations, including obsolete frame observations. Equality is a violation. Both successful R2 exact runs reported **0 warmup / 0 measured / 0 total** reference violations. This does not erase the original failed R1 observation or establish universal cold-start performance.

The first successful R2 software exact wrapper reported a **false final abort caused by a whole-line grep predicate**, as disclosed by the owner. The acceptance executable itself exited zero; its effective policy, 125 accepted frames, complete telemetry and lifecycle were independently verified. No implementation correction or rerun was required. The wrapper incident is separate from the original failed R1 workload.

## 3. Implemented headless path and native environments

The complete path is authoritative `FrameRequest` → D1 `GpuHandoff` → D2-A-R1 supervisor → actual private Unix IPC → persistent D2-B-R2 Vulkan subprocess → supervisor → D1 → independent acceptance validation. The harness does not link the renderer to obtain native frames. The owner retains the original request and full session/epoch/serial/generation/sequence identity. Both authorities use the same owner-observed monotonic time; backend faults fence eligibility before ready consumption.

The minimal D1 `SettledWithoutPayload` outcome discharges matching obsolete work without fabricating pixels, a ready frame or a GPU failure. Cancellation, timeout, process death, socket closure and reaping never supply physical settlement. Bounded optional telemetry corroborates operations and resources; it is never settlement authority. [Implemented ownership](ARCHITECTURE.md#headless-worker-ownership), [worker native contract](../experiments/gpu-worker/vulkan/NATIVE-CONTRACT.md)

| Environment | Actual archived identity |
| --- | --- |
| N1 software | `llvmpipe (LLVM 23.1.5, 256 bits)`; type `Cpu`; backend `Vulkan`; driver `llvmpipe Mesa 26.3.1-arch1.3 (LLVM 23.1.5)` |
| N2 hardware | `NVIDIA GeForce RTX 3050 6GB Laptop GPU`; type `DiscreteGpu`; backend `Vulkan`; driver `NVIDIA 620.73.04` |
| N3 unavailable ICD | Explicit Vulkan driver manifest naming a deliberately nonexistent shared library; diagnostic `required Vulkan adapter unavailable`; no adapter/device/pipeline successfully initialized. |

The owner ran these on the recorded Arch Linux host. Hardware selection did not silently fall back to software. N1 is real Vulkan through a software driver, separate from the CPU-only test fixture.

## 4. Nine owner-native acceptance workloads

All nine acceptance invocations and their worker processes exited zero for their expected workload outcomes. **Eight sessions initialized successfully; N3 instead passed the expected initialization-rejection workload.** Each invocation independently reports controlled release, Stopped, successful child reaping, confirmed shutdown, no termination request and zero unresolved supervisor/D1 debt.

| Workload | Native result | Owner attempts / admitted / dispatched | Valid / obsolete / rejected |
| --- | --- | --- | --- |
| N1-A-R2 exact | 25 warmup + 100 measured exact frames | 125 / 125 / 125 | 125 / 0 / 0 |
| N1-B cancel after dispatch | Actual complete dispatch barrier; cancellation before owner consumption; genuinely settled obsolete result, no ready publication | 1 / 1 / 1 | 0 / 1 / 0 |
| N1-C cancel before dispatch | Actual zero-byte local withdrawal; no Render transmitted or GPU operation | 1 / 1 / 0 | 0 / 0 / 1 |
| N1-D stress | 693 dispatched and validated frames; 27 missed opportunities | 693 / 693 / 693 | 693 / 0 / 0 |
| N2-A exact | 25 warmup + 100 measured exact frames | 125 / 125 / 125 | 125 / 0 / 0 |
| N2-B cancel after dispatch | Actual complete dispatch barrier; genuinely settled obsolete result, no ready publication | 1 / 1 / 1 | 0 / 1 / 0 |
| N2-C unsupported intent | One IPC Render rejected before native submission; no GPU work | 1 / 1 / 1 | 0 / 0 / 1 |
| N2-D stress | 694 dispatched jobs; 693 validated frames and one late settled obsolete result; 26 missed opportunities | 694 / 694 / 694 | 693 / 1 / 0 |
| N3 init-failure | Expected correlated `BackendFault`, retained diagnostic and no job | 0 / 0 / 0 | 0 / 0 / 0 |

Owner `rejected` includes both local withdrawal and the explicit unsupported-intent rejection; they have different transmission facts. All rows have zero owner busy and failed counters. Cancellation counts are invalidation calls, not proof of GPU cancellation. The post-dispatch tests control when the owner observes the result; they do not claim interruption of GPU execution. Obsolete results discharge genuine debt and contribute no accepted pixel frame.

### Exact parity and accepted fixture phases

Each adapter accepted **125/125** frames with zero differing bytes: 25 warmup and 100 measured, including every warmup frame. Each phase occurs 25 times per adapter. Each frame checks all **102,400 bytes**, tightly packed top-left **160 × 160 premultiplied BGRA8**, against the independently accepted CPU fixture; exact alpha masks, visible bounds, canonical hit spans and request/job identity also match. Metadata is reconstructed from returned bytes, rather than transmitted as independent worker metadata.

| Phase | Owner active elapsed | Accepted coverage characteristic |
| --- | --- | --- |
| 0 | 0 ms | Initial visible rectangle |
| 1 | 300 ms | Equal-coverage color change |
| 2 | 600 ms | Translucent region with a hole |
| 3 | 900 ms | Disconnected regions |
| 4 | 1200 ms | Entirely transparent frame |

The fixture repeats at 1500 ms; owner-supplied active elapsed time determines phase. The fully transparent frame is an exact-parity case, not evidence of X11 click-through.

### Resource and operation reconciliation

For every row, **each** of these nine counters equals N: submission attempts, accepted submissions, confirmed completions, native GPU completions, map requests, map callbacks, successful maps, verified unmaps and timed operations. These are separate recorded fields, not interchangeable proof predicates.

| Workload | Each operation count N | Allocations | Reuses | Retirements | Peak outstanding |
| --- | --- | --- | --- | --- | --- |
| N1-A-R2 | 125 | 1 | 124 | 0 | 1 |
| N1-B | 1 | 1 | 0 | 0 | 1 |
| N1-C | 0 | 1 | 0 | 0 | 0 |
| N1-D | 693 | 1 | 692 | 0 | 1 |
| N2-A | 125 | 1 | 124 | 0 | 1 |
| N2-B | 1 | 1 | 0 | 0 | 1 |
| N2-C | 0 | 1 | 0 | 0 | 0 |
| N2-D | 694 | 1 | 693 | 0 | 1 |
| N3 | 0 | 0 | 0 | 0 | 0 |

Across these R2 workloads there are **1,639 confirmed Vulkan completions**, eight persistent staging allocations, 1,633 staging reuses, **zero staging retirements** and **zero unresolved jobs**. The zero-operation initialized cancellation/unsupported sessions still allocate their persistent staging buffer once. N3 allocates none. Every row has complete native telemetry, controlled release = 1, uncertain release = 0 and telemetry error = 0. Resource release is corroborated by nine independent Stopped/zero-exit/reaping/confirmed-shutdown outcomes, including safe close after N3 rejection.

### N3 expected failure classification

The explicit unavailable-ICD run transmitted Initialize, retained `failure: Some(BackendFault)` and diagnostic **`required Vulkan adapter unavailable`**, and never admitted a job. It reports initialization_complete = 0; zero submissions, accepted submissions, completions, mapping requests/callbacks/successes/unmaps, staging allocations/reuses/retirements, outstanding work and timed operations. Every worker stage timing aggregate is zero. Device and pipeline setup timing fields have unavailable zero defaults after rejection, not measured zero successful initialization. Its correlated expected failure coexists with truthful controlled empty-backend release and confirmed child shutdown; it is not a ninth successfully initialized Vulkan session.

## 5. Performance observations and qualifications

**Strict 60 Hz readiness: NOT ACHIEVED.** Both fixed observation windows contain **577 valid frames in [2 s, 12 s)**, a **10-second denominator** and **57.7 Hz**, against the **60 Hz target**. This is no claim of production 60 FPS.

Stress keeps 120 warmup and 600 measured request opportunities, rational 60 Hz pacing, no catch-up queue and at most one outstanding job. Opportunity cohorts and observation-window outcomes are independent: post-window settlement remains in the cohort and lifetime resource count, but not in window throughput.

| Stress result | N1 software | N2 hardware |
| --- | --- | --- |
| Opportunities, warmup / measured / total | 120 / 600 / 720 | 120 / 600 / 720 |
| Dispatched, warmup / measured / total | 116 / 577 / 693 | 116 / 578 / 694 |
| Validated frames, warmup / measured / total | 116 / 577 / 693 | 116 / 577 / 693 |
| Missed opportunities, warmup / measured / total | 4 / 23 / 27 | 4 / 22 / 26 |
| Obsolete, warmup / measured / total | 0 / 0 / 0 | 0 / 1 / 1 |
| Late settled result | 0 | 1 measured-cohort obsolete result after cutoff |
| Busy / skipped opportunities | 0 / 0 | 0 / 0 |
| Admission failures / rejected / settled failures / unresolved | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |
| Scheduling attempts marked late versus ideal slot | 693 | 694 |
| Window valid / obsolete | 577 / 0 | 577 / 0 |
| Window valid / settled-frame rate | 57.7 / 57.7 Hz | 57.7 / 57.7 Hz |
| Actual measured pacing elapsed | 10.001087560 s | 10.000942658 s |
| Full pacing elapsed | 12.001087560 s | 12.000942658 s |

The absence of worker-busy opportunities supports a **scheduler-limited interpretation for these observed runs**, alongside the recorded roughly 17.3 ms cadence. It does not establish universal performance characteristics or authorize scheduler changes. Scheduling lateness is distinct from a freshness expiry or the one hardware late settled obsolete result. No growing backlog was observed.

### Owner-observed latency quantiles

All values below are milliseconds, nearest-rank median/p95/p99. Exact warmup and measured cohorts remain separate. Stress IPC/end-to-end distributions describe accepted measured-cohort frames; cadence/lateness describe measured scheduling attempts. IPC runs from owner-observed complete dispatch to terminal observation; end-to-end runs from admission to that observation. They do not include compositor presentation or scanout.

| Workload | Distribution | Median ms | p95 ms | p99 ms |
| --- | --- | --- | --- | --- |
| N1-A-R2 | warmup IPC | 0.623912 | 0.992361 | 10.567140 |
| N1-A-R2 | warmup end to end | 0.628859 | 1.000016 | 10.573893 |
| N1-A-R2 | measured IPC | 0.486287 | 0.692865 | 0.793693 |
| N1-A-R2 | measured end to end | 0.488811 | 0.699808 | 0.799234 |
| N1-D | cadence | 17.335015 | 17.734915 | 17.772299 |
| N1-D | lateness | 8.724925 | 15.820920 | 16.414986 |
| N1-D | IPC | 1.505750 | 1.836922 | 2.020888 |
| N1-D | end to end | 1.524425 | 1.855995 | 2.041479 |
| N2-A | warmup IPC | 0.435795 | 0.592867 | 1.829878 |
| N2-A | warmup end to end | 0.442468 | 0.595039 | 1.839307 |
| N2-A | measured IPC | 0.265104 | 0.573442 | 0.599120 |
| N2-A | measured end to end | 0.270704 | 0.581687 | 0.607870 |
| N2-D | cadence | 17.357820 | 17.731425 | 17.795750 |
| N2-D | lateness | 8.467569 | 16.110420 | 16.566419 |
| N2-D | IPC | 1.610588 | 1.915537 | 2.142750 |
| N2-D | end to end | 1.629352 | 1.941881 | 2.168317 |

### Independently observed initialization and shutdown

The harness's owner observations are independent of worker-internal timing. Owner initialization spans spawn origin to the observed initialization transition (expected rejection in N3); owner shutdown spans first shutdown request through bounded finish/reaping observation. Worker setup excludes portions of owner process/IPC observation. Values are milliseconds.

| Workload | Owner initialization / rejection ms | Owner shutdown ms | Worker initialization / rejection ms | Device ms | Pipeline ms |
| --- | --- | --- | --- | --- | --- |
| N1-A-R2 | 50.748049 | 6.739448 | 48.035854 | 44.614853 | 3.384853 |
| N1-B | 63.736846 | 8.520053 | 60.653692 | 56.190511 | 4.401484 |
| N1-C | 61.204867 | 7.847416 | 58.181084 | 54.500468 | 3.629531 |
| N1-D | 53.755873 | 8.830654 | 51.539097 | 48.478758 | 3.017553 |
| N2-A | 531.335350 | 75.536233 | 528.489054 | 340.814135 | 165.191110 |
| N2-B | 304.304407 | 65.526758 | 301.456063 | 258.717938 | 34.953779 |
| N2-C | 271.314293 | 66.375202 | 267.636870 | 208.933493 | 47.568205 |
| N2-D | 265.538985 | 133.357873 | 262.792283 | 219.660288 | 35.916542 |
| N3 | 4.567616 | 0.432596 | 2.119963 | Unavailable | Unavailable |

These are individual observed lifetimes, not a statistically complete cold-start characterization. Native cold-start latency remains **incompletely characterized**; in particular the original 127.485213 ms wait remains relevant historical evidence.

### Worker lifetime timing aggregates

Values below are exact nanosecond sums from complete worker telemetry, with the actual timed-operation denominator. These are **host wall-time measurements across the entire worker lifetime**, including warmup and late settlement where applicable. They are **not direct GPU timestamps**, measured-cohort quantiles or display timings. Zero-operation rows contain zero stage sums.

| Workload | Timed operations | Encode ns | Submit ns | Wait ns | Map ns | Repack ns |
| --- | --- | --- | --- | --- | --- | --- |
| N1-A-R2 | 125 | 3598051 | 1891957 | 59514619 | 698918 | 2009504 |
| N1-B | 1 | 273282 | 46207 | 15738579 | 24235 | 121044 |
| N1-C | 0 | 0 | 0 | 0 | 0 | 0 |
| N1-D | 693 | 73796081 | 36042828 | 728324931 | 14154852 | 18148573 |
| N2-A | 125 | 9649990 | 4399889 | 8468562 | 804805 | 2361812 |
| N2-B | 1 | 230684 | 95705 | 257891 | 12471 | 23524 |
| N2-C | 0 | 0 | 0 | 0 | 0 | 0 |
| N2-D | 694 | 177034883 | 78899101 | 545653741 | 14596297 | 22001211 |
| N3 | 0 | 0 | 0 | 0 | 0 | 0 |

## 6. Recorded Cargo verification and preservation

Codex's frozen R2 verification records **29/29 established matrix commands PASS**, including formatting, builds, all-target Clippy with warnings denied, unfiltered tests and test-identity listings for desktop-probe, the worker workspace in default/native-vulkan configurations, frozen B1 in both configurations, independent protocol/supervisor builds and the four required desktop-probe checks. Raw command/exit records, test output and multiset-preservation comparisons are retained in the reviewed source archive.

| Suite | Final passing identities | Previously reviewed identities lost |
| --- | ---: | ---: |
| Desktop-probe | 867 | 0 |
| Worker default | 228 | 0 |
| Worker native-vulkan | 230 | 0 |
| Frozen B1 default | 16 | 0 |
| Frozen B1 native-vulkan | 16 | 0 |

All previously accepted test identities are preserved. The final recorded runs have zero failed, ignored or filtered tests. Actual tools were Rust/Cargo **1.99.0**; **Rust 1.88 compatibility: UNVERIFIED**. Native-feature automated tests use GPU-free fixtures/injection, separately from owner-native execution. Codex also recorded six explicit CPU-only synthetic workloads; those are not native measurements and do not contribute to the R2 native totals.

ChatGPT independently inspected source, patches and archived evidence. **ChatGPT did not independently rerun Cargo.** This documentation-only mission performs content, identity and patch checks; it adds no Cargo build/test, native execution, dependency change or GPU access.

## 7. Complete C1–C10 gate register

| Gate | Result and exact evidence boundary |
| --- | --- |
| C1 — Actual adapter/native initialization | PASS for eight initialized N1/N2 sessions with the identities in section 3. N3 is separately an expected initialization rejection. |
| C2 — Software exact parity | PASS: N1-A-R2, all 125/125 exact frames and five phases, including warmup. Original failed R1 N1-A remains failed. |
| C3 — Hardware exact parity | PASS: N2-A, all 125/125 exact frames and five phases, including warmup. |
| C4 — Persistence/reuse | PASS: exact and stress operation counts reconcile with one allocation per initialized session, reuse counts in section 4, peak at most one and zero retirements. |
| C5 — Unavailable ICD | PASS for expected rejection: N3 retains BackendFault/diagnostic, no job and zero native operations/resources; controlled shutdown is separately confirmed. |
| C6 — Cancellation/late settlement | PASS: N1-B and N2-B prove post-dispatch obsolete settlement; N1-C proves local zero-byte withdrawal; N2-D additionally settles one obsolete late frame. N2-C rejects unsupported intent before GPU submission. |
| C7 — Sustained request stress/bounded backlog | Correctness and bounded-work evidence PASS: both 720-opportunity runs reconcile without growing backlog. **Strict 60 Hz readiness NOT ACHIEVED:** both windows are 57.7 Hz. |
| C8 — IPC/end-to-end measurements | Measurement evidence PASS: distinct exact warmup/measured quantiles, measured stress quantiles, actual pacing durations, independent initialization/shutdown observations and qualified lifetime timing sums. No production performance acceptance. |
| C9 — Repeated shutdown/reaping | PASS: nine controlled releases, Stopped acknowledgments, zero process exit codes, successful reapings and confirmed shutdowns, including expected N3 rejection. Zero unresolved D1/supervisor jobs. |
| C10 — Safe failure containment/truthful uncertainty | PASS within the combined evidence scope: native cancellation, rejection, late settlement and N3 containment plus preserved deterministic native-error/loss/truncation/abandonment/release tests. Physical GPU-loss and dangerous driver-hang scenarios were not deliberately executed. |

These gates document reviewed evidence and its limits. Independent ChatGPT documentation review passed and the owner explicitly approved technical acceptance on 2026-11-24; Git publication remains pending.

## 8. Native archive identity register

Private archives and raw logs are retained by the owner and are not published here. The public archive basenames and SHA-256 values identify the reviewed evidence without exposing private machine paths.

| Evidence | Archive basename | SHA-256 |
| --- | --- | --- |
| N1-A-R2 | `run-r2.gjGZ6q4b.tar.gz` | `5742c1e7723e3a6e14150d2029deebfe9a6703ebca417f513f4db843b859b193` |
| N1-B/C | `cancel-r2.OoE6V4tK.tar.gz` | `7d5676f81e82bae38298e18d69291f7e49ce78ad2ff1f9f9b354df596b7417b5` |
| N1-D | `stress-r2.ME508QZY.tar.gz` | `69ca646c2e33333facb4c8c2967384387356550cc03c6967c5c76c30dfb1d9ee` |
| N2-A | `exact-r2.cLYLOvwN.tar.gz` | `3d5631ad9dd0752a1c3e4ec09b890e95b383471fc8f924d9cf4cf9acc694fe55` |
| N2-B/C | `cancel-unsupported-r2.n80ZqEQG.tar.gz` | `62bb145bc2c9a0b85f2a35268088835c6618908926c4874ef9c18ed72774b328` |
| N2-D | `stress-r2.bHoBeRmg.tar.gz` | `4861aeba305e282475908b6c0a2eb71981b04310d67b04e8a17f5e2ee2f7a685` |
| N3 | `init-failure-r2.6sfuzum7.tar.gz` | `8855c72ae38ac30bf8be24f05b22baa20e9ee7130256ca97de01c80acdd821c7` |

Reproduction uses the frozen release identities and reviewed harness policies, explicit software/hardware adapter selection, explicit driver manifests and complete telemetry. Exact uses 25 warmup + 100 measured frames with all five phases; stress uses 120 + 600 opportunities and the fixed window. Cancellation/unsupported/init-failure require their recorded transmission and failure predicates as well as lifecycle/resource reconciliation. Historical instructions marked native execution pending describe their earlier source-review checkpoints; this record documents the later owner executions. Further execution requires separate authorization.

## 9. Architectural limits and remaining decisions

- **Rust 1.88 compatibility: UNVERIFIED.**
- **Strict 60 Hz readiness: NOT ACHIEVED.** No production 60 FPS claim.
- Native cold-start latency is incompletely characterized.
- Physical GPU-loss and dangerous driver-hang scenarios were not deliberately executed. Failure-containment evidence combines native observations and deterministic fault injection; injected cases are not physical-device observations.
- Production pixel transport remains undecided. Headless owned-byte readback over the experimental IPC path does not select production transport.
- X11 publication and SHAPE integration remain deferred. There is no production compositor, scanout or presentation acceptance from this mission. Existing A00 and experimental CPU/X11 acceptance retain their original separate scopes.
- No private Vanilla or Spine assets are included in this headless candidate.
- General repository maintenance remains deferred.
- B1 progress-site publication confirmation remains deferred; this mission changes no progress site.

**Before D3, separately resolve the publication policy for an ordinary late backend fault following a partially transmitted but genuinely settled frame.** Native fault observation and IPC transmission are not atomic; already transmitted bytes cannot be retracted. This headless evidence does not choose the future publication policy. Process isolation cannot guarantee recovery from system-wide kernel or graphics-driver hangs, nor impose hard wall-clock guarantees on synchronous driver operations/destruction.

The previously accepted [A00 foundation](DesktopRoomie-A00-M03.4-integrated-acceptance.md), [C1-A](A01-C1-A-ACCEPTANCE.md), [presentation integration](A01-C1-B-B4-ACCEPTANCE.md), [experimental CPU animation](A01-C1-C-A1-A2-ACCEPTANCE.md), [standalone Vulkan B1](A01-C1-C-B1-ACCEPTANCE.md) and [D1](A01-C1-C-B2-D1-ACCEPTANCE.md) retain their separate identities and approval boundaries. This documentation reconciles the stale D2-P future pointer with implemented headless work without advancing to managed X11 integration.

## 10. Approval and stopping point

**TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-24**

Independent ChatGPT D2-C-F documentation review passed. The owner explicitly granted D2-B/D2-C technical acceptance on 2026-11-24 and retains all Git publication authority. Publication remains pending. No staging, commit, push, merge, branch/configuration change or D3 work occurred in these documentation missions. Branch `a01` and published HEAD remain unchanged; the accepted implementation and four authorized documentation paths remain local. No commit was made.

**D2-C-F2 — READY FOR FINAL INDEPENDENT REVIEW AND OWNER PUBLICATION**
