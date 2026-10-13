# Openbox host integration — user-managed setup

**Status (recorded 2026-10-13):** stages A–F are accepted at latest executable `ccd01915bef13850067391f70f1524362a6440dc`. The original failed candidate and manual experiments remain below, separate from the [corrected Stage-F acceptance](#stage-f-acceptance-recorded-2026-10-13). The owner-approved [Stage-G plan](../DesktopRoomie-A00-M03.2-stage-G-approved-plan.md) is ready for separately authorized G1/G2/G3 implementation, with G1 next; Bring Top remains Unsupported (exit 7) in the accepted executable. H remains blocked on G acceptance; no final M03.2 acceptance exists. [STATUS](../STATUS.md) records the checkpoint. This guide authorizes neither executable work nor host-config edits.

DesktopRoomie should own safe same-instance control, WM-state verification, input cleanup and keyboard-focus isolation. Openbox should be reused for *optional* user conveniences such as keyboard shortcuts, application menus and session autostart. The application must be recoverable from its CLI without optional Openbox bindings; no global key-grabbing daemon, tray framework or automatic dotfile rewriting is required.

## Openbox off-workspace visibility amendment (2026-10-10)

This historical amendment preserves owner-supplied real-host evidence and the owner-approved correction design for **Openbox 3.6.1/X11/Picom**. At the 2026-10-10 checkpoint, A–E were accepted at `174d8a92574e16768330fb3c315578b105e4b698`, and F awaited correction/review/native acceptance. The amendment was subsequently implemented in `5ed75d7…` and accepted at `ccd0191…`, as recorded separately below. The original failed gate and manual experiments are not retroactively executable passes. No final M03.2 acceptance exists.

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

Stages **A–F are accepted**. The [Stage-G plan](../DesktopRoomie-A00-M03.2-stage-G-approved-plan.md) and D1/D2 timing are owner-approved; **G1 is the next separately authorized implementation checkpoint**, followed by gated G2/G3. Bring Top remains unsupported in the accepted executable; **H remains blocked on G acceptance** and no final M03.2 acceptance exists. This record claims neither other-WM verification nor Wayland support, and this documentation checkpoint itself authorizes no executable or host-configuration changes.

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

Accepted local Show returns success only after fresh readiness evidence. If already visible it is a no-op; on another workspace it does **not** move the body or switch desktops, and local input/placement is deferred. Show cannot raise a covered, already visible Below body. Genuinely minimized off-workspace Show now uses accepted REMOVE-HIDDEN; Hide is also accepted. Bring Top remains Unsupported. No optional shortcut is necessary.

## Optional keyboard binding examples

Openbox supports `<action name="Execute">` and `<command>` under a `<keybind>` in the `<keyboard>` section. The following examples are **illustrative only**: replace the executable path and check keys for conflicts. Show is accepted; Bring Top remains a future example. Insert a keybind **inside the existing `<keyboard>` element**; do not add a second root `<keyboard>` section.

```xml
<!-- Optional accepted --show shortcut; user must check W-C-F8 -->
<keybind key="W-C-F8">
  <action name="Execute">
    <command>/ABSOLUTE/PATH/TO/desktop-probe --show</command>
  </action>
</keybind>

<!-- FUTURE RECOVERY EXAMPLE: --bring-top currently returns Unsupported on Ready -->
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
| `--bring-here` | Move the **same instance** to the current workspace, preserving its layer; hidden-state handling to be designed | M03.3 |
| `--recover` | Explicitly compose Bring Here and Bring Top, with truthful partial-failure reporting | M03.3 |

M03.2 command spellings and transport are implemented; Hide and local/off-workspace Show have passed acceptance at `ccd0191…`. Bring Top remains Unsupported in that executable; its owner-approved G plan proceeds only through separately authorized G1/G2/G3 checkpoints, with G1 next. M03.3 remains future work. Optional bindings stay user-managed; only accepted operations should be recommended as working shortcuts.

## Official reference material

- [Openbox bindings](https://openbox.org/help/Bindings) — `rc.xml` keyboard bindings and modifier syntax.
- [Openbox actions](https://openbox.org/help/Actions) — `Execute` commands and XML action conventions.
- [Openbox autostart](https://openbox.org/help/Autostart) — user startup scripts and `openbox-session` distinction.
- [Openbox configuration](https://openbox.org/help/Configuration) — user/system configuration locations.

Hide and Show are accepted; optional binding changes and shortcut smoke tests remain owner-managed and are not claimed by this record. Once Bring Top passes owner acceptance, extend its example guidance. After M03.3 acceptance, add Bring Here and Recover.
