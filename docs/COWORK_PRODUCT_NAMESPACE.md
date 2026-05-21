# Cowork Product Namespace

Date recorded: 2026-05-21

This file records the Phase 1 namespace decision for the Cowork product fork.
It is a staged identity map only. It does not rename packages, crates, binaries,
Tauri app identifiers, launch scripts, bundle assets, or persisted state paths.

## Decision

The product namespace is **Cowork**.

Use `cowork` as the product slug for new Cowork-owned code, documentation,
daemon/protocol surfaces, and future package names. Use `com.jroth.cowork` as
the reverse-DNS app identifier root for future externally installed Cowork
applications and local service identities.

Phase 1 keeps the current BitFun build and launch path intact while Cowork's
daemon/runtime boundary is established. Existing `bitfun-*`, `BitFun`, and
`com.bitfun.*` identifiers remain live until a later task explicitly migrates
the corresponding surface and verifies the replacement path.

## Current Live Identity

These identifiers are intentionally unchanged in Phase 1:

| Surface | Current live identity | Source |
| --- | --- | --- |
| Root Node package | `BitFun` | `package.json` |
| Desktop Tauri product name | `BitFun` | `src/apps/desktop/tauri.conf.json` |
| Desktop Tauri app identifier | `com.bitfun.desktop` | `src/apps/desktop/tauri.conf.json` |
| Desktop Rust package and binary | `bitfun-desktop` | `src/apps/desktop/Cargo.toml` |
| CLI Rust package and binary | `bitfun-cli` | `src/apps/cli/Cargo.toml` |
| Shared web UI package | `@bitfun/web-ui` | `src/web-ui/package.json` |
| Installer package and binary | `bitfun-installer` | `BitFun-Installer/package.json`, `BitFun-Installer/src-tauri/Cargo.toml` |
| Installer Tauri product name | `BitFun Installer` | `BitFun-Installer/src-tauri/tauri.conf.json` |
| Installer Tauri app identifier | `com.bitfun.installer` | `BitFun-Installer/src-tauri/tauri.conf.json` |
| Workspace metadata author | `BitFun Team` | `Cargo.toml` |

The implementation fork remains `https://github.com/jroth1111/BitFun` at this
stage, with upstream notices preserved as recorded in `docs/COWORK_BASELINE.md`.

## Staged Cowork Identity Map

These are the intended Cowork identities when the corresponding surface is
intentionally migrated:

| Surface | Staged Cowork identity | Phase 1 status |
| --- | --- | --- |
| Product/display name | `Cowork` | Documented only |
| Product slug | `cowork` | Use for new Cowork-owned docs and future code |
| Reverse-DNS root | `com.jroth.cowork` | Documented only |
| Desktop app identifier | `com.jroth.cowork.desktop` | Do not apply in Phase 1 |
| Daemon service identity | `com.jroth.cowork.daemon` | Reserve for the daemon boundary task |
| Protocol package/crate prefix | `cowork-protocol`, `cowork-*` | Use for new daemon/protocol crates if added by later tasks |
| Desktop package/binary | `cowork-desktop` | Do not rename in Phase 1 |
| CLI package/binary | `cowork-cli` or `cowork` | Decide during CLI migration; do not rename in Phase 1 |
| Web UI private package | `@cowork/web-ui` | Do not rename in Phase 1 |
| Installer app identifier | `com.jroth.cowork.installer` | Do not apply in Phase 1 |
| Installer package/binary | `cowork-installer` | Do not rename in Phase 1 |
| Repository name | `jroth1111/cowork` or successor | Defer until package/app migration |

New Cowork-owned daemon/protocol work should prefer `cowork-*` names. Existing
BitFun-derived crates should not be renamed just to make the tree look branded;
rename only when the runtime owner or externally visible package surface is
actually being migrated.

## BitFun Launch Path Preservation

The existing BitFun launch path is explicitly preserved for Phase 1:

- `pnpm run desktop:dev` remains the development launch command.
- `pnpm run desktop:build` and `pnpm run desktop:build:fast` remain the desktop
  build commands.
- `pnpm run cli:dev` remains the CLI launch command.
- The desktop bundle continues to use product name `BitFun` and app identifier
  `com.bitfun.desktop`.
- The desktop and CLI binaries remain `bitfun-desktop` and `bitfun-cli`.
- The installer continues to use `BitFun Installer`, `bitfun-installer`, and
  `com.bitfun.installer`.

No current BitFun launch surface is replaced by this namespace decision. A later
replacement task must document the replacement path, data/state migration risk,
and verification evidence before changing live identifiers.

## Migration Rules

1. Add new Cowork daemon/protocol surfaces under `cowork-*` names when they are
   new runtime authority, not aliases for existing BitFun behavior.
2. Preserve upstream BitFun names where they still define the active launch or
   compatibility surface.
3. Do not mix app identifiers. A single installed Cowork app should use the
   `com.jroth.cowork.*` family only after the corresponding BitFun app
   identifier is intentionally retired or migration-safe.
4. Preserve MIT notices and bundled license files when moving or renaming any
   BitFun-derived source, assets, skills, or templates.
5. Treat package/app identifier changes as contract-surface changes: they need
   explicit migration scope, replacement proof, and launch/build verification.

## Open Follow-Up

This decision does not choose the final GUI package name, final CLI command
shape, installer migration timing, or persisted data directory migration path.
Those should be decided with the daemon protocol boundary and packaging tasks,
after the Phase 1 BitFun launch path remains proven.
