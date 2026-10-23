# A00-M03.2 — integrated verification and owner-native acceptance evidence

**Final owner decision — 2026-10-18: A00-M03.2 ACCEPTED.** The owner explicitly approved H-A after H-N1/N2/N3 real-host verification and H-D publication ([documentation commit `0e9f1f1…`](https://github.com/zNe4/DesktopRoomie/commit/0e9f1f1a4ec9d0f02e45cbf03312da088d4d1916)). Accepted executable remains `a46e432c1f97b03c576737b1fc0ac35d7f90c158` with [CI 39098551853](https://github.com/zNe4/DesktopRoomie/actions/runs/39098551853) (**453 passing tests**). H14's alternate host state was not naturally observed, H23's real workarea-change native test was conditional/not exercised, and H28 Openbox shortcuts were optional/not exercised; none is promoted to PASS. Historical Stage F and G native results retain their distinct code identities. This documentation-only closure neither changes the executable nor constitutes a fresh CI or host test. **Next: M03.3 read-only planning for Bring Here and composed Recover; no implementation authority implied.**

## Identity and proof boundaries

| Subject | Verified evidence |
| --- | --- |
| Branch | `zNe4/DesktopRoomie` / `m03` |
| Frozen owner-tested H executable | `a46e432c1f97b03c576737b1fc0ac35d7f90c158`; preflight local/remote SHA match, clean worktree, owner binary SHA-256 `fdd7863561502773106e36842169cf900a3769dedd2aaa9eede366653e32da2c` |
| Matching H CI | [39098551853](https://github.com/zNe4/DesktopRoomie/actions/runs/39098551853): fmt/build/Clippy PASS, **453 tests passed, 0 failed**. H-C1/H-C2 review and deterministic corrections accepted prior to real host. |
| H native owner host | Archcraft/Openbox 3.6.1/X11 `DISPLAY=:0`/Picom v13, two 1920×1080 displays (`eDP-2` at +0+0, `DP-1-0` at +1920+0); root workarea `10,48,3820,1022`; owner-run commands and firsthand physical observations. |
| Previously accepted Stage G | G3 `80d7df4aa1524122bd7c232f5aa09a4e39c8dd9f` / [CI 38468930784](https://github.com/zNe4/DesktopRoomie/actions/runs/38468930784), **447** passing tests. Its owner-native checks below were **not rerun on H SHA**. |
| Previously accepted Stage F | `ccd01915bef13850067391f70f1524362a6440dc` / [CI 38349035176](https://github.com/zNe4/DesktopRoomie/actions/runs/38349035176), **347** passing tests; original off-workspace-Hide failed candidate remains historically unaccepted. |

The reviewer analyzed real owner logs and owner observations; the reviewer did not run real Openbox tests. Local evidence ledgers/runbooks and raw owner `/tmp/desktoproomie-h-n1.NU0I7F` outputs informed this transcription but are **not** asserted to be committed files. Future docs-only commit SHAs are distinct from the tested executable and from matching CI.

## H01–H28 integration matrix

**PASS-H** means native host gate on frozen H SHA; **PASS-G3** and **PASS-F** designate older owner-tested executables with preserved provenance.

| Gate | Result | Scope / evidence |
| --- | --- | --- |
| H01 | **PASS-H** | Owner typed, including held key, during launch; no focus theft. |
| H02 | **PASS-H** | `--delay` Starting had zero bodies; duplicate owner exit 4; eventual one body. Delayed snapshots from different runs. |
| H03 | **PASS-H** | Hide/Show/Bring Top during Starting exited 5; no appearance, workspace or focus mutation. |
| H04 | **PASS-H** | Hide while typing confirmed Iconic/HIDDEN/unmapped and no observed focus theft; Show restored. |
| H05 | **PASS-H** | Hide/Show for each Normal/Above/Below preserves original body, desktop, focus and exact effective layer. |
| H06 | **PASS-H** | 20 Hide/Show cycles, 40 successful commands, same single body, normal post-cycle input. |
| H07 | **PASS-H** | Open popup and held non-Quit row interrupted by Hide; no leaked menu or action, no stuck capture. |
| H08 | **PASS-H** | Hide during left drag and pending right OpeningRight; no stale drag, click, popup or release. |
| H09 | **PASS-H** | CLI Hide/Show works with body fully occluded; exact exit-code transcript not pasted for this single gate. |
| H10 | **PASS-G3** | Bring Top from Normal/Below/Above, Above confirmed without activation. |
| H11 | **PASS-G3** | Genuine minimized Normal/Below/Above → restored Above. |
| H12 | **PASS-H** | Body desktop 0/current 1: Iconic/Unmapped/no HIDDEN; Show exit 0 preserves body and workspace without restoration. Manual return to 0 restored interactive Normal/Viewable without another Show; detailed Show diagnostic not supplied. |
| H13 | **PASS-G3** | Naturally observed Iconic/Unmapped/no-HIDDEN off-workspace Bring Top, no relocation. |
| H14 | **N/A — not observed** | Alternate other-desktop Normal/non-viewable representation did not occur on Openbox. Pure tests are not native evidence. |
| H15 | **PASS-F; G3 regression** | Corrected Hide elsewhere genuinely adds HIDDEN, not just Iconic/unmapped. Earlier failed F candidate remains failed. |
| H16 | **PASS-F/G3** | Off-workspace actual-minimized Show or Bring Top clears HIDDEN without moving desktop; local input deferred until return. |
| H17 | **PASS-F/G3** | Body/current desktops and focus preserved; no caller-workarea displacement. |
| H18 | **PASS-H** | No owner; three control commands exit 3; no body created. |
| H19 | **PASS-H** | SIGKILL owner, release stale ownership/body, fresh launch and normal click/drag/popup/Quit. |
| H20 | **PASS-G3** | 20 concurrent Hide + Bring Top rounds, bounded success or Busy, no duplicate, queue or stuck capture. |
| H21 | **PASS-H** | Duration expiry and popup Quit under remote command streams; truthful bounded exits, body cleanup, normal input (details below). |
| H22 | **PASS-G3 (owner reported)** | 20 mixed interaction/remote rounds; owner reported passing with stable final body. Individual round actions not separately logged. |
| H23 | **CONDITIONAL — NOT EXERCISED** | No demonstrated safe real reversible `_NET_WORKAREA` change while minimized; IPC bar hide/show left workarea constant. Deterministic geometry tests are separate. |
| H24 | **PASS-H** | Physical click-through padding, interactive translucent patch, drag offset, right cancel and popup outside dismissal. |
| H25 | **PASS-G3** | Above with fullscreen application and return, no focus/reassertion workaround; does not prove precedence over fullscreen. |
| H26 | **PASS-H** | Visible/minimized 90 s CPU measurements each unchanged in 19 sampled kernel tick readings; click/drag still work after restore. |
| H27 | **PASS-H (scoped observation)** | Plank retains icon while minimized; Polybar topbar has no task list. No pager behavior observed; UTILITY body retained. |
| H28 | **OPTIONAL — NOT EXERCISED** | Owner-managed Openbox keyboard Execute shortcut smoke omitted; CLI independent of shortcuts. |

**H-N1**: H01–H09, H12, H18–H19, H24 (**13 native passes**). **H-N2**: H21 PASS, H23 conditional/unexercised. **H-N3**: H26 PASS, H27 host-representation observation PASS. The G3/F rows above do not assert that all H01–H28 were tested on one SHA.

## Lifecycle and input — H21

For owner `--duration 25`, the first preliminary run sent 24 already-visible Show requests, all succeeded but did **not** cross expiry. The targeted 40-command Hide/Show run returned **17 success exit 0**, then at 04:31:14.724 one `--show` returned **exit 8**, correctly reporting outcome unknown because owner disappeared before a terminal reply, then **22 exit 3**, no owner. Owner exit 0; final body count 0.

For menu Quit, an owner launched with `--duration 90` was closed by **`[MENU] Quit selected`** while 80 Show requests were running. **56 successes**, then **24 exit 3/no owner**. Owner logged clean exit, process exit 0, final body count 0, and normal physical keyboard/mouse afterward. No command happened to catch a Closing exit 5; no exact shutdown scheduler interleaving is invented. Before/after active-window IDs changed in this experiment because the owner manually changed focus, not due to the probe.

## Workarea and idle/taskbar observations

**H23:** Polybar 3.7.2 IPC is enabled. Its top dock window reported `_NET_WM_STRUT=0,0,38,0`, while Plank ran independently. `polybar-msg` hide and show both succeeded as commands, but `_NET_WORKAREA` stayed `10,48,3820,1022` before, while hidden, and after restoring. Thus there was no verified real changed workarea and **no native minimized-body H23 run**. No direct root-property mutations or panel configuration edits were made; visual panel restoration was not separately reported. Source/injected correction coverage is not an H23 native PASS.

**H26:** Visible PID `1768393`: 19 `/proc/PID/stat` user+system tick samples, 90 seconds, `CLK_TCK=100`, all `8`. Minimized *different PID* `1815045`: 19 samples, 90 seconds, all `2`. Both sampled **0 tick delta** at 10-ms granularity, not literal proof of zero CPU cycles; absolute ticks from distinct processes are not compared. Minimized snapshots confirmed Iconic + HIDDEN + IsUnMapped; Show restored interaction. Layer/motion activity appeared between visible before/after snapshots, not individually timed relative to sampling.

**H27:** Owner observed a Plank icon even when minimized and no Polybar task list; a workspace pager was not separately observed. Minimized body XID `62914562`, desktop 0, geometry (495,388) 160×160, `Iconic`, `HIDDEN+ABOVE`, `IsUnMapped`; restored same XID/desktop/geometry with `Normal`, `Above`, `IsViewable` and unchanged active window across these two snapshots. No skip-taskbar/pager policy was changed.

## Stop conditions, exclusions, and next decision

No native H test identified an executable correction. Openbox evidence is not proof for other WMs, Wayland, fabricated X-server failure, real monitor removal, or H14's alternate state. Request acknowledgement alone is not WM confirmation. Logical deadlines assume responsive synchronous X11; Bring Top is Above on the **existing** workspace, not Bring Here/Recover or guaranteed visual precedence over fullscreen.

**Final disposition:** H-D was published as [documentation commit `0e9f1f1…`](https://github.com/zNe4/DesktopRoomie/commit/0e9f1f1a4ec9d0f02e45cbf03312da088d4d1916); formal H-A owner sign-off followed on 2026-10-18. **A00-M03.2 is ACCEPTED** on the exercised Openbox host with the recorded H14/H23/H28 qualifications and cross-revision provenance intact. Owner acceptance creates no new code/CI result and does not authorize M03.3 implementation. See [implementation](M03.2-IMPLEMENTATION.md), [STATUS](../../docs/STATUS.md) and [Openbox guide](../../docs/host/OPENBOX.md).
