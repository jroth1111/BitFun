# Cowork BitFun Notices

This fork preserves the upstream BitFun root notice:

- Project: `GCWing/BitFun`
- Root license file: `LICENSE`
- License: MIT License
- Copyright notice: `Copyright (c) 2026 CWing`

The full MIT License text remains in `LICENSE` and must be included with copies
or substantial portions of the upstream BitFun software.

## Built-in Skill Notices

The fork baseline includes bundled office skills with their own notice files.
Keep these package-level notices with the corresponding materials when copying,
moving, extracting, or redistributing those skills:

- `src/crates/core/builtin_skills/docx/LICENSE.txt`
- `src/crates/core/builtin_skills/pdf/LICENSE.txt`
- `src/crates/core/builtin_skills/pptx/LICENSE.txt`
- `src/crates/core/builtin_skills/xlsx/LICENSE.txt`

These paths are recorded as notice-bearing materials already present in the
BitFun fork baseline. They are not a blanket approval to extract, transform, or
redistribute bundled skill materials without the implementation-time legal
approval records described in `docs/COWORK_LEGAL_APPROVALS.md`.

## Planning Approval References

The Cowork planning package names legal approval references for reuse decisions:

- `validation-sprint/LEGAL_CLEARANCE_REGISTER.md`
- `validation-sprint/receipts/G7_LICENSE_AUDIT.md`

Those planning documents record that legal letters approved full porting or
reuse for bundled Anthropic skill packs, `dtyq/magic`,
`different-ai/openwork`, and `different-ai/openwork/ee`, subject to recording
letter identifiers, dates, approving parties, covered paths, allowed use, and
obligations before direct implementation imports.

Current restricted-source status: not imported. No source paths from
`dtyq/magic`, `different-ai/openwork`, or `different-ai/openwork/ee` are claimed
here as imported. This notice is usable before later source imports because it
preserves the existing BitFun MIT notice, records current built-in skill notice
paths, and points future imports to the legal approval record required before
bringing in restricted or letter-covered materials.
