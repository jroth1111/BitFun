# Cowork BitFun Fork Baseline

Date recorded: 2026-05-21

This repository is the Cowork implementation base forked from BitFun.

## Fork Identity

- Implementation fork: `https://github.com/jroth1111/BitFun`
- Upstream source: `https://github.com/GCWing/BitFun`
- Upstream default branch: `main`
- Local branch: `main`
- Upstream remote: `upstream`
- Upstream push URL: disabled locally with `git remote set-url --push upstream DISABLED`

## Upstream Baseline

- Recorded upstream commit: `ec3321b984951ccaee9db9781f4f07c73b5b9e5b`
- Recorded fork commit: `ec3321b984951ccaee9db9781f4f07c73b5b9e5b`
- Commit author: `CWing <302787376@qq.com>`
- Commit author date: `2026-05-21T15:26:43+08:00`
- Commit subject: `Merge pull request #819 from wsp1911/main`
- Tag at recorded commit: `nightly`
- Latest upstream release at baseline time: `v0.2.7`
- Latest upstream release URL: `https://github.com/GCWing/BitFun/releases/tag/v0.2.7`
- Latest upstream release published: `2026-05-12T13:23:23Z`
- Latest upstream release target commit: `9a45437327673254dd81e176a3189bcb8399c0a2`

The recorded implementation baseline is newer than the latest release tag. Keep
this distinction visible when comparing release behavior to current `main`.

## License And Notices

- Root license: MIT License
- Root license file preserved: `LICENSE`
- Root license copyright line: `Copyright (c) 2026 CWing`
- Built-in office skill license files present at baseline:
  - `src/crates/core/builtin_skills/docx/LICENSE.txt`
  - `src/crates/core/builtin_skills/pdf/LICENSE.txt`
  - `src/crates/core/builtin_skills/pptx/LICENSE.txt`
  - `src/crates/core/builtin_skills/xlsx/LICENSE.txt`

Do not remove or rewrite upstream notices during Cowork rebranding or daemon
extraction. Preserve package-level notices when copying, moving, or extracting
bundled skills and templates.

## Legal Approval References

The planning package records legal clearance references for approved reuse:

- `/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/LEGAL_CLEARANCE_REGISTER.md`
- `/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/receipts/G7_LICENSE_AUDIT.md`

Before importing source or assets from approved-but-restricted sources such as
`dtyq/magic`, `different-ai/openwork`, `different-ai/openwork/ee`, or bundled
Anthropic skill packs, populate the legal clearance register with the letter ID,
approval date, approving party, covered paths, allowed use, and obligations.

## Initial Build Commands

Use the repository-native commands from `AGENTS.md`, `README.md`, and
`package.json`:

```bash
pnpm install
pnpm run desktop:dev
pnpm run desktop:build
```

Fast local smoke/build lane:

```bash
pnpm run desktop:build:fast
```

Minimum verification lanes from upstream guidance:

```bash
pnpm run lint:web
pnpm run type-check:web
cargo check --workspace
cargo test --workspace
pnpm --dir src/web-ui run test:run
```

Prior validation in the planning package used Homebrew Node 26 with pnpm 10.15.0
for a clean install/build path:

```bash
/opt/homebrew/bin/node /opt/homebrew/lib/node_modules/pnpm/bin/pnpm.mjs install --frozen-lockfile
```

Reference receipt:
`/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/receipts/G1_BITFUN_BUILD_LAUNCH.md`.

## Baseline Evidence Commands

The baseline was recorded from these local checks:

```bash
git remote -v
git rev-parse HEAD
git rev-parse upstream/main
git tag --points-at HEAD
git describe --tags --always --dirty
gh release view v0.2.7 --repo GCWing/BitFun --json tagName,name,url,publishedAt,targetCommitish
```
