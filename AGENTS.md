# Project Vanilla / DesktopRoomie — Codex instructions

## Identity and principles

Project Vanilla is a long-term modular desktop-companion and character-simulation project. DesktopRoomie (`zNe4/DesktopRoomie`) is the repository and reusable software name; Project Vanilla is its private character implementation. The local directory `~/Desktop/Utils/ProjectVanilla` is intentional. Work from the current Git repository root; do not rename or relocate it.

The Rust/X11 application in `experiments/desktop-probe/` is a technical foundation, not the complete companion. Develop incrementally, keep the executable usable, and avoid speculative infrastructure. Future design favors explicit state ownership, logical module boundaries, replaceable services, and separation of simulation from presentation. These principles do not authorize adding Brain, Spine, memory, or service architecture during unrelated missions.

## Responsibilities and mission discipline

The project uses a supervised multi-agent workflow:

- **ChatGPT, in a separate authoritative session:** external research and literature reviews; comparison of open-source implementations, technical approaches, and dependencies; research reports and architectural recommendations; research prerequisites, roadmaps, and high-level design decisions; bounded implementation specifications and Codex mission prompts; model and reasoning-effort selection per mission; independent review of completed work.
- **Codex:** inspect existing code and interfaces, implement assigned missions, maintain meaningful tests, debug failures, perform applicable build/format/lint/test checks, update technical documentation required by the mission, and report risks and ambiguities.
- **Human owner:** supervise, approve decisions and scope changes, perform manual desktop acceptance, and authorize commits and pushes. The owner is Codex's intermediary with the authoritative ChatGPT session.

Work on one assigned mission at a time. Read its specification and inspect existing code and Git status before editing. Respect explicit scope boundaries, preserve local changes and already verified behavior, and prefer small, auditable changes. Do not implement future roadmap features opportunistically or perform unrelated refactoring. Report ambiguities before consequential decisions. Alternatives may be proposed, but consequential architectural changes require approval.

Do not commit, push, create branches, or discard changes without explicit authorization. Suggested commits or branch workflows in older documents are not authorization. Do not automatically advance to the next mission.

## Documentation and authority

The current mission specification and explicitly approved decisions determine implementation scope. Planning documents provide context; their proposals and historical status labels are not proof of current implementation or acceptance. Inspect code and verification evidence.

Relevant implementation references:

- [Product roadmap](docs/DesktopRoomie-roadmap-v0.1.md): long-term direction and proposed sequencing.
- [A00/A01 plan](docs/DesktopRoomie-A00-A01-plan-v0.1.md): desktop goals, proposed defaults, boundaries, and acceptance scenes.
- [Desktop probe specification](docs/DesktopRoomie-A00-M01-desktop-probe.md) and [dragging/release specification](docs/DesktopRoomie-A00-M02-dragging-and-release.md): detailed contracts when the corresponding work is assigned.
- [Environment record](experiments/desktop-probe/ENVIRONMENT.md) and [M01 acceptance report](experiments/desktop-probe/ACCEPTANCE.md): recorded host facts and verification evidence. The recorded target is Openbox/X11 with Picom; older bspwm ideas do not establish the actual target or compatibility elsewhere.

`docs/research/` preserves possibilities, investigations, and evidence. Distinguish:

1. [Idea inventory](docs/research/project_vanilla_all_ideas_design_inventory_v0.1.txt): possibilities and historical experiments, not approved requirements.
2. [R00 research roadmap](docs/research/ProjectVanilla-R00-external-projects-research-roadmap.md): future investigations and their timing, not permission to implement findings.
3. Completed research findings: evidence informing design, not automatic coding instructions.
4. Approved implementation missions: actual coding instructions and acceptance contracts.

External research belongs to ChatGPT. Codex consumes approved findings and implementation contracts; ordinary inspection of this repository, existing dependencies, and relevant API contracts remains implementation work. Do not independently conduct broad external or comparative research, select a new architecture, or adopt a third-party implementation because it seems preferable. Report unresolved architectural questions or research dependencies to the owner. Do not automatically trigger R00 studies or block unrelated implementation on them.

Build independently. Do not copy third-party source code, assets, prompts, or architecture wholesale. Raise relevant license, attribution, and provenance concerns; actual reuse needs an approved scope decision.

## Engineering and content boundaries

Preserve the existing Rust architecture: pure geometry and interaction logic, X11 host operations under `src/x11/`, and startup/event/lifecycle coordination in `src/main.rs`. Change boundaries only as the assigned mission requires, without introducing a speculative framework.

Preserve these invariants, using mission specifications for detailed requirements:

- Explicit resource ownership and cleanup, including checked release and cleanup attempts after failures.
- Safe pointer capture acquisition, completion, cancellation, and release.
- Keyboard-focus isolation and transparent click-through outside intended input regions.
- Correct distinction between root and local coordinates; checked geometry, arithmetic, fit, and bounds.
- Event-driven processing where practical, without unnecessary idle polling.
- Deterministic tests for meaningful pure logic and transitions.
- Explicit error handling, recovery, and honest diagnostics.

Keep the potentially reusable public engine separate from private character content. Do not introduce copyrighted NEKOPARA assets, extracted dialogue, voice recordings, trained character models, or private sensor data into the public repository. Use synthetic fixtures where appropriate.

## Verification

The probe has its own Cargo manifest. Run these from the repository root after applicable code changes:

```sh
cargo build --manifest-path experiments/desktop-probe/Cargo.toml
cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check
cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path experiments/desktop-probe/Cargo.toml
```

To apply formatting, use `cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml`.

Run and diagnostic commands:

```sh
cargo run --manifest-path experiments/desktop-probe/Cargo.toml
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --delay 2 --duration 30
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --duration 5
cargo run --manifest-path experiments/desktop-probe/Cargo.toml -- --diagnose
```

Runtime checks require the actual X11 session; visible transparency also depends on the compositor. Automated checks cannot prove focus preservation, click-through, managed movement, or pointer recovery on the desktop. The owner performs manual acceptance against the assigned mission's checklist. Record actual outcomes and distinguish real-host tests from injected failures, untested cases, and accepted limitations. Preserve historical acceptance reports; add or update evidence only within assigned scope.

Run checks appropriate to the change and all checks required by the mission. Documentation-only changes require content/diff review rather than unnecessary desktop execution. Never claim a check ran when it did not. Compilation alone does not complete a mission; required acceptance evidence and review still apply.

## Implementation completion reports

Report the implemented changes, files changed, tests and checks actually executed with results, important technical decisions, remaining risks or limitations, manual testing required, and final Git status. Include commit information when committing was authorized; otherwise state that no commit was made. Clearly distinguish implementation progress from pending manual acceptance or independent review.
