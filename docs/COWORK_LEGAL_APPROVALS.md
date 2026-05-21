# Cowork Legal Approval References

This file is the fork-local index for legal approval references that must stay
visible while Cowork imports, moves, or redistributes upstream and third-party
materials.

## Preserved BitFun Notice

- Upstream source: `GCWing/BitFun`
- Root notice file: `LICENSE`
- Root license: MIT License
- Root copyright notice: `Copyright (c) 2026 CWing`

The root MIT notice is preserved in `LICENSE` and summarized in `NOTICE.md`.
Do not remove or rewrite that notice during Cowork rebranding, daemon
extraction, or package restructuring.

## Existing Built-in Skill Notice Paths

The following notice files are present in the BitFun fork baseline and must
remain attached to the corresponding bundled skill materials:

- `src/crates/core/builtin_skills/docx/LICENSE.txt`
- `src/crates/core/builtin_skills/pdf/LICENSE.txt`
- `src/crates/core/builtin_skills/pptx/LICENSE.txt`
- `src/crates/core/builtin_skills/xlsx/LICENSE.txt`

These entries document existing notice-bearing paths. They do not by themselves
approve any new extraction, derivative use, redistribution, or import of bundled
skill materials.

## Planning Package References

The approval trail lives in the planning package:

- `LEGAL_CLEARANCE_REGISTER`: `/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/LEGAL_CLEARANCE_REGISTER.md`
- `G7_LICENSE_AUDIT`: `/Users/gwizz/CascadeProjects/cowork-best-of-breed-plan/validation-sprint/receipts/G7_LICENSE_AUDIT.md`

Those references state that approval letters were received and verified for
full porting or reuse of:

- bundled Anthropic skill packs
- `dtyq/magic`
- `different-ai/openwork`
- `different-ai/openwork/ee`

Before any direct source or asset import from those letter-covered materials,
create an implementation entry that records:

- letter identifier or file reference
- letter date
- approving party
- source repository and commit
- exact imported paths
- allowed use and redistribution scope
- attribution, notice, branding, or source-availability obligations
- expiry, field-of-use, or product-scope limitations
- reviewer or owner who accepted the import

## Current Import Claim

Current restricted-source status: not imported. As of this notice file, no
source paths from `dtyq/magic`, `different-ai/openwork`, or
`different-ai/openwork/ee` are claimed as imported. The approval references are
named so the fork can make later source-import decisions with a usable policy
and a concrete legal-record checklist, not to silently treat
approved-but-restricted paths as already imported.
