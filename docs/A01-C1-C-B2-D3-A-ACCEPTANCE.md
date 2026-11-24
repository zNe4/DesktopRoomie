# A01-C1-C-B2-D3-A — Asynchronous presentation seam acceptance

## TECHNICALLY ACCEPTED — OWNER APPROVED — 2026-11-24

Independent ChatGPT source and patch review of **D3-A-I1: PASS**, with no blocking findings and **no required R1 correction**. The owner explicitly granted technical acceptance on **2026-11-24** after that review. This accepts the production-used presentation seam and its deterministic preservation evidence. Managed Vulkan integration is not implemented.

**D3-A-F2 — READY FOR FINAL INDEPENDENT REVIEW AND OWNER PUBLICATION.** Independent ChatGPT D3-A-F documentation review **PASS**, approved by the owner on 2026-11-24. Final independent F2 verification and owner-controlled Git publication remain pending. This closeout authorizes no D3-B work.

## 1. Frozen implementation identity and provenance

The accepted implementation is the six-path D3-A-I1 candidate over published `a01` baseline [`1313a9251427875d8c7814b2c64131da75d5a37b`](https://github.com/zNe4/DesktopRoomie/commit/1313a9251427875d8c7814b2c64131da75d5a37b). At documentation preflight, HEAD, local `a01`, tracking `origin/a01` and live remote `a01` all matched that baseline. The accepted implementation remains uncommitted; a later documentation revision does not replace its execution evidence.

| Retained artifact | SHA-256 |
| --- | --- |
| Accepted cumulative implementation patch, `implementation.patch` | `b236578da3139992c7cd0d71342d6047ee63a9e62cbf900e8a4b7b48300c6b55` |
| Independently reviewed archive, `A01-C1-C-B2-D3-A-I1-review.tar.gz` | `186a6a6e4a122a44218304dd85e6fb241d4d442c0d75b126b95613d6f5542d5b` |
| Candidate 137-file manifest, `candidate-source.sha256` | `4c486ca6d44b35d2f6e9b4989f7dba9d544ad7b400bc42ae3bc8d8d4e21110fb` |
| Baseline 136-file manifest, `baseline/source.sha256` | `0bfa84128d761b022e984a29c12d6f9703ee8dd65768efb86128ce033a7b130b` |
| Unchanged 131-file manifest, `frozen-source.sha256` | `34b53e5110e4c2ac5a0d89afab81291bc0ccdbc335824f91b50b5550cf12c4b4` |
| Original archive content manifest, `PACKAGE-CONTENTS.sha256` | `03e55cce92e2b2089c40a06632093bb7e589deb3b674265177c7f4dd8bd9949e` |
| Original baseline snapshot, `baseline-source.tar` | `618384713fe7bd42f2f759cc0f2745009d029b28ff551712acf5208e7c2ff197` |
| Reviewed mapping, `REQUIREMENTS-TO-TESTS.md` | `70901112fb6f112a27a3091b7aac2181fe845a3cc9f9e68f397f969742157f70` |
| Owner-provided ratified D3-P specification | `71f81c14f359bd87efc3a2da2ba14871288f78239571d30009c64e532560f9ba` |

The original review directory `A01-C1-C-B2-D3-A-I1` and archive are retained unchanged as private evidence. The archive contains the candidate source, original patch, baseline snapshot, raw command records, test inventories, replay evidence and requirement mapping. Artifact basenames and hashes identify that evidence without publishing private machine paths or raw logs.

Exactly six implementation paths were authorized, comprising five modified existing experiment files and one new test file. All paths below are relative to `experiments/desktop-probe/`:

| Path | Accepted change |
| --- | --- |
| `src/body_presentation.rs` | Modified: authoritative request construction, preparation and successful-publication timing separation. |
| `src/lib.rs` | Modified: CPU preparation and shared `ProbeRuntime::publish_prepared_with` coordination. |
| `src/x11/render.rs` | Modified: final admission callback and explicit skipped/committed outcome. |
| `src/probe_tests/body_presentation_tests.rs` | Modified: one clock-order and preparation-error regression. |
| `src/probe_tests/animated_presentation_tests.rs` | Modified: registration of the new test-only child module. |
| `src/probe_tests/asynchronous_presentation_tests.rs` | New: 19 deterministic asynchronous seam tests. |

The D1 coordinator, entire worker workspace, frozen B1 experiment, manifests, lockfiles and other existing experiment files retain their accepted bytes. Of 136 baseline experiment files, 131 are unchanged. There are zero unauthorized experiment modifications. The [ratified D3-P specification](plans/A01-C1-C-B2-D3-P-SPECIFICATION.md) remains an owner-provided **untracked** local file, preserved byte-for-byte. On 2026-11-24 the owner approved F01 Option A: its exact bytes are included as a new-file addition in the final cumulative publication patch and authorized for eventual owner-controlled Git publication. It has not been staged, tracked or published. Its historical authorization labels are not rewritten by this record.

## 2. Implemented presentation architecture

The seam is used by synchronous CPU production presentation and by deterministic asynchronous completions through the accepted D1 coordinator:

1. `BodyPresentation::frame_request` constructs the authoritative immutable request from the current `FrameGeometry`, intent and `FrameTiming`. It preserves initial/redraw/scheduled checks, fixed extent and request-clock observation without reserving a committed sequence.
2. `BodyPresentation::prepare_with` passes that request to the persistent CPU producer and returns the original request with its candidate. Preparation errors stop timing through the existing error path.
3. `ProbeRuntime::publish_prepared_with` accepts the original request and prepared candidate, coordinating checked publication, successful timing, input enabling and failure fencing. Synchronous CPU preparation invokes this shared seam.
4. `FrameGeometry::publish_with_admission` validates the candidate and native limits, then invokes final admission **after validation but before publication-state mutation and native submission**. CPU admission is unconditional and adds no clock observation. Deterministic asynchronous tests use the callback to revalidate freshness after validation, including expiry at equality.
5. `FrameTiming::publish_with` observes completion only after successful checked commitment, updates active elapsed time and rearms the deadline from successful presentation. Runtime installs the full input enabling sequence only after timing succeeds.

The asynchronous fixtures deliver CPU-produced synthetic completions through `GpuHandoff::complete`. D1 retains session, eligibility epoch, job serial, original request, physical outstanding work and one-shot ready transfer. The fixture applies current eligibility/fault observations before taking ready, then consumes and publishes within the same owner turn. It retains the actual admitted deadline through consumption. There is no competing job ledger, retained consumed-candidate queue or independent animation clock. These tests exercise the production host seam; they do not connect a supervisor, worker socket, pidfd or Vulkan device to the managed event loop.

| Outcome | Publication and timing meaning |
| --- | --- |
| Successfully committed publication | Checked native work and frame Commit succeeded. Completion timing then runs; successful timing rearms the deadline and permits input enabling. GPU settlement remains a separate fact. |
| `PreparedPublication::Skipped` | Final admission declined before publication-state mutation or native submission. No committed sequence is reserved, no completion clock is read, no successful timing or rearming is fabricated, and a trustworthy previous frame remains authoritative. Eligible time can continue accruing. |
| Publication failure | An error stops timing. Native submission uncertainty enters the established fail-closed host path without retry. A post-commit timing error preserves historical commitment but revokes usable authority and fences input. A pre-submission validation error is not fabricated into a native commit. |

Existing CPU request/completion clock observations, successful deadline rearming and error-driven stopping are preserved. The seam is **not a managed Vulkan backend**. [Implemented ownership and event flow](ARCHITECTURE.md), [D3-P contract](plans/A01-C1-C-B2-D3-P-SPECIFICATION.md).

## 3. Safety and preservation

Accepted source and deterministic tests preserve authoritative immutable frame requests; independent session, epoch, job serial and frame identities; fixed extent; pixel-derived canonical hit-region validation; physical settlement and unresolved-debt accounting; gesture/lifecycle invalidation; freshness equality rejection; and freshness revalidation at final publication admission. An unpublished frame sequence may be reused only under the existing geometry contract, with a distinct job identity and the original correlated request retained.

The existing native transaction is preserved. Equal canonical coverage uses checked upload, blit and flush without unnecessary SHAPE work. Changed coverage validates first, admits and records the attempt, blocks input, uploads, installs checked empty input SHAPE, settles pointer release obligations, performs checked blit/flush, installs exact final input SHAPE and commits frame geometry. Timing succeeds before the full enabling-sequence freshness boundary restores runtime input readiness. Fully transparent frames use exact empty coverage.

Expose repaints only the trustworthy committed frame while preparation is pending. Ordinary prospective invalidation does not itself invalidate that previous frame. Uncertain native publication retains fail-closed handling, original-error priority and no automatic retry. Post-commit timing failure explicitly fences input and revokes usable authority. Existing A00 and CPU behavior retain their accepted contracts and evidence boundaries.

**X11 pixels, input SHAPE and compositor presentation are not atomic.** Intermediate empty input can allow click-through, and compositor-visible pixels may represent another point in the update sequence. Deterministic ordering and checked X11 operations do not prove simultaneous visible/input coverage, compositor acknowledgment or physical scanout.

## 4. Accepted automated verification

Cargo execution belongs to **Codex's recorded D3-A-I1 verification**. This documentation closeout does not rerun Cargo or replace accepted execution evidence.

| Suite | Baseline | Accepted | Lost | Added |
| --- | ---: | ---: | ---: | ---: |
| Desktop-probe | 867 | 887 | 0 | 20 |
| Worker default | 228 | 228 | 0 | 0 |
| Worker native-vulkan | 230 | 230 | 0 | 0 |
| Frozen B1 default | 16 | 16 | 0 | 0 |
| Frozen B1 native-vulkan | 16 | 16 | 0 | 0 |

The frozen `final-verification/` records **29/29 working-tree matrix commands PASS**. The independent scratch replay's `scratch/` records **29/29 PASS**, for **58/58 combined recorded command successes**. The matrix includes formatting, builds, all-target Clippy with warnings denied, unfiltered tests and identity listings across the five configurations, independent protocol/supervisor builds and the required desktop checks. Rust/Cargo 1.99.0 was used; Rust 1.88 compatibility remains unverified. Feature-enabled automated tests use GPU-free fixtures and injections.

Target-qualified test-identity multiset comparisons establish zero lost accepted identities. Candidate and scratch multisets match exactly for all five inventories. Final recorded verification reports **zero failed, ignored or filtered tests**. Twenty new deterministic identities consist of one CPU clock/error regression and 19 asynchronous seam tests. **All A01–A25 required behavioral categories are satisfied; 25 categories do not mean 25 new test functions.** The exact reviewed requirement-to-test mapping is reproduced in section 8.

The original `verification/` evidence preserves intermediate failed development checks: a test-only Rectangle comparison compilation error and a run with three incorrect test expectations, corrected before final verification. It also preserves an earlier passing matrix before the final skipped-admission refinement. Those logs remain historical development evidence; only `final-verification/` and its matching scratch replay identify the frozen accepted candidate.

## 5. Independent ChatGPT review and owner decision

As reported by the owner for this assignment, independent ChatGPT source and patch review verified:

- **326/326** listed package content hashes, **136/136** baseline experiment-file hashes and **137/137** candidate experiment-file hashes.
- **131** unchanged baseline experiment files, **five** modified existing files, **one** new experiment file and **zero** unauthorized experiment modifications.
- Strict independent patch applicability and replay, followed by exact **137/137** candidate-file byte comparison.
- Independent comparison of all five test-identity inventories, source, tests, patches and recorded evidence.
- **No blocking findings and no required source corrections.** No R1 correction is required.

**ChatGPT did not independently rerun Cargo.** Its independent inspection, identity verification and patch replay are distinct from Codex's recorded automated execution. The owner granted technical acceptance on 2026-11-24 after that review PASS. Independent ChatGPT D3-A-F documentation review also passed and was owner-approved on 2026-11-24. Final independent F2 verification and owner-controlled Git publication remain pending.

### Independent D3-A-F documentation review and F01 resolution

| Reviewed D3-A-F artifact | SHA-256 |
| --- | --- |
| Review archive, `A01-C1-C-B2-D3-A-F-review.tar.gz` | `b906b46649667a7b8cc9f9d355d3a6abe2dd285eaaf09b38e16bf361e6fc6ef0` |
| Incremental documentation patch | `446d4b17ff079355a68f811239c1b49a03a712f46c5e2120c49517e76bf7d9e2` |
| Cumulative implementation/documentation patch | `b4c43ab7701d82150e1a7f7c9eef7f926ae1d9a851bb05255634d858b48cd582` |

The original D3-A-F review directory and archive remain unchanged alongside D3-A-I1 evidence.

The owner reported independent ChatGPT D3-A-F documentation review **PASS** and approved it on **2026-11-24**. That review verified:

- **61/61** closeout archive content hashes, **326/326** original implementation archive content hashes and **137/137** frozen candidate experiment files.
- **Four** authorized documentation paths and **zero unauthorized changed paths** in the reviewed manifest comparison.
- Preserved owner specification identity, protected evidence and index checksum records.
- Independent incremental documentation patch replay: **190/190 exact file comparisons**.
- Independent cumulative patch replay: **190/190 exact file comparisons**.
- Accepted implementation patch reproduced byte-for-byte; **zero blocking implementation findings**, **zero technical documentation findings**, and **one minor publication finding, F01**.

ChatGPT did not execute Cargo or native workloads and did not independently repeat Codex's complete Markdown reference check. These completed source and documentation reviews are distinct from the pending final independent F2 verification.

**F01 — resolved administratively by owner-approved Option A.** Six valid relative Markdown hyperlinks across the four closeout documents pointed to the ratified D3-P specification, which was untracked and excluded from the reviewed D3-A-F publication patch. F01 was a documentation/publication matter, not an implementation defect. The owner authorized inclusion of the exact existing specification on 2026-11-24. Its SHA-256 remains `71f81c14f359bd87efc3a2da2ba14871288f78239571d30009c64e532560f9ba`; the final cumulative patch includes its original bytes as a new file and preserves all six hyperlinks. Inclusion is authorized; actual publication remains pending and exclusively owner-controlled. The specification remains untracked and unstaged locally, with its historical labels unchanged.

The reviewed D3-A-F replay sets each contained 190 files and **excluded** the local specification. F2 derives its expected publication set from the pinned baseline tree plus the accepted new test, acceptance record and specification: **191 files**. Replay A starts from the reviewed D3-A-F candidate with the preserved local specification; Replay B starts from the pinned published baseline and obtains the specification solely through the cumulative patch. F2 requires exact file-set and byte equality, all 137 experiment identities, and exact accepted implementation-patch identity. The incremental F2 patch changes only these four documents; the final cumulative publication patch has exactly eleven paths: six frozen implementation paths, four finalized documents and the unchanged specification. No live index is used for patch construction.

## 6. Forward D3-B integration obligations

The independent review identified three non-blocking integration obligations. They are forward requirements, not unresolved D3-A source defects. D3-B requires separate authorization.

| ID | Required future integration behavior |
| --- | --- |
| **N01 — Authoritative deadline retention** | Retain the deadline associated with the actual admitted job. Do not reconstruct, extend or replace it when a result becomes ready. |
| **N02 — Production owner eligibility** | Supply actual owner-observed predicates at final admission: supervisor drainage, observed backend-fault precedence, buffered X11 event ordering, current lifecycle/interaction eligibility, current target authority and freshness at admission. Deterministic fake completions do not establish actual IPC integration. |
| **N03 — Request opportunity consumption** | Prevent the integrated event loop from spinning on an already-due animation deadline while physical GPU work remains outstanding. Preserve bounded work, no catch-up queue and separate request-opportunity versus successful-publication bookkeeping. |

## 7. Limits, exclusions and unchanged native evidence

| Classification | Status |
| --- | --- |
| Managed Vulkan integration | **NOT IMPLEMENTED** |
| Actual managed worker/supervisor integration | **NOT IMPLEMENTED**; forward D3-B obligation |
| Managed Vulkan publication | **NOT IMPLEMENTED** |
| D3-A owner-native execution | **NOT PERFORMED** |
| Automatic CPU fallback | **NOT IMPLEMENTED** |
| Production transport | **UNSELECTED** |
| Production empty-SHAPE acceptance | **PENDING** |
| Strict 60 Hz readiness | **NOT ACHIEVED** |
| Rust 1.88 compatibility | **UNVERIFIED** |
| Repository maintenance | **DEFERRED** |
| B1 progress-site publication confirmation | **DEFERRED** |
| Private Vanilla/Spine assets | **OUT OF SCOPE** |

Previously accepted [D2-C native evidence](A01-C1-C-B2-D2-C-ACCEPTANCE.md) remains unchanged and separate from D3-A deterministic seam evidence: **nine successful workloads** (eight initialized Vulkan sessions and one expected initialization rejection), **250/250 dedicated exact-parity frames**, **1,639 confirmed Vulkan completions**, **zero unresolved jobs**, **zero staging retirements**, and **both 57.7 Hz stress observations**. These are headless Vulkan measurements, not managed X11 performance. The original unsuccessful R1 native observation remains historical failure evidence. D3-A adds no owner-native, real Vulkan or X11 acceptance execution.

D3-A-F2 changes only this record, [STATUS](STATUS.md), [ARCHITECTURE](ARCHITECTURE.md) and [INDEX](INDEX.md). Its private review package retains before/after manifests, frozen implementation and specification verification, separate incremental documentation and cumulative patches, patch hashes, separate scratch replay logs and byte comparisons, local Markdown reference checks, whitespace results and complete Git state. The live index is not used to construct patches. Original review/native archives and the progress site remain untouched. No staging, commit, push, merge, branch creation or D3-B execution is authorized.

## 8. Reviewed A01–A25 requirement-to-test mapping

The table below reproduces the actual `REQUIREMENTS-TO-TESTS.md` mapping identified in section 1. Test names are exact Rust identities; the unfiltered desktop inventory additionally prefixes the Cargo target `desktop_probe::`. Existing tests and the 20 new identities jointly satisfy these categories. Every listed test passed in the frozen recorded working-tree verification. No native execution or actual supervisor integration is represented.

| ID | Behavior and assertions | Exact test identities |
|---|---|---|
| A01 | Initial CPU frame unchanged | `body_presentation::tests::initial_request_has_exact_identity_extent_intent_and_zero_timing`<br>`body_presentation::tests::split_seam_preserves_clock_order_and_skips_completion_clock_on_preparation_error` |
| A02 | Scheduled CPU frame, delta and rearming unchanged | `body_presentation::tests::recurring_deadline_caps_delta_and_rearms_after_completion_without_catch_up`<br>`x11::shape::animated_presentation_tests::scheduled_cutovers_follow_exact_checked_order_and_replay_only_the_commit` |
| A03 | Palette redraw unchanged | `body_presentation::tests::persistent_procedural_producer_publishes_lavender_coral_lavender` |
| A04 | Immediate fake result uses the actual shared publication seam | `x11::shape::animated_presentation_tests::asynchronous_tests::immediate_fake_completion_uses_shared_image_only_publication_and_timing` |
| A05 | Delayed result cannot publish prematurely | `x11::shape::animated_presentation_tests::asynchronous_tests::delayed_completion_and_expose_use_only_committed_pixels_until_owner_publication` |
| A06 | Outstanding physical work blocks admission | `x11::shape::animated_presentation_tests::asynchronous_tests::repeated_due_opportunities_cannot_admit_backlog_or_duplicate_jobs`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::unresolved_cancelled_work_retains_debt_until_genuine_obsolete_settlement` |
| A07 | Gesture revokes prospective eligibility while retaining committed authority | `x11::shape::animated_presentation_tests::asynchronous_tests::gesture_interruption_obsoletes_prospective_frame_without_settling_pending_work`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::current_control_or_body_interruption_prevents_ready_consumption_and_native_mutation` |
| A08 | Pause/resume rejects previous epoch | `x11::shape::animated_presentation_tests::asynchronous_tests::pause_resume_reused_sequence_keeps_original_request_and_rejects_old_job` |
| A09 | Expiry before completion, consumption or admission prevents native mutation | `x11::shape::animated_presentation_tests::asynchronous_tests::expired_or_equal_deadline_completion_settles_but_never_mutates_or_rearms`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::ready_candidate_expiring_before_consumption_is_discarded_without_publication`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::expiry_during_host_validation_is_checked_before_submission_including_equality` |
| A10 | Deadline equality is expiry at completion and final admission | `x11::shape::animated_presentation_tests::asynchronous_tests::expired_or_equal_deadline_completion_settles_but_never_mutates_or_rearms`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::expiry_during_host_validation_is_checked_before_submission_including_equality` |
| A11 | Reused sequence does not identify a job or replace the original request | `x11::shape::animated_presentation_tests::asynchronous_tests::pause_resume_reused_sequence_keeps_original_request_and_rejects_old_job`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::wrong_session_epoch_serial_and_frame_cannot_discharge_or_publish_the_current_job` |
| A12 | Previously observed backend fault discards ready | `x11::shape::animated_presentation_tests::asynchronous_tests::fault_observed_before_consumption_discards_ready_but_preserves_settlement_and_commit` |
| A13 | Later fault preserves truthful historical commitment | `x11::shape::animated_presentation_tests::asynchronous_tests::fault_observed_after_commit_closes_future_admission_without_falsifying_history` |
| A14 | Unresolved physical debt survives cancellation and resumed admission attempts | `x11::shape::animated_presentation_tests::asynchronous_tests::unresolved_cancelled_work_retains_debt_until_genuine_obsolete_settlement` |
| A15 | Malformed pixels, coverage, extent and identities cannot submit | `x11::shape::animated_presentation_tests::asynchronous_tests::malformed_ready_payload_coverage_extent_and_identity_never_reach_native_submission`<br>`x11::shape::animated_presentation_tests::animated_preflight_rejects_identity_extent_payload_and_wire_size_without_requests` |
| A16 | Equal coverage uploads/blits/flushes without SHAPE | `x11::shape::animated_presentation_tests::asynchronous_tests::immediate_fake_completion_uses_shared_image_only_publication_and_timing`<br>`x11::shape::animated_presentation_tests::bounds_and_anchor_updates_with_identical_coverage_do_not_send_shape` |
| A17 | Changed coverage follows Block/Upload/Empty/Release/Blit/Flush/FinalShape/Commit/timing/enable | `x11::shape::animated_presentation_tests::asynchronous_tests::delayed_completion_and_expose_use_only_committed_pixels_until_owner_publication`<br>`x11::shape::animated_presentation_tests::scheduled_cutovers_follow_exact_checked_order_and_replay_only_the_commit` |
| A18 | Fully transparent frame installs exact empty input region | `x11::shape::animated_presentation_tests::asynchronous_tests::fully_transparent_fake_candidate_commits_exact_empty_input_after_checked_cutover` |
| A19 | Upload and blit failures fence, preserve original error, do not retry | `x11::shape::animated_presentation_tests::asynchronous_tests::async_native_failure_fences_without_retry_and_keeps_gpu_settlement_independent`<br>`body_presentation::tests::partial_upload_blit_or_flush_failure_fences_once_without_retry` |
| A20 | Final SHAPE failure retains input fencing | `x11::shape::animated_presentation_tests::asynchronous_tests::async_native_failure_fences_without_retry_and_keeps_gpu_settlement_independent`<br>`x11::shape::animated_presentation_tests::every_checked_cutover_failure_fences_and_preserves_original_error_through_cleanup` |
| A21 | Post-commit timing failure revokes usable authority | `x11::shape::animated_presentation_tests::asynchronous_tests::async_post_commit_timing_error_revokes_usable_authority_and_fences_input`<br>`x11::shape::animated_presentation_tests::post_commit_clock_error_revokes_both_image_only_and_changed_authority_before_fencing` |
| A22 | Invalidated target generation cannot publish | `x11::shape::animated_presentation_tests::asynchronous_tests::invalidated_host_generation_rejects_ready_before_native_mutation`<br>`x11::render::tests::invalidated_provisional_or_committed_target_cannot_issue_a_frame` |
| A23 | Expose during pending preparation repaints the committed frame | `x11::shape::animated_presentation_tests::asynchronous_tests::delayed_completion_and_expose_use_only_committed_pixels_until_owner_publication`<br>`body_presentation::tests::expose_and_post_map_repaint_the_commit_without_preparing_a_frame` |
| A24 | Rejected, failed, obsolete and validation-expired candidates do not advance successful timing | `x11::shape::animated_presentation_tests::asynchronous_tests::rejected_or_failed_fake_preparation_never_reports_success_or_mutates_the_host`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::expired_or_equal_deadline_completion_settles_but_never_mutates_or_rearms`<br>`x11::shape::animated_presentation_tests::asynchronous_tests::expiry_during_host_validation_is_checked_before_submission_including_equality` |
| A25 | Repeated opportunities remain bounded and one-shot | `x11::shape::animated_presentation_tests::asynchronous_tests::repeated_due_opportunities_cannot_admit_backlog_or_duplicate_jobs` |
