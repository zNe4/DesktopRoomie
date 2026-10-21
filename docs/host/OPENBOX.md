# Openbox host integration — user-managed setup

**Acceptance status update, 2026-10-21:** M03.2 remains final-accepted at H. M03.3 standalone `--bring-here` is Stage-D accepted at `4c67da0b09ffe7b69605ac4a415d256acf8abd23` ([CI 39377294762](https://github.com/zNe4/DesktopRoomie/actions/runs/39377294762), 677 tests). Public `--recover` is **Stage-E accepted on the exercised Openbox/X11 host** at `aa00f8e94aff1cf2ef1ff6a6cecf734f822d5603`, [CI 39529051360](https://github.com/zNe4/DesktopRoomie/actions/runs/39529051360), 737 tests. Owner N01–N19 native scenarios passed with [documented limitations](../../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md). No host configuration edit or Stage-F/G authorization is implied.

**Historical 2026-10-18 snapshot:** M03.2 H and M03.3 A/B/H0/C were the then-accepted foundation, before public Bring Here. Current status is the Stage-E acceptance above; dated M03.2 F/G/H evidence stays tied to original tested revisions.

DesktopRoomie should own safe same-instance control, WM-state verification, input cleanup and keyboard-focus isolation. Openbox should be reused for *optional* user conveniences such as keyboard shortcuts, application menus and session autostart. The application must be recoverable from its CLI without optional Openbox bindings; no global key-grabbing daemon, tray framework or automatic dotfile rewriting is required.

## Openbox off-workspace visibility amendment (2026-10-10)

This historical amendment preserves owner-supplied real-host evidence and the owner-approved correction design for **Openbox 3.6.1/X11/Picom**. At the 2026-10-10 checkpoint, A–E were accepted at `174d8a92574e16768330fb3c315578b105e4b698`, and F awaited correction/review/native acceptance. The amendment was subsequently implemented in `5ed75d7…` and accepted at `ccd0191…`, as recorded separately below. The original failed gate and manual experiments are not retroactively executable passes. Later M03.2 final acceptance is recorded in STATUS and ACCEPTANCE-M03.2.

### Failed Stage-F host gate

Candidate `2424c8887d83d1e503caba326cd512d036639c3e` passed source review and [matching CI 38089949879](https://github.com/zNe4/DesktopRoomie/actions/runs/38089949879), **332 passed, 0 failed**. Basic same-workspace native Hide passed: `WM_STATE=Iconic`, `_NET_WM_STATE` contained `_NET_WM_STATE_HIDDEN`, `_NET_WM_DESKTOP=0`, Map State `IsUnMapped`. Hide → Show → Hide also worked locally. These initial checks do not establish full Stage-F acceptance.

**H15 failed:** with body workspace 0 and user/current workspace 1, the owner ran `desktop-probe --hide` and received **`HIDE=7`**. Observed:

```text
WM_STATE = Iconic
_NET_WM_STATE = empty
_NET_WM_DESKTOP = 0
Map State = IsUnMapped
_NET_CURRENT_DESKTOP = 1
```

This is coherent managed off-workspace **not-minimized** state. Continued Iconic/unmapped without HIDDEN must never count as Hide success. The historical failure remains a failure.

### Host semantics collision

Openbox computes ICCCM `WM_STATE=Iconic` when the client is actually iconified **or** merely on a non-current desktop. Its separate internal iconic flag controls whether `_NET_WM_STATE_HIDDEN` is present. An ordinary off-workspace body therefore has Iconic + Unmapped + no HIDDEN.

On `WM_CHANGE_STATE / IconicState`, Openbox compares the requested ICCCM state with its current `WM_STATE`. The off-workspace body already reports Iconic, so Openbox treats the request as no state change and does not set its internal iconified state; HIDDEN never appears. This is a **host semantics collision**, not a DesktopRoomie timeout, scheduler defect or insufficient wait budget.

### Owner manual experiments, separate from executable behavior

The owner used **wmctrl only as an experimental sender** of root-directed EWMH ClientMessages to the WM. These were not direct property writes and were not DesktopRoomie executable implementations. wmctrl is not a product dependency.

1. **ADD-HIDDEN:** a WM-directed `_NET_WM_STATE ADD _NET_WM_STATE_HIDDEN` request, while body workspace was 0 and current workspace was 1, produced:

   ```text
   WM_STATE = Iconic
   _NET_WM_STATE = _NET_WM_STATE_HIDDEN, _NET_WM_STATE_ABOVE
   _NET_WM_DESKTOP = 0
   Map State = IsUnMapped
   _NET_CURRENT_DESKTOP = 1
   ```

   This confirmed actual Openbox minimization without moving the body or switching the user workspace.

2. **Existing Show failed after that manual Hide:** the then-current Stage-E checked user-time-zero + MapWindow mechanism returned **`SHOW=7`** for the genuinely minimized off-workspace body:

   ```text
   WM_STATE = Iconic
   _NET_WM_STATE = _NET_WM_STATE_HIDDEN, _NET_WM_STATE_ABOVE, _NET_WM_STATE_DEMANDS_ATTENTION
   _NET_WM_DESKTOP = 0
   _NET_CURRENT_DESKTOP = 1
   ```

   Openbox handles MapRequest on an actually iconified client through its activation path. For a client on another desktop that path refuses to present it there and marks it demanding attention. Thus accepted local MapWindow Show is not a valid off-workspace deiconification mechanism on Openbox. This corrects an unverified Stage-E host representation; previously accepted local Show behavior remains accepted.

3. **REMOVE-HIDDEN:** the owner then sent WM-directed `_NET_WM_STATE REMOVE _NET_WM_STATE_HIDDEN` with body workspace 0 and current workspace 1:

   ```text
   WM_STATE = Iconic
   _NET_WM_STATE = _NET_WM_STATE_ABOVE, _NET_WM_STATE_DEMANDS_ATTENTION
   _NET_WM_DESKTOP = 0
   Map State = IsUnMapped
   _NET_CURRENT_DESKTOP = 1
   ```

   HIDDEN disappeared, both workspace values remained unchanged, and the body stayed unmapped while off-workspace. The owner manually returned to workspace 0 and Vanilla appeared normally. DEMANDS_ATTENTION was left over from the earlier failed MapWindow experiment; it must **not** be attributed to REMOVE-HIDDEN.

### Approved request matrix — implemented in the F correction

The semantic contract remains **Hide = genuinely minimized; Show = not minimized**. The host adapter needs distinct primitives for local and off-workspace state:

| State | Hide request | Show request when genuinely minimized |
| --- | --- | --- |
| Current workspace / all desktops | Retain checked ICCCM `WM_CHANGE_STATE → IconicState` | Retain accepted checked `_NET_WM_USER_TIME=0` + MapWindow |
| Other workspace | Checked root-directed EWMH `_NET_WM_STATE`, ADD HIDDEN | Checked root-directed EWMH `_NET_WM_STATE`, REMOVE HIDDEN; **no MapWindow** |

For the EWMH path, use `SendEvent(false, body_root, SUBSTRUCTURE_NOTIFY | SUBSTRUCTURE_REDIRECT, ClientMessage)` with `window=body`, `type=_NET_WM_STATE`, format 32, and data `[action, _NET_WM_STATE_HIDDEN, 0, application_source, 0]`: ADD action 1, REMOVE action 0, application source indication 1. DesktopRoomie now issues this checked request itself through x11rb, with no new dependency.

Hide still requires **fresh coherent Iconic + Unmapped + HIDDEN**. Off-workspace Show requires fresh no-HIDDEN evidence, the same original body workspace, unchanged user current workspace, and coherent off-workspace not-minimized state: Iconic + Unmapped + no HIDDEN, or another already-approved coherent representation. Already not-minimized off-workspace Show remains a no-op with preservation checks and deferred local placement/input.

These are **WM-directed ClientMessages**, not `ChangeProperty` on `_NET_WM_STATE` or directly writing HIDDEN. They do not authorize workspace movement, activation, MapRaised, SetInputFocus, `_NET_ACTIVE_WINDOW` or re-management. Fresh observation proves success; request acknowledgement does not. Preserve input fencing, layer/workspace preservation and existing failure handling.

At the accepted Stage-F executable, Hide owner is **1000 ms**, Show owner **2500 ms**, Bring Top is still Unsupported with its pre-G **2000 ms** placeholder, caller is **4000 ms**, and geometry confirmation is **150 ms**. The later owner-approved Stage-G plan changes only future Bring Top timing: **3500 ms owner**, **2500 ms visibility cutoff**, up to **1000 ms layer work capped by the owner deadline**, and **5000 ms Bring Top caller**; Hide/Show callers remain 4000 ms. These G values are approved design, not implemented F behavior. All paths retain **no retries and no deadline reset**.

This split is approved from direct Openbox evidence and must not be generalized to all X11 WMs. Other WMs require their own evidence before assuming ADD/REMOVE-HIDDEN semantics. Successful manual requests alone did not establish executable acceptance; the corrected DesktopRoomie paths subsequently passed the separate source/CI and owner native gates below.

## Stage-F acceptance (recorded 2026-10-13)

**Conclusion: A00-M03.2-F ACCEPTED.** This record transcribes the owner's acceptance assignment after independent ChatGPT source/CI review and successful native Openbox checks. It is a Stage-F checkpoint, not final M03.2 acceptance. No Cargo checks or desktop experiments were rerun for this documentation update.

### Revision identities and automated review

| Identity | Role |
| --- | --- |
| `2424c8887d83d1e503caba326cd512d036639c3e` | Original F candidate: source/CI and initial local checks passed, H15 failed as preserved above. This candidate remains unaccepted. |
| `5ed75d7124c10bde0a72a1bc14b4075c09633922` | Substantive owner-approved off-workspace visibility correction: checked ADD/REMOVE-HIDDEN selection, explicit phases and deterministic regressions. |
| `ccd01915bef13850067391f70f1524362a6440dc` | Final **accepted executable**, after correcting only stale CLI help (M03.2 heading, Hide capability, Bring Top still unsupported). This commit did not change the visibility mechanism. |
| [CI 38349035176](https://github.com/zNe4/DesktopRoomie/actions/runs/38349035176) | Matching accepted-executable CI, reported by the owner following independent review. |

ChatGPT independently reviewed the correction and confirmed the exact off-workspace envelopes: ADD `[1, _NET_WM_STATE_HIDDEN, 0, 1, 0]` and REMOVE `[0, _NET_WM_STATE_HIDDEN, 0, 1, 0]`, using checked root-directed SendEvent, `propagate=false`, `SUBSTRUCTURE_NOTIFY | SUBSTRUCTURE_REDIRECT`, body as event.window, `_NET_WM_STATE` as event.type, and format 32. There is no direct `_NET_WM_STATE` property mutation. Off-workspace Show sends neither MapWindow nor a user-time mutation. Unrelated state atoms are preserved; there is no attention-clearing request.

Review confirmed retained local ICCCM Hide and local user-time-zero + MapWindow Show, fixed deadlines (Hide 1000 ms, Show 2500 ms, Bring Top 2000 ms, caller 4000 ms, geometry 150 ms), no retries or resets, the Stage-E scheduling correction, settled input fencing and deferred local input restoration. Attempts are consumed before dispatch; actual dispatch remains recorded after a failed check, and acknowledgement alone is never success. Bring Top remained Failed / Unsupported / Preflight, exit 7.

| Matching CI check | Result |
| --- | --- |
| `cargo fmt --manifest-path experiments/desktop-probe/Cargo.toml --check` | PASS |
| `cargo build --manifest-path experiments/desktop-probe/Cargo.toml` | PASS |
| `cargo clippy --manifest-path experiments/desktop-probe/Cargo.toml --all-targets -- -D warnings` | PASS |
| `cargo test --manifest-path experiments/desktop-probe/Cargo.toml` | **347 passed, 0 failed** |

### Owner native acceptance

Environment: **Openbox 3.6.1 / X11 / Picom v13**. Tested executable: `ccd01915bef13850067391f70f1524362a6440dc`. Observed body XID: **58720258**. The following are owner-reported real-host results, not Codex-injected tests. Automated native drivers and physical/manual interactions are identified separately; no claim is made that all H01–H28 cases were exercised here.

| Gate | Owner procedure and observed result |
| --- | --- |
| **H04 — PASS** | Hide while typing: `HIDE=0`, `FOCUS_BEFORE=50331662`, `FOCUS_AFTER=50331662`. Show afterward succeeded: `show: restored here; original workspace/layer preserved; placement verified, input ready`. Owner reported no typing/focus interruption. |
| **H06 — PASS** | Automated real-host driver completed **20 local + 20 off-workspace Hide/Show cycles, 80 visibility commands total**; all tests passed. Original XID `58720258` was preserved and final local visibility/layer confirmed. No stale input, duplicate body or accumulated failure. |
| **H07 — PASS** | Owner manually hid with popup open and with a non-Quit row pressed. Popup safely disappeared; no leaked/stale menu action or retained capture. Show restored normal menu operation. |
| **H08 — PASS** | Owner manually hid during active left drag and while holding right button/pending OpeningRight. Interaction cancelled safely; no stale final movement, click/menu resurrection or stuck pointer. Restored body remained normally draggable. |
| **H09 — PASS** | Owner selected Normal and fully covered the body with an ordinary window. `HIDE=0`, `SHOW=0`; CLI stayed independently usable. Openbox raised the restored body relative to the covering peer. See the effective-layer/relative-order distinction below. |
| **H15 — PASS** | Off-workspace `--hide` changed Iconic + no HIDDEN into confirmed Iconic + HIDDEN + IsUnMapped, with `HIDE=0`, body desktop 0/current desktop 1 and unchanged focus. Repeated Hide while already minimized also returned `HIDE=0`, preserving coherent minimized state. |
| **H16 — PASS (Show only)** | Without switching workspaces after H15, `--show` returned `SHOW=0`, cleared HIDDEN and left Iconic + IsUnMapped on body desktop 0/current desktop 1. Focus stayed unchanged and no new DEMANDS_ATTENTION appeared. Later manual return restored Normal + IsViewable and working interaction; details below. Bring Top was not tested or accepted by this gate. |
| **H17 — PASS** | H15/H16 preserved body desktop 0 and user/current desktop 1 until the owner manually returned. No caller-workarea displacement was observed. |
| **H20 — PASS** | Owner ran **20 rounds of concurrently launched Hide and Show**. All invocations terminated truthfully; final round included `hide exit=0`, `show exit=0`. Final body search returned exactly `58720258`; state was WM_STATE Normal, `_NET_WM_STATE_ABOVE`, desktop 0. No duplicate, hang, orphan capture or accumulated failure. Busy is permitted, but not required when one fast operation finishes before its competitor is admitted. |
| **H22 — PASS** | Owner ran **20 mixed rounds** of physical drag, popup open/pressed row, pending right-button interaction, Above/Normal/Below selection, popup outside dismissal and remote Hide/Show. Driver completed all rounds. Final state: WM_STATE Normal, `_NET_WM_STATE_ABOVE`, desktop 0, IsViewable, XID `58720258`. Owner confirmed no stuck capture, orphan popup, stale release/action or progressive degradation; normal click/drag and padding click-through remained functional. |

For H15, the initial ordinary off-workspace state was `WM_STATE=Iconic`, no HIDDEN, body desktop 0/current desktop 1. Unlike the original failed candidate, corrected Hide confirmed actual minimization by observing HIDDEN, while remaining unmapped and preserving both desktops and focus.

For H16, the diagnostic was: `restored on another workspace; original workspace/layer preserved; placement deferred, input unavailable; not necessarily visible here`. After the owner **manually** returned to desktop 0, WM_STATE was Normal, the previously selected Above layer was present, body desktop remained 0 and Map State was IsViewable. Vanilla appeared automatically; the owner physically confirmed clicking, dragging, popup operation and transparent-padding click-through. This establishes deferred local input restoration, not input readiness from the remote success itself.

H09 preserves the **effective Above/Normal/Below layer**, not exact relative z-order among peers within Normal. DesktopRoomie issued no MapRaised, `_NET_ACTIVE_WINDOW` or explicit stack operation. Openbox may raise a locally restored mapped client through its own MapRequest presentation behavior. This observation is **not Bring Top** and does not establish a relative peer-order preservation guarantee.

### Accepted scope and next gate

Accepted F scope: managed Hide; same-workspace/current/all-desktop ICCCM minimization; Openbox off-workspace ADD-HIDDEN minimization; retained local Stage-E Show; Openbox off-workspace REMOVE-HIDDEN Show; input fencing and deferred local input restoration; workspace/effective-layer preservation; truthful second-process terminal results. Native observations above establish the exercised Openbox cases; all-desktop and synthetic failure variants remain deterministic evidence unless separately observed.

**At the historical Stage-F acceptance checkpoint**, stages A–F were accepted and [Stage-G design](../DesktopRoomie-A00-M03.2-stage-G-approved-plan.md) with D1/D2 was approved; G1 was next and Bring Top was still unsupported **at that F revision**. H was blocked at that time. This historical record remains separate from the later [Stage-G acceptance](#stage-g-acceptance-recorded-2026-10-13). No final M03.2 acceptance exists. This record claims neither other-WM verification nor Wayland support, and this documentation checkpoint itself authorizes no executable or host-configuration changes.

## Stage-G acceptance recorded 2026-10-13

**Decision: A00-M03.2-G ACCEPTED.** This records owner-provided native Openbox evidence after independent exact-commit source/CI review. It is a Stage-G checkpoint, **not final integrated M03.2 acceptance**. No Cargo checks or live X11 experiments were rerun for this later docs-only update.

### Reviewed revisions and CI

| Checkpoint | Exact revision and matching successful CI | Scope |
| --- | --- | --- |
| G1 | `ba159932d3245fae438a37896cc68623c3dc75b3`; [CI 38434302376](https://github.com/zNe4/DesktopRoomie/actions/runs/38434302376), 367 passed, 0 failed | Existing layer controller gains bounded/structured outcomes and marked dispatch, plus qualified off-workspace remote reader. Bring Top still Unsupported **at G1**. |
| G2 | `96de45d3a236a2cdedb1ec74d1a855470b86fa9a`; [CI 38463014850](https://github.com/zNe4/DesktopRoomie/actions/runs/38463014850), 414 passed, 0 failed | Visibility→Above composition, truthful restoration-proof Partial, async admission, CLI exposure, phase-aware routing and approved timing. |
| **G3 — final accepted executable** | **`80d7df4aa1524122bd7c232f5aa09a4e39c8dd9f`**; [CI 38468930784](https://github.com/zNe4/DesktopRoomie/actions/runs/38468930784), **447 passed, 0 failed** | Additional deterministic failure, lifecycle, transport, pointer, concurrency, deadline and result-invariant hardening, with narrowly tested production fixes. Owner native acceptance followed. |

Accepted G timing: owner Bring Top **3500 ms**, visibility cutoff **admission +2500 ms**, layer deadline `min(layer admission +1000 ms, owner deadline)`, geometry verification **150 ms** inside visibility cutoff, Bring Top caller **5000 ms**; Hide/Show callers remain **4000 ms**. The existing M03.1 layer controller and one remote operation owner are reused; no transport, activation, focus request, generic restack, fullscreen reassertion or retry mechanism was introduced. Logical deadlines assume responsive synchronous X11.

### Owner-observed Openbox matrix

Host: owner's Openbox 3.6.1 / X11 / Picom v13. Native G checks used a single managed body, XID **`56623106`**, on body desktop **0**, with deliberate current desktop **0** or **1**. These XIDs are recorded observations, not portable identifiers.

| Gate | Result | Evidence and scope |
| --- | --- | --- |
| **H10** | **PASS** | Local Bring Top from Normal, Below and the already-Above case: exit **0**; final `WM_STATE Normal`, exact `_NET_WM_STATE_ABOVE`, desktop 0, `IsViewable`, focus before/after **58720270**. Layer selections were made through the owner popup. |
| **H11** | **PASS** | Three genuine local minimized initial layers: `HIDDEN`, `HIDDEN+BELOW`, `HIDDEN+ABOVE`, each `WM_STATE Iconic / IsUnMapped`. Each Hide and subsequent Bring Top returned **0**; all ended `Normal / Above / IsViewable`, desktop 0 with focus unchanged. |
| **H13** | **PASS** | Naturally observed Openbox off-workspace not-minimized representation `Iconic + IsUnMapped + no HIDDEN`: body desktop **0**, current desktop **1**. Bring Top exit **0** yielded Above without HIDDEN or MapState change; body/current desktops and focus **58720270** remained unchanged. This representation is **native evidence**, not an injected variant. |
| **H14** | **N/A — not observed** | The alternate `Normal + Unmapped/Unviewable + no HIDDEN` off-workspace representation did **not** naturally appear. Deterministic test coverage is separate; no fabricated native PASS. |
| **H16** | **PASS** | With a Below body on desktop 0/current 1, remote Hide exit **0** produced `Iconic + IsUnMapped + HIDDEN+BELOW`. Remote Bring Top exit **0** cleared HIDDEN/BELOW and confirmed exact Above while remaining Iconic/Unmapped elsewhere; no workspace transport. |
| **H17** | **PASS** | During remote recovery body stayed on **0**, current desktop stayed **1**, focus stayed **58720270**. Without another recovery command the owner manually returned to desktop 0 and observed `Normal / Above / IsViewable`. CLI said `placement deferred, input unavailable` while elsewhere. |
| **H20** | **PASS** | Twenty concurrent Bring Top + Hide rounds: Bring Top exit **0 Success** or **5 Busy**, Hide exit **0** each round; no hang or OutcomeUnknown in pasted outputs. Final body search yielded only XID `56623106` in `Normal / Above / IsViewable` on desktop 0 after normalization. Either concurrent command may finish first; no fixed winner is required. |
| **H22** | **PASS (owner-reported)** | Owner ran the twenty-round mixed drag/menu/layer/remote exercise and supplied final XID `56623106`, `Normal / Above / IsViewable` on desktop 0. The **individual manual per-round observations were not pasted**; the owner treated the completed exercise as passing. Do not claim individual events were logged. |
| **H25** | **PASS (owner acceptance)** | With an ordinary application fullscreen, Bring Top exit **0** preserved focus **39845892**, desktop 0, and `Normal / Above / IsViewable`. After exiting fullscreen **without invoking Bring Top again**, the same body properties remained. The supplied X11 log verifies focus/state preservation, but does not separately capture relative visual fullscreen overlap; Above never guarantees fullscreen precedence. |

The owner's off-workspace CLI said: `not minimized + Above confirmed on another workspace; original workspace preserved; placement deferred, input unavailable`. Native evidence confirms WM property states for exercised cases, not generic exact peer z-order or visual precedence over fullscreen.

### Selected E/F regressions on the accepted G executable

- **Local Hide/Show:** both returned **0**; Show reported `restored here; original workspace/layer preserved; placement verified, input ready`. Focus remained **58720270**, with body `WM_STATE Normal / Above / IsViewable` on desktop 0.
- **Other-workspace Hide/Show:** from body desktop **0**/current desktop **1**, ordinary Openbox representation was `Iconic + IsUnMapped + Above` without HIDDEN. Hide exit **0** added **HIDDEN** while retaining Above; Show exit **0** removed **only HIDDEN**, remaining `Iconic + Above + IsUnMapped`, body/current desktops unchanged. CLI correctly reported deferred placement and input.
- Earlier Stage-F input, popup and transparent-padding click-through acceptance remains in the [Stage-F record](#stage-f-acceptance-recorded-2026-10-13). Owner declared the final G regression sweep passing; this record does not invent new real-host all-desktops, synthetic failure or monitor-removal checks.

**Boundary/next gate:** accepted Bring Top establishes not-minimized plus freshly confirmed effective **Above on the body's existing workspace**. It does not Bring Here, switch the user's desktop, focus/activate, guarantee caller-workspace visibility, or override fullscreen. Other WMs and Wayland remain unverified. **H is unblocked for separately authorized planning, not yet started or accepted.** Full integrated `ACCEPTANCE-M03.2.md` remains reserved for H; M03.3 Bring Here/Recover remains future. No executable or user Openbox-configuration edits are authorized by this documentation update.

## Stage-H integrated Openbox host evidence (2026-10-18)

**H candidate:** `a46e432c1f97b03c576737b1fc0ac35d7f90c158`, compiled SHA-256 `fdd7863561502773106e36842169cf900a3769dedd2aaa9eede366653e32da2c` in owner preflight, matching [CI 39098551853](https://github.com/zNe4/DesktopRoomie/actions/runs/39098551853) (453 passing tests). Owner host: Openbox 3.6.1/X11 `:0`/Picom v13, eDP-2 + DP-1-0 1920×1080 each, published workarea `10,48,3820,1022`.

- **H-N1 13 gates PASS:** H01–H09, H12, H18–H19, H24. Physical typing/input/occlusion, managed Hide/Show layer preservation, 20 cycles, safe gesture/menu interruptions, real Iconic/no-HIDDEN off-workspace Show and clean SIGKILL relaunch.
- **H21 PASS:** 25 s duration shutdown yielded 17 successes, one truthful exit-8 unknown, 22 no-owner responses; separate manual Quit under 80 Show invocations yielded 56 successes then 24 no-owner results. Both owner exit 0, zero bodies afterward; user input normal.
- **H23 conditional/not exercised:** Polybar 3.7.2 IPC hide/show succeeded as commands but `_NET_WORKAREA` remained `10,48,3820,1022` before, while hidden, and after. Polybar dock reports top strut 38; Plank unaffected. No root-property fabrication or config changes.
- **H26 PASS:** 19 samples over 90 seconds with zero accumulated CPU tick delta at `CLK_TCK=100` in each separate process, visible 8→8 ticks; minimized 2→2. This is below 10-ms counter resolution, not physically zero cycles. Input worked after Show.
- **H27 scoped PASS:** Plank icon persists when minimized; Polybar topbar has no task list. Original managed UTILITY body, layer, desktop and geometry survived Hide/Show. Pager behavior not separately recorded.
- **H14 N/A/unobserved** alternate off-workspace Normal/non-viewable representation; **H28 optional/unexercised** user Openbox shortcut binding.
- Earlier owner G3 H10/H11/H13/H16/H17/H20/H22/H25 and F H15 remain attributed to `80d7df4…` / `ccd0191…`, **not** new H-SHA native tests.

See [H01–H28 matrix](../../experiments/desktop-probe/ACCEPTANCE-M03.2.md#h01h28-integration-matrix) and [M03.2 source design](../../experiments/desktop-probe/M03.2-IMPLEMENTATION.md). H-D itself was documentation-only; final H-A owner acceptance was subsequently recorded on 2026-10-18.

## Configuration levels


| Level | Current guidance |
| --- | --- |
| **Required** | Use the accepted Openbox/X11/Picom host environment and its normal X11/WM facilities. No custom shortcut is required for the executable to behave correctly. |
| **Recommended** | Keep explicit command paths and a safe, user-chosen manual method to invoke accepted operations once enabled. Verify the actual session configuration and test without changing focus policy. |
| **Optional** | Openbox keybindings, menu launchers and startup commands for the accepted CLI operations. Each is installed and maintained by the user, not by DesktopRoomie. |

## User-controlled files and precautions

- **`~/.config/openbox/rc.xml`**: user keyboard bindings in the existing `<keyboard>` section. Some Openbox derivatives/themes use their own session startup or configuration location; inspect the active configuration instead of overwriting a custom file.
- **`~/.config/openbox/menu.xml`**: optional menu launch entries, where that file is in use.
- **`~/.config/openbox/autostart`**: optional startup when using `openbox-session` / an Openbox session that runs these scripts. Invoking `openbox` alone does not guarantee that script runs.
- **`~/.config/openbox/environment`**: optional session environment configuration, only if actually needed; using an absolute CLI path in `Execute` is simpler and less surprising.

Before making any manual change, inspect the existing configuration, preserve a backup and choose key combinations not already in use. Do not replace Archcraft-specific files with stock defaults. Do not change focus policy, configure global keyboard capture in DesktopRoomie or execute configuration edits as part of the program.

## Working manual Show command

With the owner running in the same X11 session, use a second terminal. Point `BIN` at the actual compiled executable:

```sh
BIN=/ABSOLUTE/PATH/TO/desktop-probe
"$BIN" --show
```

Accepted local Show returns success only after fresh readiness evidence. If already visible it is a no-op; on another workspace it does **not** move the body or switch desktops, and local input/placement is deferred. Show cannot raise a covered, already visible Below body. Genuinely minimized off-workspace Show now uses accepted REMOVE-HIDDEN; Hide is also accepted. Accepted Bring Top additionally establishes Above on the existing body workspace without focus/activation or workspace transport; it does not move Vanilla to the caller or override fullscreen. No optional shortcut is necessary.

## Optional keyboard binding examples

Openbox supports `<action name="Execute">` and `<command>` under a `<keybind>` in the `<keyboard>` section. The following examples are **illustrative only**: replace the executable path and check keys for conflicts. Show and Bring Top are accepted; these examples remain optional, user-managed and not smoke-tested by this documentation-only change. Insert a keybind **inside the existing `<keyboard>` element**; do not add a second root `<keyboard>` section.

```xml
<!-- Optional accepted --show shortcut; user must check W-C-F8 -->
<keybind key="W-C-F8">
  <action name="Execute">
    <command>/ABSOLUTE/PATH/TO/desktop-probe --show</command>
  </action>
</keybind>

<!-- Optional accepted --bring-top shortcut; user must check W-C-F9 -->
<keybind key="W-C-F9">
  <action name="Execute">
    <command>/ABSOLUTE/PATH/TO/desktop-probe --bring-top</command>
  </action>
</keybind>
```

After deliberately editing the active `rc.xml`, `openbox --reconfigure` requests a reload of the running Openbox configuration. Verify keyboard behavior in the real session and revert the user change if it conflicts with an existing binding. Openbox `Execute` commands are not shell scripts by default; if shell features are needed, they must be invoked explicitly and safely. Escape XML-special characters in command text.

## Planned CLI semantics and mission ownership

| Candidate operation | Intended meaning | Earliest mission |
| --- | --- | --- |
| `--show` | Restore hidden/iconified body; no implicit workspace/layer change | M03.2 |
| `--hide` | Hide safely while leaving the same instance controllable | M03.2 |
| `--bring-top` | Show if hidden and place Above on the **existing body workspace** | M03.2 |
| `--bring-here` | **Accepted standalone Stage D**: move same body to frozen caller workspace; preserve layer and genuine minimization without focus/user-desktop switching; minimized geometry/input deferred | M03.3-D |
| `--recover` | **Accepted Stage E on this Openbox/X11 host**: frozen-workspace Bring Here followed by local restore/Above and fresh input/geometry proof | M03.3-E |

M03.2 Hide/Show/Bring Top are final-accepted at H. M03.3 A/B/H0/C/D and Stage E are separately accepted, including Bring Here at D4.1 and Recover at E4. Optional keyboard bindings remain user-managed and were not separately smoke-tested for Recover. Stage F/G require separate authorization.

## Stage-D owner-native Bring Here acceptance (2026-10-20)

Frozen executable `4c67da0b09ffe7b69605ac4a415d256acf8abd23`, [CI 39377294762](https://github.com/zNe4/DesktopRoomie/actions/runs/39377294762), 677 tests. Unlike H0's manually instrumented Stage-B feasibility experiment, D4.1 exercised the public `--bring-here` command with real owner/reply routing. Normal/Above/Below and genuine minimized relocation were confirmed, as were uninterrupted typing/focus, same body identity, transparent-padding interaction, already-local gesture cancellation, independent covered/minimized control, repeated successful relocation, duplicate refusal, clean Quit/relaunch and passive state stability. The owner's N08 delayed tests captured an **already-local** workspace at execution time, not remote relocation during a drag; N09 was manual with retained-monitor geometry corroboration. Real malformed-property/monitor-loss/WM-refusal tests N10–N13 were not fully exercised, nor were concurrent/kill/duration native regressions repeated at D4.1. Full [acceptance record](../../experiments/desktop-probe/M03.3-D-ACCEPTANCE.md).

Bring Here and Recover capture the **current workspace when the caller executes** before owner discovery; scheduling a command before switching back does not freeze the earlier workspace. Neither is an automatic-follow feature nor targets the caller terminal's physical monitor. Recover was unavailable at the historical Stage-D executable and became public at accepted E4.

## Stage-E owner-native Recover acceptance (2026-10-21)

Frozen public Recover executable `aa00f8e94aff1cf2ef1ff6a6cecf734f822d5603`, [matching CI 39529051360](https://github.com/zNe4/DesktopRoomie/actions/runs/39529051360), **737 passed / 0 failed**; fmt/build/Clippy PASS. E1–E4 and owner-native N01–N19 passed for exercised cases. The [Stage-E acceptance](../../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md) supplies exact revision identities, per-test findings and evidence limits.

Native coverage: local and remote visible/minimized Normal/Above/Below recovery, historical relocation/restoration, retained body and focus, click-through, popup/drag behavior, and 20 sequential operations. Bounded concurrent N13 no-ops were all successful; genuine remote minimized N14 contention gave **one Success and seven Busy (exit 5)**, with N15 proving post-Busy release. N16 no-owner returned exit 3 without implicit launch; N17 Quit/relaunch, N18 four local variants, and N19 popup/drag interaction plus identical 15-second passive X11 snapshots all passed. The original N13 shell **harness** used zsh's special `commands` parameter and launched runaway jobs; after a reboot a bounded Bash harness passed. This was not an observed DesktopRoomie failure.

Monitor retention and some pointer/focus observations were owner-reported without new RandR topology snapshots. No native hot-plug, induced WM/X11 errors, transport-loss Partial/Unknown, other-WM/Wayland certification or optional binding tests are claimed. Openbox configuration remains user-managed and unchanged.

## Official reference material

- [Openbox bindings](https://openbox.org/help/Bindings) — `rc.xml` keyboard bindings and modifier syntax.
- [Openbox actions](https://openbox.org/help/Actions) — `Execute` commands and XML action conventions.
- [Openbox autostart](https://openbox.org/help/Autostart) — user startup scripts and `openbox-session` distinction.
- [Openbox configuration](https://openbox.org/help/Configuration) — user/system configuration locations.

Hide, Show, original-workspace Bring Top, standalone Bring Here and composed Recover are accepted on the exercised Openbox host at their distinct frozen revisions. Optional bindings remain owner-managed, not independently acceptance-tested. M03.2 H-A, M03.3 H0, Stage D and Stage E evidence retain separate executable identities. [Stage-D acceptance](../../experiments/desktop-probe/M03.3-D-ACCEPTANCE.md); [Stage-E acceptance](../../experiments/desktop-probe/M03.3-E-ACCEPTANCE.md).
