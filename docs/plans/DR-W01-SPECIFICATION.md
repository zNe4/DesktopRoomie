# DesktopRoomie — DR-W01: Workflow and context continuity

**OWNER ACCEPTED — DR-W01 DOCUMENTATION/WORKFLOW CANDIDATE, 2026-11-26; GIT PUBLICATION SEPARATE**

Revision: B-R1 ratified, 2026-11-26. Mission branch: `dr-w01`. Verified starting HEAD: `4dc65ca8a80d1b0bd29a88b29cb3724b78eaeeaa` (clean tree).

## 1. Contract and approval boundary

Extend the accepted [D00-M01 contract](../DesktopRoomie-D00-M01-documentation-context-hygiene.md); retain its purpose, authority distinctions, technical protections and historical acceptance. This proposal changes context organization and workflow policy only. It neither accepts D3-C-I2 nor authorizes its continuation. Its separate local preservation ref, `checkpoint/d3c-i2`, resolves to `197126db36190cfe72f46febfed7ae31c947b8a7`; preservation is not technical acceptance.

Owner ratification was granted on 2026-11-26 following independent design review PASS of B-R1, resolving F01–F04. DR-W01-A was delivered outside the repository and reviewed by ChatGPT; the owner confirms its findings are covered by this design. This records the owner-provided review/ratification, not a new independent review by the implementer. The DR-W01-C assignment authorizes documentation implementation only on the pinned branch/base above: this specification, AGENTS.md, docs/STATUS.md, docs/WORKFLOW.md, docs/INDEX.md and .agents/skills/desktoproomie-verify/SKILL.md. No commits, publication, application/tooling changes or DR-W02 implementation are authorized. At the DR-W01-C assignment checkpoint, independent implementation review and DR-W01-D cold-start reconstruction remained pending before owner acceptance; section 12 records their subsequent disposition.

Existing [workflow](../WORKFLOW.md), [invariants](../INVARIANTS.md), approved mission contracts and verification obligations remain binding until approved migration. No reduction of technical guarantees, required checks, independent review, owner-native acceptance or owner publication authority is permitted. A conflict must be reported with both sources; the worker must not silently choose a weaker requirement.

## 2. Authoritative homes

One maintained home owns each live fact or rule. Other documents link to it; concise routing notices may identify a restriction without restating its detailed procedure. Historical records remain immutable evidence of their own checkpoint, not competing live policy. Conflicts require explicit correction with provenance, not deletion of inconvenient history.

| Home | Responsibility after approved implementation |
| --- | --- |
| [AGENTS.md](../../AGENTS.md) | Compact mandatory bootstrap: identity/public-private boundary, mission discipline, authority/stop guardrails, loading routes and pointers to binding rules. Remove the manually maintained project checkpoint. Retain explicit no-implicit-publication and no-implicit-scope-expansion guardrails. |
| [WORKFLOW.md](../WORKFLOW.md) | Canonical collaboration, role, assignment, escalation, verification routing, incremental review, handoff and publication procedures. Replace provider-specific role ownership with section 3. |
| [STATUS.md](../STATUS.md) | Sole maintained checkpoint summary: accepted baseline, active/unaccepted work, next approval gate, material limitations and direct evidence links. Distinguish source, reviewed candidate, published revision and acceptance. No historical narrative or test matrices. |
| [INDEX.md](../INDEX.md) | Navigation by purpose and loading profile; authority map and links to current STATUS and historical records. Remove copied checkpoint narratives; no parallel ledger. |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | Implemented ownership and interactions, traced to source; proposals stay in approved design/decision records. No workflow or acceptance ledger. |
| [INVARIANTS.md](../INVARIANTS.md) | Traceable cross-mission review obligations; detailed technical behavior stays in its approved contract. |
| Approved specifications / decision records | Mission requirements and owner-ratified architectural decisions. A later decision explicitly names what it supersedes; chronology alone does not confer authority. |
| Acceptance / review / evidence records | Exact candidate, observations, findings, approvals and historical decisions. Summaries reference these records without copying their full evidence. |
| README, skills, roadmaps and research | Landing/navigation, operational procedures, proposals and research respectively, under D00-M01 boundaries. Skills reference canonical policy; they do not grant authority. |

Migrate each duplicated live statement to its owning document or an existing evidence record before removing the duplicate. Preserve unique historical facts and their revision identities. Do not rewrite original contracts or evidence to match current tense. Do not move every detail into WORKFLOW: technical contracts retain their existing homes. Reconcile stale live publication labels against verified Git state, identifying the relevant local/remote revision and the evidence actually checked; a local commit alone does not prove remote publication. If remote state cannot be verified, mark publication unverified and escalate rather than infer it. Preserve historical publication labels as dated observations; correct current summaries through traceable pointers without rewriting historical records. Git publication proves neither technical acceptance nor owner approval, and this reconciliation grants no permission to mutate Git state.

## 3. Roles and authority

Roles are provider-independent; assignments identify the actual actor/session and responsibility. Tool or model access grants no decision authority.

- **Architect:** prepares bounded designs, approved research inputs, consequential architecture decisions and specifications for owner ratification. Implementers may propose alternatives but cannot adopt them unilaterally.
- **Implementer:** inspects and executes only authorized scope; preserves owner work and produces identifiable candidates and evidence.
- **Verifier:** performs applicable deterministic checks and records exact inputs/results/limits. May be the implementer; disclose this. Verification is not independent review or owner acceptance.
- **Independent reviewer:** a separate actor/session from candidate authorship, with access to the exact artifact and relevant contracts; reviews architecture or completed changes as assigned. An author's self-check is not independent review. Missing independent capacity leaves the gate pending.
- **Owner:** ratifies scope/decisions, authorizes publication per assignment, performs or explicitly qualifies required host acceptance, and alone controls acceptance and merge. Independent review remains mandatory; owner approval does not silently erase findings.

One actor may architect and later independently review another actor's implementation; a reviewer must not independently approve their own authored candidate. Record role overlaps. Delegation, parallel work and model choice require assignment authority; this policy does not authorize autonomous agents or broaden research scope.

## 4. Progressive loading and compact assignments

Bootstrap: AGENTS → STATUS → assignment and its approved specification → INDEX profile. This explicitly refines D00-M01's universal loading sequence: loading is staged, not optional compliance. Before any assigned action, read the applicable ARCHITECTURE and INVARIANTS sections and relevant WORKFLOW obligations, selecting them by the action's affected responsibilities, guarantees and authority; record applicability or a justified non-applicability in the assignment/handoff. Technical edits/reviews additionally require applicable technical contracts; documentation edits require the owning document and relevant authority contracts; publication/acceptance work requires exact review/evidence records. Selection must not omit binding obligations merely because the task is documentation-only. Load historical evidence only when a current claim, requirement or finding depends on it. Unread linked rules cannot be presumed satisfied.

Every assignment states: mission and role; concrete output; approved specification revision/path and approved amendments; expected branch and full starting SHA; allowed paths/actions and exclusions; required checks and acceptance gates; stop/escalation conditions; and explicit publication authority or prohibition. Reference persistent contracts instead of copying them. Identify any previously reviewed candidate and unresolved finding IDs. If approval or a required input is missing, perform only authorized independent inspection and report the missing dependency.

A fresh session reconstructs authority from repository records plus the explicit current assignment and any explicitly identified, authorized private review evidence (section 9), never from prior conversation memory, old prompts, branch names or historical next-task prose. Check actual branch, HEAD and tracked/untracked changes before editing. Preserve the existing mismatch, dirty-work and remote-state stop rules in WORKFLOW; no automatic reconciliation is introduced.

## 5. Risk-based effort and escalation

Mechanical verification uses minimal model reasoning and deterministic tools for identity, hashes, diffs, path/link checks, inventories and reproducible mechanical comparisons; model judgment interprets requirements, consequences and evidence. Tool success alone does not prove semantic correctness. Never replace an existing required check with reasoning or a cheaper proxy.

Reasoning and review-depth defaults below are advisory effort guidance, not exemptions from required gates. Use the highest applicable profile and increase depth when uncertainty or interactions warrant it; a task's file type alone does not determine risk.

| Risk profile | Advisory reasoning and review-depth default; required obligations |
| --- | --- |
| **Low:** mechanical documentation/navigation changes with no policy or semantic change | Minimal reasoning for mechanical verification; focused reasoning for authority/provenance checks. Review the scoped diff and referenced context, links and whitespace; independent documentation review and owner approval remain required. No native/Cargo execution solely for documentation. |
| **Moderate:** bounded behavior or substantive documentation/policy changes within approved architecture | Standard reasoning with explicit requirement-to-evidence mapping; review affected contracts, full changed behavior and adjacent interactions. Run applicable mandated automated checks and meaningful tests; independent source/documentation review, exact CI and owner-native gates remain required where applicable. |
| **High:** architecture/guarantee changes, ambiguous authority, evidence conflicts or safety-sensitive ownership/lifecycle/IPC/rendering regressions | Deep reasoning and expanded cumulative review of affected contracts, ownership, failure paths and proof boundaries. Pause dependent execution for unresolved architecture/authority/guarantee questions; architect resolves them and owner ratifies scope/contract changes before continuation. Retain all mandated verification and independent review. |

Record the selected risk and rationale in the assignment/handoff. Escalate when identity cannot be reproduced, required evidence is missing, a finding contradicts a guarantee, scope or baseline changes, or remote/publication state violates WORKFLOW. Routine implementation choices inside an unambiguous contract need no new architecture decision. Risk-based effort may increase scrutiny; it cannot waive mission-specific requirements or independent review. Model/reasoning selection is an explicit assignment choice based on this risk, not a provider-specific permanent rule.

## 6. Identifiable candidates and incremental independent review

A candidate is either a full commit SHA or a frozen uncommitted snapshot identified by full base SHA, complete changed-path inventory (including untracked additions), exact patch/content hashes and accessible artifacts sufficient to reconstruct it. A label such as I1 or a mutable branch alone is insufficient. Identify code/build inputs separately from documentation-only changes and evidence revisions.

A review record identifies reviewer, contract revisions, candidate, review scope, prior reviewed candidate, findings with stable IDs/severity/status, evidence consumed and limits. A previously reviewed candidate may have outstanding findings: it is a comparison anchor, never automatically an accepted baseline. Carry every unresolved finding forward with disposition and proof; do not reset the ledger when renaming a candidate.

For each correction, supply both the prior-reviewed→new delta and the approved-baseline→new cumulative change. Reviewer checks the correction, affected context and regressions, and verifies retained findings. Reuse prior review only for identifiable unchanged material within its actual reviewed scope. Expand to cumulative/full review when identities, requirements, dependencies or broad interactions change, or when prior coverage cannot be established. Repeating all unrelated review is unnecessary; claiming cumulative PASS from delta review without a traceable chain is forbidden.

Record fixed, open, rejected-with-approved-rationale and deferred findings distinctly. Blocking findings prevent technical acceptance; deferral of a mandatory requirement requires an explicit architect/owner contract decision and does not become a PASS. Publication is a separate owner-authorized action, not proof of acceptance. Preserve exact pushed-SHA diff and matching CI review under WORKFLOW even when pre-publication incremental reviews exist.

## 7. Evidence preservation and handoffs

Preserve all mission-required raw evidence and reproducibility artifacts in their existing approved locations with revision identities. DR-W01 does not mandate new archive formats, redundant bundles, universal file hashes or repeated full logs. Preservation requirements and reviewer consumption are separate: a reviewer starts with a compact claim-to-evidence map, then inspects the underlying evidence needed to substantiate the claims and findings. Evidence presence is not evidence inspection.

A handoff contains: assignment/spec identity; branch/base/candidate; scoped changes; checks actually run and outcomes; required pending checks; evidence paths; review anchor and open findings; limitations; next authorized gate; and final Git/publication state. Link detailed records rather than pasting logs. Distinguish local tests, CI, independent review, injected cases, owner-native observations and explicitly untested limits. State which evidence the reviewer consumed versus merely verified as preserved. Do not inherit acceptance across executable changes or turn documentation-only HEAD into a tested-code identity.

## 8. Future DR-W01 implementation envelope

Only a subsequent explicit assignment may edit AGENTS, WORKFLOW, STATUS and INDEX to implement this policy; narrowly scoped README, ARCHITECTURE or INVARIANTS routing corrections require inclusion in that assignment's allowlist. Allowlisted non-executable verification skill documentation corrections may update routing, terminology and policy references while preserving all verification commands and obligations; the assignment must name the exact skill documentation paths. Executable tooling and changes to verification behavior remain excluded. Preserve technical content and evidence identities. Any necessary new acceptance/review record must also be named in the allowlist. Do not bulk-modernize old missions, invent current acceptance, or resume D3-C.

No application/source/test, Cargo/dependency, script/CI, executable skill tooling, host/Git configuration, asset, branch/history or evidence-artifact change is authorized by this design. Publication remains separately authorized under existing WORKFLOW. The completed B/B-R1 drafting assignments modified only this specification. The subsequent DR-W01-C authorization is recorded in section 1; it does not expand the technical or publication envelope.

## 9. Acceptance gates for implementation

1. Review a before/after responsibility map for migrated live statements; no unique rule, guarantee, finding, historical decision or evidence identity is lost. Resolve conflicts explicitly, including stale live publication labels against verified Git state while retaining historical observations and marking unverifiable remote state honestly. AGENTS has no maintained checkpoint; STATUS alone owns it; INDEX routes rather than duplicates it.
2. Verify scoped file inventory, baseline/HEAD, complete diff, whitespace, internal paths/anchors and absence of prohibited changes. Use documentation checks, not native runs. Preserve required technical verification for any separately authorized technical mission.
3. Exercise one compact assignment and handoff against an identifiable previously reviewed candidate with an open finding. Demonstrate delta and cumulative access, carried finding disposition, evidence consumption limits and unchanged publication/acceptance gates. This is a workflow exercise, not new technical acceptance or a mandate to modify D3-C evidence.
4. Conduct an independent fresh-session cold-start reconstruction with no previous conversations. Provide the repository candidate, explicit assignment and, when needed, explicitly identified private review evidence whose use the owner has authorized for that reviewer. Identify each private artifact by stable revision/hash, scope, access location and authorization; record availability and actual consumption. Missing required access leaves the affected gate pending, never inferred. Keep private evidence outside the public repository and public diffs; public records may contain only authorized non-sensitive references or summaries. Private evidence supplies review provenance, not hidden execution authority or a substitute for the explicit assignment. No previous conversation history is required. Record loaded paths, answers with source references, errors/ambiguities and corrections. Answer all ten D00-M01 cold-start questions, using current evidence rather than its historical checkpoint, and additionally identify role authority, exact accepted versus unaccepted candidates, open findings, required evidence versus inspected evidence, publication permission and the next gate. Correct routing must recover all mandatory inputs without loading all history. No unsupported acceptance claim or hidden conversation dependency may remain; rerun affected questions after correction.
5. Independent architecture/documentation review must confirm these gates and preservation of existing obligations. Owner explicitly approves the exact reviewed candidate. Record acceptance separately from publication and any still-pending technical mission. A compact document alone is not acceptance.

## 10. DR-W02 boundary

DR-W01 establishes human-readable contracts only. DR-W02 requires its own proposed specification, independent architecture review, owner approval and pinned implementation assignment. Possible automation may check links, identity, inventories, candidate deltas or handoff completeness; it must cite authoritative inputs, expose uncertainty and fail visibly on missing/stale/conflicting inputs. Generated output must be identified as derived, reproducible and subordinate to its canonical inputs, never a second manually maintained authority.

DR-W02 must define interfaces, allowed writes, permission boundaries, failure/recovery behavior and verification before implementing anything. It must preserve raw evidence and review chains, support human inspection and fresh-session reconstruction, and avoid falsely treating mechanical checks as semantic review. No automatic architecture decisions, scope advancement, finding dismissal, acceptance, host-result inference, branch reconciliation, commits/pushes/merges or expanded agent/tool authority follow from DR-W01. Any future publication capability must remain subject to explicit mission-scoped owner authorization and the existing prohibitions; no unattended acceptance or merge is proposed.

## 11. Ratified review decisions

No implementation-blocking technical choice is delegated to the implementer. Owner ratification after independent B-R1 PASS covers the staged replacement of D00-M01's universal reading order, provider-independent role mapping/overlap, incremental review reuse from candidates with outstanding findings, DR-W01-A coverage and F01–F04 corrections. No automation technology, schema or provider selection is decided here. Design ratification is separate from DR-W01-C implementation review, DR-W01-D reconstruction and final owner acceptance.

## 12. Final acceptance and DR-W01-F closeout — 2026-11-26

The owner explicitly granted acceptance in the DR-W01-F assignment following **DR-W01-D-R1 independent assessment PASS**, and directed this documentation closeout. This records the owner-provided assessment and acceptance; the closeout author is also its mechanical verifier, not the independent reviewer. The accepted operational policies in AGENTS, WORKFLOW and the verification skill are preserved unchanged by F.

### Exact accepted candidate and review chain

- Branch and full base/unchanged HEAD: `dr-w01` / `4dc65ca8a80d1b0bd29a88b29cb3724b78eaeeaa`.
- Accepted uncommitted C-R1 cumulative patch SHA-256: `a6a6f20ddcc5330fd064e3a9c88741c935d817a2c7ad08c5e20f78fb22b5ea1b`.
- Complete candidate inventory: `AGENTS.md`, `docs/WORKFLOW.md`, `docs/STATUS.md`, `docs/INDEX.md`, `.agents/skills/desktoproomie-verify/SKILL.md` and this specification. The cumulative patch includes the untracked specification; a tracked-only Git diff is incomplete.
- The retained C-R1 cumulative patch and all six candidate file hashes were matched to the starting worktree before F edits. F is a separate three-document acceptance-record delta, not the original accepted patch. Its incremental and cumulative artifacts and final hashes are supplied in the closeout handoff, outside this record to avoid self-reference.
- C-R1 corrected C review findings F01 (stop on ordinary non-force push rejection) and F02 (changed identity alone does not invalidate traceable review coverage). DR-W01-D-R1 PASS and final acceptance are recorded on the owner's authority; F does not independently re-adjudicate that review or infer a finding ledger for D3-C-I2.

### Gate outcomes and preservation

The owner-directed closeout records successful independent cold-start reconstruction and incremental-review exercises under section 9, as covered by DR-W01-D-R1 PASS. The cold-start gate covers the ten D00-M01 questions and the additional role, identity, finding, evidence-consumption, publication and next-gate questions. The incremental exercise covers an identifiable prior candidate with an open finding, delta and cumulative access, carried disposition, limits on reused review/evidence and preserved publication/acceptance gates. These are workflow/documentation results, not new executable or native-host acceptance.

The recorded preservation result retains the authoritative-home map, staged mandatory loading, technical guarantees, verification commands/obligations, independent review, owner authority, exact evidence identities and public/private separation. Prior C-R1 verification recorded unchanged bytes for 195 files outside its two-path correction, successful cumulative and incremental replays, and passing six-document whitespace/link/anchor checks. F consumed that handoff and verification record and matched candidate identities; the detailed independent assessment, raw cold-start answers and private technical review evidence were not supplied or inspected in this closeout session. Their underlying contents are not reproduced or claimed as newly verified here. The explicit owner assignment supplies the PASS/acceptance provenance; it does not authorize disclosure of private evidence.

F documentation checks passed: exactly the three authorized paths changed, all other 194 nonignored repository files remained byte-identical, and AGENTS, WORKFLOW and the verification skill retained their accepted bytes. Six-document whitespace checks, 133 relative links and 22 anchors passed. Separate accepted-candidate→F and baseline→F scratch replays each reproduced all 197 files exactly; reverse applicability checks passed. The closeout handoff retains the incremental/cumulative patches and final content hashes. These are author/verifier results; focused independent closeout review remains pending.

### Remaining limits, publication and next gate

Acceptance applies to the exact documentation/workflow candidate above. It neither accepts nor resumes **D3-C-I2**, which remains paused and technically unaccepted at `197126db36190cfe72f46febfed7ae31c947b8a7`. Existing technical and host limitations remain in [STATUS](../STATUS.md#host-and-material-limitations) and their linked evidence. No Cargo/native checks or new CI are implied by documentation acceptance. DR-W02 still requires its own specification, independent review, owner approval and assignment.

Git publication is separate. The F read-only live remote check found `a01` at the pinned base, `main` at `4fe0ba519607626324be64210cae446b0dee9b92`, and `checkpoint/d3c-i2` at the preserved SHA above; no remote `dr-w01` ref was returned. F authorizes edits only to this specification, STATUS and INDEX, with no staging, commit, push, merge or other publication. Stop for **focused independent closeout review** of this delta and affected context. Owner-executable publication/integration commands are handoff guidance for a later owner-controlled action, not authority for the closeout author. Exact pushed-SHA review and matching CI remain separate obligations under WORKFLOW when publication occurs.
