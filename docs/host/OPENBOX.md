# Openbox host integration — user-managed setup

**Status (2026-10-10):** M03.2 stages A–D are accepted. Independent second-invocation control CLI now exists: `--hide`, `--show` and `--bring-top` are parsed and transported, but a Ready owner returns `Failed / Unsupported / Preflight` (exit 7). Actual visibility/recovery behavior is not enabled. [STATUS](../STATUS.md) records exact executable and acceptance evidence; Stage E is next and has not started. This page is not a mandatory installation procedure and does not authorize code or host configuration edits.

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

## Future keyboard binding example — not active yet

Openbox supports `<action name="Execute">` and `<command>` under a `<keybind>` in the `<keyboard>` section. The following is **illustrative only**: the executable path must be replaced, the key combination checked for conflicts, and the actual `--bring-top` operation enabled and accepted first. Insert a keybind **inside the existing `<keyboard>` element**; do not add a second root `<keyboard>` section.

```xml
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

The M03.2 command spellings and transport are implemented under the approved design; their visibility/layer operations remain unimplemented. M03.3 commands remain future design scope. Optional Openbox bindings are user-managed and must not be presented as working recovery shortcuts before their operations are accepted.

## Official reference material

- [Openbox bindings](https://openbox.org/help/Bindings) — `rc.xml` keyboard bindings and modifier syntax.
- [Openbox actions](https://openbox.org/help/Actions) — `Execute` commands and XML action conventions.
- [Openbox autostart](https://openbox.org/help/Autostart) — user startup scripts and `openbox-session` distinction.
- [Openbox configuration](https://openbox.org/help/Configuration) — user/system configuration locations.

Once the actual M03.2 visibility/recovery operations have passed owner host acceptance, update this guide with the verified executable name/path, concrete working commands, an optional keyboard-binding smoke test and any target-specific configuration differences. Once M03.3 is accepted, extend it with Bring Here and combined Recover examples.