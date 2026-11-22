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

Future users may install the extension by cloning its repository, subject to applicable licensing conditions. The intended distribution model is individual-user licensing. Ordinary users must obtain the appropriate Spine Editor license under that route. Private repository visibility does not exempt users or maintainers from the applicable terms.

Historical runtime evaluation permissions, redistribution terms and license acknowledgment requirements remain unresolved pending confirmation. Legal clearance remains pending. Authorized users supply character assets independently. Neither repository may bundle those assets.

## Evidence and next objective

- The historical `rusty_spine = "=0.7.2"` evaluation and six-edit correction remain experimental. R00-M12-T1-B2 retains **PARTIAL** status.
- The recovered D6-F2 report records a bounded structural **PASS** and exercise of the active linked-parent mesh feature. Complete runtime qualification and visual acceptance remain outstanding.
- The accepted R00-M13-P2-C2-C2 renderer checkpoint is immutable. Future renderer work must use a separately authorized isolated derivative. [C2-C2 acceptance](../renderer/R00-M13-P2-C2-C2/C2-C2-ACCEPTANCE.md)
- The next major technical objective is to render an actual privately supplied Spine character pose through wgpu, subject to separate authorization and confirmation of applicable licensing conditions.
- Hina development remains independent of Spine and follows usable Vanilla integration at A01 C5. [A00/A01 plan](../DesktopRoomie-A00-A01-plan-v0.1.md)
