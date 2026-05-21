# Cowork GUI Strategy

Date recorded: 2026-05-21

Status: selected for Phase 1

Bead: `cowork-1rz.1.6`

## Decision

Selected: keep the existing **BitFun Tauri** desktop UI as the first Cowork GUI client while the daemon boundary is established.

This is a Phase 1 integration decision, not the final Phase 10 workbench design. The GUI must be treated as a replaceable client of the daemon. Objective, task, evidence, context, browser-session, provider, and artifact authority must move to the daemon/API boundary rather than becoming owned by React components, the Tauri shell, or any external desktop shell.

## Rationale

The conservative Phase 1 path is to preserve the already-buildable BitFun desktop path and spend the first engineering risk budget on the daemon boundary, protocol/versioning, task/evidence export, `SubmitReport`, and compaction/resume gates.

The planning decision already selects a BitFun-derived headless Rust durability daemon with a swappable GUI. The first GUI should therefore minimize new UI/runtime uncertainty until the daemon has authoritative state and a stable local API. BitFun's current Tauri UI is the only candidate already present in this implementation fork and already tied to BitFun's Rust/Tauri/React structure.

Build/launch smoke evidence reference:

- `/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/receipts/G1_BITFUN_BUILD_LAUNCH.md`
  - `pnpm install --frozen-lockfile` passed with explicit Homebrew Node 26.
  - `pnpm run desktop:build:fast` built `bitfun-desktop`.
  - A bounded launch smoke reached `BitFun Desktop started successfully`, stayed alive for 15 seconds, terminated the spawned child PID, and left no `bitfun-desktop` process behind.
- `/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/receipts/G8_RESOURCE_PACKAGING_PROFILE.md`
  - BitFun launched from the built Tauri binary and idled near `166.6-166.7 MB` RSS in the short local profile.
  - OpenCowork's Electron process group idled near `688.9-713.7 MB` RSS in the same probe.
  - AionUi was not relaunched in G8 because G1 showed dev launch isolation risk for Electron `userData`.

## Compared Options

| Option | Phase 1 fit | Integration latency | Proof surface | Phase 10 migration cost | Decision |
|---|---|---:|---|---|---|
| Keep BitFun Tauri UI | Best fit. It is already in the fork and aligned with BitFun's Rust workspace plus shared React frontend. | Low | Existing G1 build/launch smoke and G8 runtime profile. New proof can focus on daemon APIs and state ownership. | Medium. The UI can later be narrowed, replaced, or evolved once daemon contracts are stable. | **Selected** |
| Build a minimal new workbench | Clean conceptual shape, but it creates new UI surface before the daemon contract is proven. | Medium | No existing build/launch smoke for a Cowork workbench in this fork. It would need fresh routing, app shell, packaging, and API integration receipts. | Low-to-medium if kept tiny, but likely to grow into throwaway scaffolding before Phase 10 requirements are clear. | **Rejected for Phase 1** |
| Adapt an external UI from AionUi/OpenWork/OpenCowork | Useful reference material for provider UX, artifact/workspace UX, worker views, plan mode, and approvals. | High | AionUi has product-shell and packaging strengths but G8 left runtime profiling blocked by dev `userData` isolation risk. OpenWork is approved as UX/artifact source but is not the durability kernel. OpenCowork is a subagent/control-flow reference, not the selected GUI base. | High. Early adaptation would create a second shell before daemon state authority is proven, then require another migration after Phase 10 contracts mature. | **Rejected for Phase 1; retained as reference** |

## Rejected Paths

### Minimal New Workbench

Rejected for Phase 1 because it moves effort from daemon boundary proof into greenfield UI construction. A minimal workbench would still need session/objective/task/evidence panels, protocol wiring, launch scripts, packaging behavior, and smoke receipts before it could replace the existing BitFun UI. That expands the proof surface while the real Phase 1 question is whether the BitFun-derived daemon can own state cleanly.

This path may become appropriate after the daemon exposes stable task/evidence/objective APIs and the acceptance target is a thin UI compatibility client rather than a speculative first shell.

### Adapted External UI: AionUi / OpenWork / OpenCowork

Rejected for Phase 1 because external UI adaptation front-loads product-shell migration before the daemon boundary exists.

- AionUi remains the provider/product-shell and packaging reference, but current evidence does not make it the first GUI client. G8 records that AionUi was not relaunched because earlier launch evidence wrote Electron `userData` outside the isolated profile.
- OpenWork remains an artifact/workspace/CDP UX reference and a legally approved source for selective reuse once legal metadata is recorded, but it should not own Cowork task/evidence/context state.
- OpenCowork remains the worker/subagent/plan-approval UX and control-flow reference, but the selected architecture reimplements its strongest semantics in the BitFun-derived daemon instead of adopting its Electron shell as the first client.

These are rejected as Phase 1 GUI bases, not rejected as design sources.

## Operating Rules For The Selected Path

- The BitFun Tauri UI is a temporary first client, not the authority model.
- New daemon state must be reachable through a local protocol/API before UI-specific state is treated as product behavior.
- React/Tauri UI changes should read and display daemon state; they should not become the canonical task, evidence, context, subagent, provider, browser, or artifact store.
- Any UI feature that cannot survive GUI disconnect/reconnect belongs behind the daemon boundary before Phase 10 claims are made.
- AionUi, OpenWork, and OpenCowork patterns should be imported only after the daemon contracts define what data and lifecycle events the GUI consumes.

## Residual Gaps

- This decision reuses validation-sprint smoke evidence; no new build or GUI launch was run in this task because the requested verification mode is inspection/static checks only.
- The referenced G1 BitFun smoke was recorded against an earlier validation checkout, while `docs/COWORK_BASELINE.md` records the implementation fork baseline separately. The baseline document points back to the same G1 receipt as the current build/launch evidence reference.
- Phase 10 still needs a real workbench design and runtime verification: disconnect/reconnect, daemon-sourced task/evidence/artifact views, approval persistence, manual-login handoffs, and GUI replacement invariance.
- AionUi needs a safe packaged-mode or fixed-`userData` runtime profile before its desktop runtime behavior can be used as implementation evidence.
- Windows GUI build/runtime smoke remains unverified.
