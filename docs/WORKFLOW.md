# Mission workflow and authority

The accepted [D00-M01 context contract](DesktopRoomie-D00-M01-documentation-context-hygiene.md) is extended by the owner-ratified [DR-W01 R1 specification](plans/DR-W01-SPECIFICATION.md). Persistent procedure lives here; [AGENTS](../AGENTS.md) supplies mandatory bootstrap guardrails. A proposed document, branch name or historical next step is never execution authority.

## Document responsibilities

One maintained home owns each live rule/fact; other documents route to it. Historical records retain their original dates, decisions and proof boundaries. Report conflicts with both sources rather than choosing a weaker requirement.

| Home | Authority and responsibility |
| --- | --- |
| Approved mission specification and ratified decisions | Assigned requirements, allowed changes and acceptance gates. Explicit amendments identify what they supersede; chronology alone grants no authority. |
| AGENTS / WORKFLOW | Mandatory bootstrap restrictions / canonical collaboration, escalation, verification, review and publication procedures. |
| [STATUS](STATUS.md) | Sole maintained checkpoint summary, accepted versus unaccepted candidates, limitations and next gate; no implementation permission. |
| [INDEX](INDEX.md) | Navigation and loading profiles, not a checkpoint ledger or roadmap. |
| [ARCHITECTURE](ARCHITECTURE.md) / source | Descriptive implemented ownership / actual implementation. Correct descriptive disagreement only within authorized scope; do not infer architectural approval from code. |
| [INVARIANTS](INVARIANTS.md) | Traceable cross-mission review obligations; detailed behavior remains in its approved contract. No inferred override of an explicitly approved change. |
| Acceptance, review and decision records | Exact candidate, historical decision, observation, approval and finding provenance. Separate executable identity from later evidence/documentation revisions. |
| Implementation notes / roadmaps / research / idea inventory | Historical rationale / proposals / evidence and recommendations / possibilities; none automatically authorizes implementation. |
| README / skills | Public entry point / operational procedures. Reference canonical policy, never grant authority. |

When consolidating, preserve unique facts and revision identities in their owning records before removing duplicates; do not bulk-modernize historical contracts. Reconcile stale live publication labels against verified local and live remote Git state, identifying revisions and evidence checked. A local commit alone does not prove publication. If remote state cannot be verified, mark it unverified and escalate. Preserve historical labels as dated observations and route current summaries to the correction. Reconciliation of labels grants no Git mutation permission or technical acceptance.

## Roles and scope

Assignments name actual actors/sessions, independent of provider or tool access:

- **Architect:** external/comparative research, architecture recommendations/decisions, authoritative design/roadmap documents and bounded specifications for owner ratification. Implementers may propose alternatives but cannot adopt them independently.
- **Implementer:** repository/dependency/API inspection, scoped read-only planning, approved implementation/tests/debugging and implementation-specific evidence. No broad external research, architecture adoption or roadmap change on its own initiative; no unrelated R00 study.
- **Verifier:** applicable deterministic checks and honest input/result/limit recording; may be the implementer, with overlap disclosed. Verification is neither independent review nor acceptance.
- **Independent reviewer:** separate actor/session from candidate authorship, with exact artifacts and relevant contracts. An architect may review another actor's implementation; an author cannot independently approve their own candidate. Missing independent capacity leaves the gate pending.
- **Owner:** ratifies decisions/scope, relays evidence where necessary, authorizes mission-scoped publication, performs or explicitly qualifies required real-host acceptance, and controls acceptance and merge. Owner approval does not silently erase findings or remove independent review.

Record role overlaps. Delegation, parallel work and model choice require assignment authority; tool availability grants none. Work one assigned mission at a time. Resolve consequential ambiguity before dependent work; routine implementation choices within an unambiguous contract need no new architecture decision. Preserve accepted behavior and local owner work.

## Assignments and context loading

Use compact assignments: mission/role and output; approved specification revision/path and amendments; expected branch/full starting SHA; allowed paths/actions and exclusions; required checks and acceptance gates; stop/escalation conditions; explicit publication authority or prohibition. Reference durable contracts rather than copying history. Identify prior reviewed candidate and open finding IDs when applicable.

Bootstrap and role routes are in [INDEX](INDEX.md#reading-profiles). Before assigned actions, selectively read applicable ARCHITECTURE and INVARIANTS sections and relevant WORKFLOW obligations; record applicability or justified non-applicability. Technical edits/reviews additionally load affected contracts; documentation edits load the owning document/authority sources; publication/acceptance loads exact evidence/review records. Do not assume unread linked rules are satisfied. Historical evidence is loaded when a current claim, obligation or finding depends on it.

Prefer a fresh session for each small approved checkpoint. Reconstruct authority from repository records and the explicit current assignment, not prior conversation memory, old prompts or historical next-task prose. Explicitly identified, owner-authorized private review evidence may supply provenance under the evidence rules below. If approval or required inputs are missing, perform only authorized independent inspection and report the dependency; do not advance scope.

## Risk and verification

Use deterministic tools with minimal model reasoning for hashes, identities, diffs, inventories and path/link checks. Model judgment interprets requirements, consequences and evidence; mechanical success does not establish semantic correctness. Record the highest applicable risk profile and rationale in assignment/handoff. These reasoning/review-depth defaults are advisory; they cannot reduce required gates or independent review.

| Profile | Advisory effort and review depth |
| --- | --- |
| Low — mechanical documentation/navigation, no semantic/policy change | Minimal mechanical reasoning, focused authority/provenance review, scoped diff/context and links/whitespace. Independent documentation review and owner approval remain required. |
| Moderate — bounded behavior or substantive documentation/policy within approved architecture | Standard reasoning and requirement-to-evidence mapping; review affected contracts, changed behavior and adjacent interactions. Required automated checks, meaningful tests, independent source/documentation review, matching CI and owner-native gates apply as specified. |
| High — architecture/guarantee changes, authority ambiguity, evidence conflict or safety-sensitive ownership/lifecycle/IPC/rendering regression | Deep reasoning and expanded cumulative review of affected ownership, failure paths, contracts and proof boundaries. Pause dependent execution for unresolved design/authority/guarantee questions; architect resolves and owner ratifies scope/contract changes. |

Escalate missing evidence, unreproducible identities, guarantee contradictions, scope/baseline changes and publication-state violations. Increase depth for uncertainty/interactions regardless of file type. Model/reasoning choice is an assignment choice, not a permanent provider rule. Never replace a required check with reasoning or a cheaper proxy.

Documentation-only changes require baseline/status, complete tracked/untracked scoped diff and content/consistency review, `git diff --check` (plus checks covering new files), internal links/anchors and prohibited-path inspection; no unnecessary Cargo/native runs. Rust changes require all four desktop-probe Cargo checks in the [verification skill](../.agents/skills/desktoproomie-verify/SKILL.md), relevant component checks and mission-specific tests. Apply formatting only when authorized/appropriate using `cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml`. Build success alone is insufficient.

Actual X11-session checks depend on the WM and compositor. Automated tests cannot prove focus, click-through, managed movement or pointer recovery on the desktop; owner observations, injected failures and CI remain distinct. Preserve required meaningful pure-logic/transition tests and test identities; never weaken checks to accommodate a move. Record actual outcomes, pending/untested cases and accepted limitations without fabricating host acceptance.

## Lifecycle

1. Architect supplies just-in-time research/design when needed, then a bounded specification and owner-ratified decisions; scoped read-only planning and independent design review precede execution where required.
2. Implementer executes the explicit assignment after inspecting baseline/status/contracts; verifier records applicable checks and full scoped diff.
3. Independent review examines an exact identifiable candidate; corrections follow the incremental review procedure. No automatic next mission.
4. Only with explicit publication authority, publish verified scoped work under the restrictions below. Independently review the exact pushed-SHA diff and matching CI, even if pre-publication review already occurred.
5. Owner performs required host acceptance after source/CI review, or the specified documentation/cold-start gate. Record actual evidence and qualifications, then owner approval of the exact candidate. Publication and CI are not acceptance.
6. Update evidence/checkpoint only within authorized paths, preserving tested-source versus evidence identities. Owner manages merge and the next assignment. DR-W01's independent cold-start exercise is a separate DR-W01-D gate; implementing these documents does not satisfy it.

## Mission-scoped publication

**Default:** no branch creation/switch, commit, push, merge, discard or scope advancement without explicit authority. Old examples and previous missions grant no standing permission.

A specific owner assignment may authorize creation/switch to a named mission branch, expected starting branch/full base SHA and explicit commit/push permission. Only verified mission-scoped paths may be committed; use an ordinary non-force push to `origin` on that branch. Inspect actual branch, HEAD, tracked/untracked changes and live remote state before editing; confirm the approved baseline. Fast-forward-only synchronization requires explicit instruction and safety. Inspect complete diff, status, HEAD and intended staged paths before committing; check remote again before publication.

On branch/base mismatch, divergence, unexpected remote movement, unrelated dirty owner changes or rejection of an ordinary non-force push (including a non-fast-forward rejection): **stop and report**. Do not automatically merge, rebase, force, discard or reconcile. Preserve unrelated owner work.

The implementer must never directly push `main`, merge, force-push, rewrite published history, delete branches or create releases/tags. Do not change Git configuration or relocate the repository during a documentation mission. Owner controls acceptance and merges. Report any actually observed CI and exact publication result; a successful push/CI is not acceptance.

## Identifiable candidates and incremental review

Use a full commit SHA or a frozen uncommitted snapshot: full base SHA, complete changed-path inventory including untracked additions, exact patch/content hashes and accessible reconstruction artifacts. A mutable branch or I1 label is insufficient. Separate code/build input identities from documentation/evidence revisions.

Review records identify reviewer, candidate, contract revisions, actual scope, prior reviewed candidate, stable finding IDs/severity/status, evidence consumed and limits. A previously reviewed candidate with open findings is a valid comparison anchor, not an accepted baseline. Carry unresolved findings forward; renaming candidates never resets them.

Corrections provide both prior-reviewed→new delta and approved-baseline→new cumulative change. Review correction, affected context/regressions and retained findings. Reuse prior review only for identifiable unchanged material within its actual scope. A changed candidate SHA or snapshot hash alone does not invalidate traceable prior review coverage. Expand to cumulative/full review when identity traceability is lost, prior coverage cannot be established, or changes to requirements, dependencies or broad interactions invalidate that coverage; review the affected scope rather than automatically repeating all prior review. Do not claim cumulative PASS from an untraceable delta review or repeat unrelated review merely for volume.

Distinguish fixed, open, rejected-with-approved-rationale and deferred findings. Blocking findings prevent technical acceptance. Deferring a mandatory requirement requires explicit architect/owner contract change, never an invented PASS. Preserve independent exact pushed-SHA/CI review and owner acceptance gates.

## Evidence and handoffs

Preserve mission-required raw evidence/reproducibility artifacts in approved locations with revision identities. No universal redundant archives, full logs or file hashes are required beyond the mission/candidate identification needs. Evidence preservation is separate from consumption: provide a compact claim-to-evidence map; reviewers inspect what substantiates the claims/findings and state what was consumed versus merely checked as preserved. Evidence presence is not inspection.

Separate local checks, matching clean-environment CI, independent review, owner-native observations, controlled/injected tests and pending/untested cases. Record full reviewed executable SHA independently of documentation-only revisions. An empty source/build/CI diff proves unchanged inputs, not new checks or host acceptance. Historical dates/contracts and failed observations stay historical; add lifecycle pointers only within authorized scope.

For fresh-session/cold-start work, the assignment may explicitly identify owner-authorized private review evidence for the intended reviewer. Record stable revision/hash, scope, access location, authorization, availability and actual consumption. Missing required access leaves the affected gate pending. Keep private artifacts outside public repositories/diffs; include only authorized non-sensitive references/summaries publicly. Private provenance is not hidden execution authority or a replacement for the explicit assignment; previous conversations are unnecessary.

Return a compact handoff with mission/spec revision; actor/role overlap and applicable context/risk; branch/base/candidate; scoped changes; actual commands/results and pending checks; evidence paths and consumed/preserved limits; review anchor/open findings; limitations; next authorized gate; starting/ending HEAD and final status; and authorized publication state, new commit/pushed SHA/result or explicit no commit/push. Link detail rather than pasting logs. Never infer acceptance across executable changes.

## Automation boundary

[DR-W01](plans/DR-W01-SPECIFICATION.md#10-dr-w02-boundary) defines the requirements for a separately specified, reviewed and owner-approved DR-W02. No executable tooling, automation, agent expansion, autonomous finding dismissal, scope advancement, publication, acceptance or merge is authorized by this workflow. Derived outputs must remain visibly subordinate to canonical inputs; mechanical checks cannot replace semantic review or owner authority.
