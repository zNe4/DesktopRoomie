# Openbox host integration — user-managed setup

**Status (2026-10-10):** stages A–E remain accepted at executable `174d8a92574e16768330fb3c315578b105e4b698`. Stage-F candidate `2424c8887d83d1e503caba326cd512d036639c3e` passed source/CI review and initial same-workspace checks but failed H15/off-workspace Hide; it is **not accepted**. Stage G remains blocked. The owner approved the host amendment below; this documentation checkpoint implements no correction and authorizes no executable work. [STATUS](../STATUS.md) records the checkpoint. This guide does not authorize host-config edits.

DesktopRoomie should own safe same-instance control, WM-state verification, input cleanup and keyboard-focus isolation. Openbox should be reused for *optional* user conveniences such as keyboard shortcuts, application menus and session autostart. The application must be recoverable from its CLI without optional Openbox bindings; no global key-grabbing daemon, tray framework or automatic dotfile rewriting is required.

## Openbox off-workspace visibility amendment (2026-10-10)

This is owner-supplied real-host evidence and an owner-approved future correction design for **Openbox 3.6.1/X11/Picom**. It is not a final Stage-F acceptance record. The latest accepted executable remains `174d8a92574e16768330fb3c315578b105e4b698` until a corrected F executable passes independent review, matching CI and owner native acceptance. No final M03.2 acceptance exists.

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

2. **Existing Show failed after that manual Hide:** the current Stage-E checked user-time-zero + MapWindow mechanism returned **`SHOW=7`** for the genuinely minimized off-workspace body:

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

### Approved request matrix — future narrow F correction

The semantic contract remains **Hide = genuinely minimized; Show = not minimized**. The host adapter needs distinct primitives for local and off-workspace state:

| State | Hide request | Show request when genuinely minimized |
| --- | --- | --- |
| Current workspace / all desktops | Retain checked ICCCM `WM_CHANGE_STATE → IconicState` | Retain accepted checked `_NET_WM_USER_TIME=0` + MapWindow |
| Other workspace | Checked root-directed EWMH `_NET_WM_STATE`, ADD HIDDEN | Checked root-directed EWMH `_NET_WM_STATE`, REMOVE HIDDEN; **no MapWindow** |

For the EWMH path, use `SendEvent(false, body_root, SUBSTRUCTURE_NOTIFY | SUBSTRUCTURE_REDIRECT, ClientMessage)` with `window=body`, `type=_NET_WM_STATE`, format 32, and data `[action, _NET_WM_STATE_HIDDEN, 0, application_source, 0]`: ADD action 1, REMOVE action 0, application source indication 1. DesktopRoomie must later issue this checked request itself through x11rb, with no new dependency.

Hide still requires **fresh coherent Iconic + Unmapped + HIDDEN**. Off-workspace Show requires fresh no-HIDDEN evidence, the same original body workspace, unchanged user current workspace, and coherent off-workspace not-minimized state: Iconic + Unmapped + no HIDDEN, or another already-approved coherent representation. Already not-minimized off-workspace Show remains a no-op with preservation checks and deferred local placement/input.

These are **WM-directed ClientMessages**, not `ChangeProperty` on `_NET_WM_STATE` or directly writing HIDDEN. They do not authorize workspace movement, activation, MapRaised, SetInputFocus, `_NET_ACTIVE_WINDOW` or re-management. Fresh observation proves success; request acknowledgement does not. Preserve input fencing, layer/workspace preservation and existing failure handling.

No deadline amendment: Hide owner **1000 ms**, Show owner **2500 ms**, Bring Top **2000 ms pending Stage G review**, caller **4000 ms**, geometry confirmation **150 ms**. The EWMH request must fit the existing fixed operation deadline: **no retries and no deadline reset**.

This split is approved from direct Openbox evidence and must not be generalized to all X11 WMs. Other WMs require their own evidence before assuming ADD/REMOVE-HIDDEN semantics. The corrected DesktopRoomie paths, remaining host cases and regressions are still untested; successful manual requests do not establish executable acceptance. **Stage F requires a narrow correction and remains pending; Stage G remains blocked.**

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

Accepted local Show returns success only after fresh readiness evidence. If already visible it is a no-op; on another workspace it does **not** move the body or switch desktops, and local input/placement is deferred. Show cannot raise a covered, already visible Below body. Genuinely minimized off-workspace Show requires the pending correction above. Hide is enabled only in the unaccepted F candidate; Bring Top remains Unsupported. No optional shortcut is necessary.

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

M03.2 command spellings and transport are implemented; only Show's real visibility operation has passed acceptance. The F candidate enables Hide but failed off-workspace acceptance; its correction, Bring Top and M03.3 remain gated work. Optional bindings stay user-managed; only accepted operations should be recommended as working shortcuts.

## Official reference material

- [Openbox bindings](https://openbox.org/help/Bindings) — `rc.xml` keyboard bindings and modifier syntax.
- [Openbox actions](https://openbox.org/help/Actions) — `Execute` commands and XML action conventions.
- [Openbox autostart](https://openbox.org/help/Autostart) — user startup scripts and `openbox-session` distinction.
- [Openbox configuration](https://openbox.org/help/Configuration) — user/system configuration locations.

Once Hide and Bring Top pass owner acceptance, extend the examples and record any actual shortcut smoke test (none is claimed for E). After M03.3 acceptance, add Bring Here and Recover.
