# A00-M03.3 Stage G — integrated native acceptance closeout

**Status (2026-10-22): OWNER-APPROVED PASS for exercised Stage G scope; A00-M03.3 COMPLETE.** Owner explicitly accepted G0–G4 and confirmed the final G3-C, G3-D/E and G3-G visual observations. No source edits or host configuration changes were required. This is not M03.4 or A01 acceptance.

## 1. Immutable identity and evidence types

- **Accepted executable:** Stage F-T3 `f3458549dbbdd4b9ef42ed85d3bfaeb6ed683863`; [CI 39688537222](https://github.com/zNe4/DesktopRoomie/actions/runs/39688537222), **760 passing tests**, formatting, build and all-target Clippy. Stage F added 23 deterministic tests with no production changes; [Stage F closeout](DesktopRoomie-A00-M03.3-stage-F-closeout.md) remains an independent, historical source/test record.
- **Stage G starting documentation HEAD:** `ceedb0aec261531279a76f6dbf93950ac8837a20`, on `m03`; [CI 39690049162](https://github.com/zNe4/DesktopRoomie/actions/runs/39690049162) succeeded. At G0 and G4, the owner verified a clean local worktree, matching locally tracked `origin/m03`, and **no executable source difference** from F-T3 in the checked Rust/Cargo paths. The G4 comparison to `origin/m03` was a local remote-tracking check, not a fresh network fetch.
- **G0 owner-executed local checks:** Cargo format check, build, all-target Clippy with warnings denied, and tests were reported passing on the frozen executable. Those local outcomes and the matching GitHub CI are separate evidence; this closeout does not claim a new Rust test execution during documentation publication.
- **Live-host evidence:** Command exit codes and X11 snapshots collected by the owner, plus explicit user-observed typing, pointer and visibility behavior. These are not injected unit-test results. Command statuses alone do not prove imperceptible focus continuity or physical stacking.

## 2. Native host and run identity

| Attribute | Observed Stage G environment |
| --- | --- |
| Host | Arch Linux / Archcraft-style workstation; X11 `DISPLAY=:0`; `XDG_CURRENT_DESKTOP=openbox` |
| Window manager / compositor | Openbox 3.6.1 / Picom v13 (both running at G4) |
| Displays | **One** primary eDP-2 monitor, 1920×1080 at +0+0; five virtual desktops (0–4) |
| X11 facilities | RENDER 0.11; SHAPE 1.1; managed 160×160 procedural body; input silhouette |
| Body | `WM_CLASS=(desktop-probe, DesktopRoomie)`, `WM_NAME=DesktopRoomie-Probe` |
| Body identity | XID `54525954` (hex `0x3400002`) in many runs; destroyed and reused on later owner starts, **not** persistent process identity |
| User focus | Existing kitty focus and `_NET_ACTIVE_WINDOW` remained unchanged around controlled operations; actual focus XIDs varied between kitty terminals |

The final G4 read-only snapshot confirmed `m03` at `ceedb0aec261531279a76f6dbf93950ac8837a20`, clean worktree, no Rust/Cargo source change from F-T3, one eDP-2 monitor, five desktops, live Openbox/Picom and **zero DesktopRoomie bodies**.

## 3. Executed acceptance matrix

The labels below are **Stage G native test labels**, distinct from Stage F-P's deterministic G1–G3 coverage-gap labels.

| Check | Executed native evidence | Result |
| --- | --- | --- |
| **G0** | Frozen branch/source, clean environment, owner local fmt/build/Clippy/tests, matching 760-test CI and working X11 probe | **PASS** |
| **G1-A** | Local `--bring-here` and `--recover` already-satisfied behavior; same body/workspace/focus | **PASS** |
| **G1-B/C/D** | Remote visible Here preserving **Above / Normal / Below** from separate desktops; Openbox off-workspace Iconic/Unmapped without HIDDEN becomes Normal/Viewable after placement; geometry and focus preserved | **PASS** |
| **G1-E** | `--hide` sets genuine HIDDEN; remote Here moves desktop 3→4 while retaining Iconic/HIDDEN/Below and deferring input | **PASS** |
| **G1-F/G** | Local minimized Below→Above Recover and remote minimized Recover (desktop 4→0), with restoration and relocation histories reported only when applicable | **PASS** |
| **G1-H/I** | Remote non-minimized Normal→Above and Below→Above Recover; relocation history, no fabricated minimization restoration | **PASS** |
| **G2-A** | Continuous typing throughout Hide→Recover; focus and active window stable in sampled states; owner observed uninterrupted typing, no visible flicker and correct visibility | **PASS** |
| **G2-B** | Recover during open popup; owner saw no stranded popup and confirmed subsequent menu/body interaction | **PASS** |
| **G2-C** | Recover during held-button drag; post-release geometry stayed stable, pointer was usable and fresh drag/menu worked (owner-confirmed) | **PASS** |
| **G2-D** | Transparent padding passed mouse interaction to kitty; visible body/menu remained usable; post-Recover Above/Normal/Viewable, no focus flicker (all five owner checks YES) | **PASS** |
| **G3-A** | Eight simultaneous remote minimized Recover requests: **1 exit 0**, **7 Busy exit 5 (Preflight)**, **0 unexpected**, one body, correct final desktop/layer, focus preserved | **PASS** |
| **G3-B** | Sequential `--recover`, `--bring-here`, `--bring-top` after contention: all exit 0, no stuck Busy | **PASS** |
| **G3-C1/C2** | Normal and Below Hide/Show cycles: HIDDEN toggles correctly; original layer/desktop/160×160 geometry/focus preserved | **PASS** |
| **G3-C3** | Genuinely minimized Below on desktop 3, `--show` from desktop 4: removes HIDDEN without moving body or user; return to desktop 3 maps body Normal/Viewable/Below; owner confirmed visible interaction | **PASS** |
| **G3-D/E** | Menu Quit destroys original body; no-owner Show/Recover exit 3; fresh owner starts and accepts Recover with one body. Owner confirmed clean Quit and post-relaunch interaction | **PASS** |
| **G3-F** | Attempt duplicate owner: refused with exit **4**; original singleton body/window/focus unchanged | **PASS** |
| **G3-G** | 12-second timed owner creates body, exits **0** naturally, destroys body and yields no-owner Recover exit 3; owner confirmed visible appearance/disappearance | **PASS** |
| **G3-H1/H2** | Disposable owner SIGKILL returns **137**; body/selection released and no-owner exits 3. Subsequent owner reacquires singleton, rejects duplicate exit 4, accepts Recover, then naturally expires exit 0 with no body left | **PASS** |
| **G4** | Source/branch/evidence reconciliation, explicit final owner observations, and owner approval | **PASS** |

At G3-A the shell launches shared a start delay. Seven explicit Busy responses prove real contention occurred, but this does not guarantee coverage of every scheduler interleaving. In G2-B/C/D the owner observations are essential to the verdict; X11 property snapshots by themselves are not proof of popup/pointer release.

## 4. Explicit final owner signoff

**Date: 2026-10-22. Decision: approved.** The owner additionally answered **YES** to each previously unrecorded observation:

1. **G3-C:** Show restored Vanilla visibly and interactively on its original workspace.
2. **G3-D/E:** Quit exited the original owner and the relaunched body remained interactive.
3. **G3-G:** The timed body appeared and disappeared naturally without intervention.

The owner accepted Stage G with the documented scope limits. **No exercised Stage G mechanical or owner-observed failure remains open.**

## 5. Limits, nonclaims and next authority gate

**NOT EXERCISED in this Stage G host session:** live dual-monitor/hotplug or removed retained monitor, deliberate `_NET_WORKAREA`/viewport/desktop-count drift, compositor/WM replacement, fullscreen precedence, real X-server disconnection, WM refusal/malformed property injection, late native event storms at each deadline, lost terminal reply/OutcomeUnknown, all Partial branches, and exhaustive race/deadline interleavings. Earlier E4/H0/RF native evidence and Stage F deterministic/injected tests remain separately attributed; they are not relabelled as new Stage G runs. Synchronous X11 reply transport still lacks a hard independent timeout.

**Disposition:** **A00-M03.3 is closed at the tested Arch Linux/X11/Openbox/Picom scope.** The existing public `--bring-here` and `--recover` behavior is accepted without changes to Rust, Cargo, protocol, workflows, Openbox or Picom. This docs-only closeout does not imply completion of the overall A00 probe, M03.4 integrated closure, A01, other-WM support, Wayland, monitor hotplug, or broader fault-injection gates.

**Next:** M03.4 planning/authorization is a **separate owner decision**. No new implementation mission, commit or publication is authorized by this acceptance report.
