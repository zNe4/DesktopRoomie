# DesktopRoomie — D00-M01: Documentation and Context Hygiene

**Decision:** **ACCEPTED** on 2026-10-08 after independent documentation/source-scope review and owner approval. Original implementation contract and acceptance criteria are preserved below.  
**Baseline:** A00-M03.1 accepted 2026-10-08 on target Openbox/X11/Picom, documented at `experiments/desktop-probe/ACCEPTANCE-M03.1.md`.  
**Sequencing:** D00-M01 precedes A00-M03.2 design and implementation.  
**Scope:** documentation and contributor instructions only; do not change the desktop-probe code, Cargo files, scripts, CI, Git behavior, or host configuration.

## 1. Purpose and success condition

The repository must be sufficient to recover current project status, architecture, accepted invariants, documentation authority, collaboration workflow, and detailed evidence without reconstructing long ChatGPT/Codex conversation history.

**Document information that is expensive to reconstruct, not details readily discoverable in code.** Avoid a parallel documentation universe that merely duplicates existing mission contracts, acceptance reports, research, and the large historical idea inventory.

A fresh reader should load a **small context bundle** rather than every historical document. Documentation accuracy, navigability, and honest provenance are the primary acceptance criteria, not the volume of new text.

## 2. Existing materials and their roles

Audit and classify the current material before editing. Relevant sources include:

- `README.md` and `AGENTS.md`;
- `docs/DesktopRoomie-roadmap-v0.1.md` (long-term proposals);
- `docs/DesktopRoomie-A00-A01-plan-v0.1.md` (near-term execution plan);
- `docs/DesktopRoomie-A00-M01-desktop-probe.md`, `DesktopRoomie-A00-M02-dragging-and-release.md`, and `DesktopRoomie-A00-M03-placement-recovery-host-selection.md` (mission contracts);
- `experiments/desktop-probe/ENVIRONMENT.md`, `ACCEPTANCE.md`, `ACCEPTANCE-M02.md`, `ACCEPTANCE-M03.1.md` (environment and acceptance evidence);
- `experiments/desktop-probe/M02.5-IMPLEMENTATION.md`, `M02.6-IMPLEMENTATION.md`, `M03.1-IMPLEMENTATION.md` (historical implementation explanation);
- `docs/research/ProjectVanilla-R00-external-projects-research-roadmap.md`, `docs/research/studies/` (research and evidence, not implementation authority);
- `docs/research/project_vanilla_all_ideas_design_inventory_v0.1.txt` (historical possibilities, not approved requirements);
- `.agents/skills/desktoproomie-verify/SKILL.md` (existing verification instructions);
- actual `experiments/desktop-probe/src/` modules (source of truth for implemented structure).

Do not infer current acceptance from a proposed roadmap or a historical header. Do not rewrite historical technical contracts to make them look like newer designs.

## 3. Deliverables and document boundaries

### `README.md`: public landing page

Explain briefly what reusable DesktopRoomie is versus private Project Vanilla, its experimental Rust/X11 probe status, first supported Openbox/X11/Picom target, minimal build/run instructions, and where the documentation index lives. Do not publish private/copyrighted character materials or turn README into an internal acceptance ledger.

### `docs/INDEX.md`: documentation map and authority

Index important existing and new documents by purpose; distinguish current/active guidance, accepted contracts/evidence, historical implementation notes, roadmaps, research, and idea inventory. Provide reading profiles for implementation, research/design, independent code review, and evidence/acceptance work. This is a stable directory, **not** a second roadmap or volatile status log.

### `docs/STATUS.md`: current checkpoint (deliberately volatile)

Keep compact. Record accepted M01/M02/M03.1 with acceptance links; present supported host with verified facts; current D00-M01 status and M03.2 as next **technical design** gate; relevant deferred near-term work/research; meaningful current limitations. Distinguish reviewed executable code revision `f8332ec99e876db1d809cd718dc76dff9fd38e27` from later docs-only HEAD. Avoid per-test matrices and ephemeral session notes. Do not claim D00-M01 accepted before independent review.

### `docs/ARCHITECTURE.md`: implemented responsibility map

Inspect actual source and describe state ownership, interactions, module boundaries, event/deadline flow, and native managed X11 host boundary. Cover current `main.rs`, `geometry.rs`, `interaction.rs`, `layer.rs`, and relevant `x11/` modules, including window, pointer, menu, monitors, state, rendering/visuals, shape, and resource cleanup **only as present in the repository**. Explain what is deliberately not implemented (Brain/Spine/runtime memory services, cross-device abstractions, A01 window-relative stacking). Architecture documentation is descriptive; no speculative interface design, function catalog, or refactor authorization.

### `docs/INVARIANTS.md`: accepted cross-mission guarantees

Distill short, traceable invariants with links to acceptance/specification evidence:
- no keyboard-focus theft or activation;
- explicit pointer-capture ownership, cancellation, and failed release obligation;
- click-through of transparent/noninteractive regions;
- checked resource cleanup and original-error priority;
- root/local coordinates, checked bounds and fitting;
- selected-monitor identity stability;
- managed WM cooperation and request-versus-observed-state truth;
- finite asynchronous operation deadlines/no WM-fighting retries;
- event-driven idle without permanent unnecessary polling;
- safe handling of body/layout/lifecycle interruption;
- reusable-public versus copyrighted/private asset boundary.

Treat an invariant as a **review obligation** grounded in accepted work, not an excuse to invent new behavior. Link details instead of reproducing full proofs or tests.

### `docs/WORKFLOW.md`: mission authority and publication process

Document ChatGPT architect/researcher/prompt-writer/independent reviewer; Codex implementation/documentation worker; owner approval, host acceptance, and merge authority.

Mission lifecycle: just-in-time research if necessary → bounded specification → read-only plan if appropriate → review → authorized execution → verification → explicitly authorized mission-branch commit/push → exact pushed-SHA diff/CI review → owner host acceptance when required → evidence/status update → owner-managed merge.

A specific owner-approved assignment may grant Codex normal commit/push access to a **named mission `mXX` branch** with known base SHA. Without that assignment, no publication permission exists. Never directly push `main`; never merge, force-push, rewrite, delete branches, create releases/tags, silently include unrelated changes, or advance scope. On unexpected upstream movement, non-fast-forward/rejected push, dirty unrelated owner changes, or branch/base mismatch: stop and report; no automatic reconciliation. A push and CI are not manual/independent acceptance.

Record reviewed code SHA independently of docs-only evidence commits. Distinguish actual tests, manual owner observations, pending checks, and historical evidence.

### `AGENTS.md`: mandatory, compact Codex rules

Preserve the existing constraints on scope, architecture/host boundaries, privacy, focus/pointer invariants, and verification honesty. Point to new durable docs as appropriate. Maintain scoped Git authorization from the D00-M01 bootstrap. Do **not** remove mandatory safety rules or turn `AGENTS.md` into an enormous project manual.

## 4. Documentation authority and lifecycle

- **Active approved mission contract + approved decisions:** current scope and acceptance requirements.
- **AGENTS/WORKFLOW:** rules for execution and collaboration.
- **INVARIANTS:** accepted non-regression obligations; never override a specific approved change by inference.
- **STATUS:** current checkpoint and links; not permission to implement.
- **ARCHITECTURE:** snapshot of actually implemented ownership, corrected when code/evidence disagree.
- **Acceptance evidence:** observed/reviewed result for an exact revision; distinguish unit tests/CI from host observation.
- **Implementation notes:** historical rationale and state at that checkpoint.
- **Roadmaps/plans:** directions/proposals, not verification.
- **Research:** comparative evidence/recommendations, not automatic coding decisions.
- **Idea inventory:** non-authoritative possibilities.

For earlier specs or notes that now misleadingly claim accepted M01/M02 work is unimplemented/pending, a **small historical lifecycle banner** may be added referring to the acceptance evidence. Preserve the original contract, old dates, and accurately historical statements. Do not bulk-modernize historical writing.

## 5. Required audit, drafting, and review phases

1. **D00-M01.1 Audit:** inventory actual docs/source modules, identify authority/conflicts/stale lifecycle labels, decide where existing information should be linked rather than repeated. Do not make broad edits yet.
2. **D00-M01.2 Context layer:** create INDEX, STATUS, ARCHITECTURE, INVARIANTS, WORKFLOW and improve README, with concise accurate cross-links.
3. **D00-M01.3 Instructions and historical banners:** adjust AGENTS to use new docs and add only targeted lifecycle/status pointers where necessary. Do not edit old technical specifications or evidence facts.
4. **D00-M01.4 Cold-start check:** independently inspect completeness, consistency, source traceability, link/path correctness, and scope. Remove needless duplication and conflicting claims.

These are phases of **one** docs-only mission, not additional code missions or authority to continue to M03.2.

## 6. Prohibited scope expansion

No Rust/source refactor, no split of `main.rs`, no Cargo/lockfile/dependency change, no workflow/CI/script or Git configuration change, no host setup alteration, no automatic architectural redesign, no future Brain/Spine/service API sketches, no full function-level API catalog, no dependency/class graph merely for decoration, and no unsolicited external research. Findings for future maintenance may be recorded only as deferred observations.

No manual owner acceptance is simulated or inferred. No invented status or fabricated CI data.

## 7. Verification and acceptance gate

Documentation-only verification must include:
- repository status, base SHA, scoped changed-file list, and final diff inspection;
- `git diff --check` (including the final reviewable commit/diff when applicable);
- verify internal links and file paths actually exist;
- confirm that no `src/`, Cargo, GitHub Actions, scripts, or binary files changed;
- inspect current status and the code modules before describing architecture;
- classify old lifecycle wording correctly, preserve historical evidence;
- confirm no accidental private assets/data entered the public repository.

**Cold-start test:** a reviewer reading AGENTS + STATUS + ARCHITECTURE + INVARIANTS + active mission, using INDEX for deeper links, can accurately answer:
1. DesktopRoomie vs private Project Vanilla;
2. currently supported host and evidence boundary;
3. accepted missions and exact acceptance sources;
4. active mission and next technical gate;
5. major implemented state/module owners and event flow;
6. accepted non-regression invariants;
7. distinction among spec, current status, source, research, and historical evidence;
8. where deeper decisions/experiments live;
9. where Codex may commit/push, and who may merge/accept;
10. how to identify tested code versus later docs-only commit revisions.

Do not run unnecessary Cargo/X11/graphical tests for a docs-only edit. Report only checks actually executed, and do not label documents "accepted" before ChatGPT's independent review and owner approval.

## 8. Completion report and next work

Report starting/ending branch/HEAD, clean/dirty state, every changed path, documents/links added, lifecycle banners added, exact checks/results, unresolved contradictions, and any deferred maintenance notes. If the explicit Codex assignment authorizes commit/push on the designated mission branch, report the exact pushed SHA and push outcome (CI may still be pending).

The repository documentation mission is complete only after independent review and human acceptance. **Next technical assignment afterward is A00-M03.2 read-only design for hide/show and same-instance independent recovery, not implementation.**
## 9. Final acceptance record — 2026-10-08

**Decision: D00-M01 ACCEPTED.** The owner approved closure after an independent ChatGPT review of the published documentation changes. The acceptance applies only to this docs-only mission, not to M03.2 implementation or remaining A00 work.

- **Approved baseline:** `47746975437c75e8244b6dc95ee714bf40135d6c` (approved contract and scoped Codex publication authority).
- **Reviewed D00-M01 implementation revision:** `e3266aeee5a4757f465b90376bcf97838e225888` on `m03`, one commit after that baseline.
- **Matching GitHub CI:** [run 37807374691](https://github.com/zNe4/DesktopRoomie/actions/runs/37807374691) completed successfully for that exact revision.
- **Independent review:** PASS. All five planned context documents, public README, AGENTS integration, and targeted historical lifecycle notices were present. Scope remained documentation-only. The context bundle recovered current status, source ownership, established invariants, collaboration authority, and evidence provenance without requiring the complete chat/idea history.
- **Documentation reference review:** 107 relative links had existing destinations; the reviewed heading anchors resolved. The changed paths were Markdown-only, with no Rust, Cargo, CI, or binary changes. Changes from the reviewed executable `f8332ec99e876db1d809cd718dc76dff9fd38e27` through the D00-M01 implementation were documentation-only.
- **Limit:** the accepted source executable remains `f8332ec99e876db1d809cd718dc76dff9fd38e27`; documentation publication and acceptance are not new host tests or code acceptance. This final status-only documentation revision follows the reviewed D00-M01 content revision.

**Next task:** A00-M03.2 read-only design for managed hide/show and independent same-instance recovery/control. The preliminary M03.2 boundary in the A00-M03 contract is not implementation authorization.
