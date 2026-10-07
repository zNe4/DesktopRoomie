---
name: desktoproomie-verify
description: Verify DesktopRoomie Rust implementation changes after a coding mission, bug fix, or reviewer correction. Run the project's documented Cargo checks, inspect Git state, and report automated results separately from manual X11 desktop acceptance. Do not use for research, planning, or documentation-only edits.
---

# DesktopRoomie — Verify an implementation

Use this skill when the owner or an assigned mission requests verification of Rust changes (for example, "verify M02.5", "run the DesktopRoomie checks", or "validate this fix"). It is a **verification procedure**, not authority to implement, approve acceptance, commit, push, or advance missions.

## Inputs and boundaries

- Work from the Git repository root, regardless of its local directory name. Read root `AGENTS.md` and the **assigned mission specification** before deciding which checks apply.
- Identify the changed component(s), the relevant Cargo manifest(s), any mission-specific tests, and the acceptance requirements. For the current desktop probe, use `experiments/desktop-probe/Cargo.toml`.
- If the task is **only** documentation, planning, or research, do not run Cargo checks by default: inspect documentation and Git diff instead. If the task is outside DesktopRoomie or the assigned scope is unclear, report that rather than guessing.
- Follow any stronger constraints in the current mission or owner's instructions. Never interpret this skill as permission to modify application code, run graphical tests, install dependencies, access the network, or change desktop/system configuration.

## Verification procedure

1. **Record the baseline.** Run `git status --short --branch` and `git rev-parse HEAD`. Identify tracked and untracked changes. Do not stage, discard, or rewrite anything. Read the relevant diff and mission scope; use `git diff --check` to detect whitespace errors in tracked changes (it does not cover untracked files).
2. **Choose the checks.** For changes under `experiments/desktop-probe/`, run the four exact commands below from the repository root. If a later mission uses another crate or build system, inspect the actual repository instructions and report the commands chosen; do not assume the probe commands cover unrelated components.
3. **Execute and record each check independently.** Preserve each command's real exit status and diagnostic output. If one check fails, attempt the others when still safe and meaningful. Do not mask errors, automatically apply formatting, silently regenerate dependencies, or change project files to obtain a pass. If sandbox permissions, missing tools, or unavailable dependencies prevent a check, record it as **BLOCKED**, not PASS.
4. **Inspect regressions and evidence.** Compare observed results with the mission's required tests and already-proven invariants. Summarize meaningful failures with enough command output to reproduce them. Do not equate compile success, unit tests, or simulated tests with real-desktop verification.
5. **Separate manual acceptance.** Extract a short, mission-specific checklist for the owner to run on the recorded X11/WM/compositor host (e.g., focus isolation, transparent-region click-through, drag/cancel/release, changed display/workspace bounds). Mark all items **NOT RUN** until the owner provides observations. Never run `cargo run`, rearrange monitors, synthesize mouse input, or manipulate the user's graphical session without explicit authorization.
6. **Check the final state.** Run `git status --short --branch` again and report any new file changes or unexpected artifacts. Do not commit, push, create branches, or mark the mission accepted.

### Desktop-probe Cargo checks

```bash
cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check
cargo build --manifest-path experiments/desktop-probe/Cargo.toml
cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path experiments/desktop-probe/Cargo.toml
```

Run these from the repository root. Report the real outcome of **each** command, including failures or skips; do not assume a previously reported test count still holds. Building may create ordinary ignored `target/` artifacts, but should not unexpectedly change tracked sources or lockfiles. Raise such changes to the owner.

## Required handoff format

Give a concise verification report containing:

- **Scope:** mission/fix, examined revision (`HEAD`), affected components, and whether the working tree was clean initially.
- **Automated checks:** one row per command with `PASS`, `FAIL`, `BLOCKED`, or `NOT RUN`, plus relevant diagnostics or test counts supported by actual output.
- **Diff/scope observations:** unexpected changes, whitespace problems, and regressions; explicitly note anything not examined.
- **Manual desktop acceptance:** concrete mission-specific scenarios, all marked pending unless the owner reports actual results.
- **Outcome:** `automated verification passed/failed/incomplete`, distinct from `mission accepted` (which requires human evidence and independent review).
- **Final Git state:** changed/untracked files and confirmation that no commit or push was performed by this skill.

If results are incomplete, say exactly what prevented completion and what needs to happen next. Never fabricate test executions, desktop observations, or cleanup success.
