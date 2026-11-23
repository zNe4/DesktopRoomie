# R00-M12 — Spine backend separation decision

The owner approved this repository boundary in R00-M12-S1-B/C. Implementation, runtime adoption and private asset inspection each require explicit authorization.

## Repository boundary

- DesktopRoomie remains independently licensed and must build and operate without a Spine runtime or an installed extension.
- DesktopRoomie-Spine is a separate, initially private repository for Spine-specific runtime integration, skeleton evaluation, geometry extraction, applicable runtime corrections and adapter code.
- Dependencies flow from DesktopRoomie-Spine to DesktopRoomie. The public application must have neither a mandatory dependency nor an unresolved optional path dependency on the extension.
- Local development uses the ignored relative symlink `extensions/spine` → `../../DesktopRoomie-Spine`. Keep it outside Git tracking and do not add a submodule.
- Start with a separate extension build entry point. Dynamic plugins, a stable ABI, IPC and generalization beyond the approved integration are outside scope.
- DesktopRoomie owns general animation decisions, interaction, movement and window management.

## Distribution and assets

The planned distribution model is individual-user licensing, with installation by cloning the extension repository subject to the terms applicable to the runtime version and distribution arrangement. Character asset licensing and redistribution are separate. Authorized users supply character assets independently, and both repositories exclude those assets.

## Esoteric Software licensing clarification — 2026-11-23

The owner shared a direct written reply from Misaki at Esoteric Software LLC on November 23, 2026. This record relies on the owner-supplied correspondence. Independent public authentication was outside the scope of this update.

- The changes in the November 14, 2026 Spine Runtimes License Agreement apply starting with Spine 5.0. Spine 4.1 remains subject to its applicable runtime license.
- Esoteric confirmed that the proposed optional backend approach is permitted. DesktopRoomie users who do not use the backend need no Spine Editor license. Users who use the backend must hold a valid Spine Editor license.
- Backend distributions must include the required Spine Runtimes license and copyright notices. Maintainers only need to inform users clearly of the license requirement. Users are responsible for compliance, and no monitoring or independent license verification is required.

The confirmation is scoped to the proposed optional backend. Redistribution remains subject to the terms for the specific runtime version and distribution arrangement. Esoteric invited follow-up if “separately maintained backend” means a different arrangement, so the exact repository arrangement can be clarified further if needed. Historical runtime evaluation permissions remain unconfirmed. Technical acceptance of the backend and runtime integration remains separate.

## Evidence and next objective

- The historical `rusty_spine = "=0.7.2"` evaluation and six-edit correction remain experimental. R00-M12-T1-B2 retains **PARTIAL** status.
- The recovered D6-F2 report records a bounded structural **PASS** and exercise of the active linked-parent mesh feature. Complete runtime qualification and visual acceptance remain outstanding.
- The accepted R00-M13-P2-C2-C2 renderer checkpoint is immutable. Future renderer work must use a separately authorized isolated derivative. [C2-C2 acceptance](../renderer/R00-M13-P2-C2-C2/C2-C2-ACCEPTANCE.md)
- The next major technical objective is to render an actual privately supplied Spine character pose through wgpu, subject to separate authorization and the applicable runtime and character asset terms.
- Hina development remains independent of Spine and follows usable Vanilla integration at A01 C5. [A00/A01 plan](../DesktopRoomie-A00-A01-plan-v0.1.md)
