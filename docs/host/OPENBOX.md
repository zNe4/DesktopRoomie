# Openbox host integration — user-managed setup

**Status (2026-10-10):** stages A–E accepted. Independent `--show` restores an externally minimized managed body, owner-tested on dual/differing monitors with retained main-monitor placement, no focus/layer/workspace drift and preserved input/click-through. Accepted executable `174d8a92574e16768330fb3c315578b105e4b698`. Ready `--hide` and `--bring-top` still return `Failed / Unsupported / Preflight` (exit 7). [STATUS](../STATUS.md) records exact evidence; F is not authorized. This guide does not authorize host-config edits.

DesktopRoomie should own safe same-instance control, WM-state verification, input cleanup and keyboard-focus isolation. Openbox should be reused for *optional* user conveniences such as keyboard shortcuts, application menus and session autostart. The application must be recoverable from its CLI without optional Openbox bindings; no global key-grabbing daemon, tray framework or automatic dotfile rewriting is required.

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

Show returns success only after fresh readiness evidence. If already visible it is a no-op; on another workspace it does **not** move the body or switch desktops, and local input/placement is deferred. Show cannot raise a covered, already visible Below body. Hide/Bring Top remain Unsupported. No optional shortcut is necessary.

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

M03.2 command spellings and transport are implemented; only Show's real visibility operation has passed acceptance. Hide/Bring Top and M03.3 remain future gated work. Optional bindings stay user-managed; only accepted operations should be recommended as working shortcuts.

## Official reference material

- [Openbox bindings](https://openbox.org/help/Bindings) — `rc.xml` keyboard bindings and modifier syntax.
- [Openbox actions](https://openbox.org/help/Actions) — `Execute` commands and XML action conventions.
- [Openbox autostart](https://openbox.org/help/Autostart) — user startup scripts and `openbox-session` distinction.
- [Openbox configuration](https://openbox.org/help/Configuration) — user/system configuration locations.

Once Hide and Bring Top pass owner acceptance, extend the examples and record any actual shortcut smoke test (none is claimed for E). After M03.3 acceptance, add Bring Here and Recover.
