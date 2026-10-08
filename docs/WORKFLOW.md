# Mission workflow and authority

## Roles and scope

- **ChatGPT in the separate authoritative session:** external research/comparison, architecture recommendations and decisions, bounded specifications/prompts, model/reasoning selection and independent review of exact completed work.
- **Codex:** repository/API inspection, assigned implementation or documentation, meaningful tests/debugging, applicable verification, technical documentation and honest reporting of gaps/risks. No broad external research or architecture adoption on its own initiative.
- **Owner:** intermediary with ChatGPT, scope/decision approval, real-host acceptance, mission-scoped publication authorization and merge authority.

Work one assigned mission at a time. The active approved contract and explicit decisions determine scope; [AGENTS](../AGENTS.md) supplies mandatory operating restrictions. [INVARIANTS](INVARIANTS.md) captures accepted review obligations, [STATUS](STATUS.md) the current checkpoint, [ARCHITECTURE](ARCHITECTURE.md) implemented ownership, and [INDEX](INDEX.md) deeper evidence. Roadmaps, studies and historical next-action text are not authorization.

## Lifecycle

1. Just-in-time research by ChatGPT if the next decision needs it; existing repository/dependency/API inspection remains ordinary Codex work. Do not trigger unrelated R00 studies.
2. Bounded specification and approved decisions, followed by a read-only implementation/design plan when appropriate and independent review before authorized execution.
3. Codex execution within the approved scope, preserving owner work and accepted behavior; report consequential ambiguity before changing architecture.
4. Applicable local verification and complete scoped diff review. Documentation-only work uses content/path/consistency review and `git diff --check`; it does not need Cargo or desktop runs. Rust work uses the checks in AGENTS and the [verification skill](../.agents/skills/desktoproomie-verify/SKILL.md), plus mission-specific requirements.
5. Only with explicit assignment authorization, commit/push verified work to the named mission branch under the publication rules below.
6. ChatGPT independently reviews the **exact pushed-SHA diff and matching CI**, with the owner relaying evidence. Review corrections remain in scope; a successful push/CI is not acceptance.
7. Owner performs actual host acceptance when required, after source/CI review. Record actual results and limitations; do not simulate manual acceptance. For docs-only D00-M01, independent consistency/cold-start review and owner approval are the gate.
8. Update evidence/current status within authorized scope, identifying tested code separately from evidence commits. Owner manages the merge and approval of the next assignment. Codex does not advance missions automatically.

## Mission-scoped publication

**Default:** no branch creation, commit, push, merge, discard or scope advancement without explicit authority. Old documents, examples and previous missions are not standing permission.

A specific owner-approved assignment can authorize Codex to create/switch to a **named mission `mXX` branch**, with the expected starting branch/base SHA and explicit commit/push permission. That permission applies only to that assignment and branch. Commit only verified mission-scoped paths; use an ordinary non-force push to `origin` on that branch.

Before editing, inspect branch, HEAD, tracked/untracked status and actual remote branch state. Confirm the approved baseline. Fast-forward-only synchronization is permissible when explicitly instructed and safe; unexpected upstream movement is not a reason to reconcile automatically. Before committing, inspect status, HEAD, the complete diff and intended staged paths; preserve unrelated owner work and never silently include it.

On **branch/base mismatch, divergence, unexpected upstream movement, unrelated dirty owner changes, or a non-fast-forward/rejected push: stop and report**. Do not automatically merge, rebase, force, discard or otherwise reconcile. Check the remote again before publication.

Codex must never directly push `main`, merge, force-push, rewrite published history, delete branches, create releases/tags or include unrelated changes under mission publication authority. The owner controls acceptance and merges. Repository location is intentional; do not rename/relocate it or change Git configuration as part of a documentation mission.

Report authorized branch/base, starting and ending HEAD, changed paths, exact checks/results, commit/pushed SHA, push result, any actually observed CI, pending review/manual acceptance and final `git status --short --branch`. If publication was not authorized or failed, state what was and was not committed/pushed.

## Evidence and revision identity

Acceptance records must separate local automated checks, matching clean-environment CI, independent source review, owner desktop observations, controlled/injected failure tests and pending/untested cases. Preserve historical dates, facts and original contracts. Add a small lifecycle pointer when old pending language could mislead; do not rewrite earlier observations into current results.

Record a full **reviewed executable code SHA** independently of subsequent documentation-only revisions. HEAD alone can be misleading, especially for an original uncommitted implementation session. [M03.1 evidence](../experiments/desktop-probe/ACCEPTANCE-M03.1.md) identifies corrected code `f8332ec99e876db1d809cd718dc76dff9fd38e27`; later evidence/context commits do not identify distinct tested executables. To check whether a later HEAD preserves that foundation, inspect:

```sh
git diff f8332ec99e876db1d809cd718dc76dff9fd38e27 HEAD -- experiments/desktop-probe/src experiments/desktop-probe/Cargo.toml experiments/desktop-probe/Cargo.lock .github/workflows
```

An empty diff on those paths confirms unchanged source/build/CI definitions, not a newly executed check or host test. New code requires its own verification/review evidence. After acceptance, keep STATUS compact and link detailed records rather than copying per-test matrices or transient session logs.
