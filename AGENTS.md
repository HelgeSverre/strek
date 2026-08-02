# Strek Delivery Rules

## Tangible progress, anti-ceremony, and honest credit

The purpose of this project is a working native vector editor whose drawing,
editing, import, export, persistence, clipboard, and automation behavior users
can exercise. Deliver changes as usable vertical slices through the core model,
history, display list, GPUI frontend, persistence, and relevant interchange
formats. Process exists to serve that outcome; it is not the product.

- **No process porn.** Plans, matrices, status documents, and compatibility
  inventories count only when they gate a named behavior, specification clause,
  platform decision, or release. Do not substitute administration for product
  behavior.
- **Feature-first work.** Most work must produce user-visible editor behavior or
  a required correctness property. Documentation and repository maintenance must
  name the behavior, invariant, or release gate they unblock.
- **Honesty is absolute.** Never fake a test, weaken an assertion or validation
  limit to make a case pass, regenerate expected output around a regression, or
  claim stronger evidence than was run. Report the verified layer precisely:
  unit, fixture, import/export pipeline, raster comparison, interactive GPUI,
  platform, or end-to-end.
- **Refusal is not delivery.** Rejecting malformed or unsupported files is
  required safety behavior, but it is not support for the rejected feature. Each
  supported workflow needs a positive case that exercises real document data and
  reaches its user-visible result.
- **Vertical slices ship together.** A behavior is not complete when only its
  type, parser, display-list variant, backend, UI, or test exists. Implement and
  verify every applicable layer in the same change.

These rules bind humans and agents working in this repository and should shape
the acceptance criteria and review of every implementation task.

## Capability claims

Use precise language in code, documentation, issues, and handoffs:

- **Imported** means source data reaches the native document model.
- **Persisted** means it survives a validated native save/load round trip.
- **Rendered** means both SVG export and the current GPUI canvas path preserve its
  appearance within documented limits.
- **Editable** means the user has an intentional native editing workflow for the
  property. Merely storing it or editing generated path anchors does not imply a
  semantic editor.
- **Round-tripped** means the source semantics and structure survive export; a
  normalized visual equivalent is not a structural round trip.
- **Cross-platform** means the behavior was exercised on each named platform or
  is explicitly backed by platform-independent code with remaining platform
  evidence called out.
- **Complete** means positive and negative behavior, persistence when applicable,
  undo/redo for authored edits, rendering/export, bounded failure, documentation,
  and the relevant test layers are all handled.

Do not hide limitations behind “supports,” “editable,” “safe,” “normalized,”
“fallback,” or “mostly.” State what is preserved, transformed, rasterized,
resolved, discarded, or rejected. In particular:

- A software-raster canvas fallback is rendering support, not native GPU/vector
  editing support. State its pixel and cache limits.
- Text converted to outlines is path-editable, not semantic text; font fallback
  may change its appearance.
- Expanded `<use>`, symbols, markers, links, switches, CSS, or metadata are not
  structurally preserved unless the native model and export prove that they are.
- SVG normalization is admitted only after local references and a conservative
  expansion-work budget are validated. CSS resource references are limited to
  selectors the preflight can account for.
- A parser accepting syntax is not fidelity. SVG claims must survive import,
  native persistence, display-list generation, canvas rendering, and export as
  applicable.
- A strict rejection is an unsupported boundary, not partial feature delivery.

## Named reward-hacking patterns (forbidden)

1. **Gate self-weakening** — changing validation limits, scene invariants,
   interaction rules, expected output, or tests so a failure passes without
   fixing the underlying behavior. If the contract is wrong, change it
   deliberately and explain why.
2. **Proof-class inflation** — presenting hand-built display items, parser-only
   fixtures, string-presence assertions, or one nontransparent raster pixel as
   proof of visual fidelity. Use those only for the layer they actually test.
   End-to-end claims require real document construction and the real pipeline;
   visual claims require meaningful geometry/color assertions or differential
   rendering when practical.
3. **Golden regeneration reflex** — updating snapshots, screenshots, or expected
   SVG merely to match broken output. Inspect and explain the semantic visual
   change.
4. **Compile-stream pumping** — adding types, variants, scaffolding, `todo!()`, or
   `unimplemented!()` and counting compilation as feature delivery. Compilation
   is the floor.
5. **Tautological tests** — duplicating implementation logic in a test, asserting
   only that output exists or parses, or testing a fallback without proving it
   was selected. Include a case a plausible incorrect implementation would fail.
6. **Easy-task cherry-picking** — adding more primitives, aliases, menu items, or
   parser mappings while core editing semantics, history, rendering fidelity,
   persistence, or platform behavior remain incomplete.
7. **Premature completion** — declaring a tool, file-format feature, platform, or
   SVG capability complete from a happy-path unit test. Verify the applicable
   positive, negative, migration, interaction, render, and export paths.
8. **Scope-splitting** — claiming separate completion credit for the model,
   implementation, tests, and docs of one behavior. They are one vertical slice.
9. **Spec-editing as progress** — weakening or endlessly refining
   [`docs/spec.md`](docs/spec.md) instead of implementing it. Documentation may
   clarify or deliberately change the contract, but does not satisfy it.
10. **Conformance metastasis** — adding speculative matrices, abstractions,
    benchmarks, or validators without a named product invariant, observed defect
    class, compatibility boundary, or release target.
11. **Dependency smuggling** — using a crate, helper process, renderer, network
    request, or generated file to bypass project safety or fidelity boundaries.
    SVG import must not fetch external resources or silently delegate unsupported
    behavior. New dependencies must preserve bounded, deterministic operation.
12. **Demo-path hardcoding** — special-casing fixture names, sample geometry,
    repository paths, fonts, viewport sizes, IDs, platform state, or the current
    machine so showcased cases pass.
13. **Fallback laundering** — calling a rasterized, flattened, approximated, or
    discarded representation native support without documenting the conversion
    and its limits.
14. **Refusal farming** — accumulating descriptive errors for unsupported inputs
    while avoiding the positive implementation paths users need.

## Project-specific completion gates

Apply only the gates relevant to the change, but do not omit a relevant layer:

- Core authored changes are undoable and redoable through commands or
  transactions; transient previews do not leak into history.
- New persisted data has serde defaults or a deliberate migration, validation,
  complexity limits, a format-version decision, and save/load tests.
- New scene behavior reaches `editor_render` without frontend-specific types.
- Simple artwork retains the native GPUI path. Any fallback is bounded, cached
  where appropriate, keeps editor overlays native, and has a regression test
  proving the fallback is selected.
- SVG work distinguishes source validation, `usvg` normalization, native import,
  display-list conversion, SVG serialization, raster export, GPUI display, and
  hit testing. Test every layer affected by the claim.
- Import must fail descriptively rather than silently changing unsupported
  appearance. Accepted normalization may discard nonvisual structure only when
  documentation says so.
- Export tests use meaningful view boxes, transforms, transparency, and pixel or
  geometry checks. Parsing generated SVG is necessary but not sufficient for
  fidelity claims.
- Security and resource work exercises malformed, deeply nested, oversized,
  non-finite, and externally referencing inputs where applicable.
- Platform-specific claims name and test the platform. A macOS run is not
  Windows or Linux evidence.

The normal Rust floor is:

```text
just format
just lint
just test
```

If an unrelated or platform-specific failure prevents the full gate, run the
largest relevant focused suites, report the exact failing test and error, and do
not describe the workspace as green.

Before completing a substantial implementation, review the diff for correctness,
security, regressions, performance, maintainability, and adequate tests. Fix
findings before handoff when they are in scope.

## Current SVG capability boundaries

Keep these limitations explicit until code and tests remove them:

- Gradient and advanced-stroke values can be imported, persisted, rendered, and
  exported, but do not yet have dedicated native editing controls.
- Clip paths can be imported, persisted, rendered, exported, and used by hit
  testing, but do not yet have a native authoring/editing workflow.
- Imported text is converted through installed/fallback fonts to outline paths;
  it is not retained as semantic text.
- Reuse, symbols, markers, links, switches, CSS, and nested SVG are normalized to
  visual document nodes. Their original structure and interactive semantics are
  not round-tripped. Metadata is accepted but not preserved.
- Advanced GPUI artwork uses a bounded world-space software-raster cache. Every
  new artwork revision and active-gesture preview is limited to 4,096 pixels per
  axis and 1,000,000 pixels total; 50 ms trailing-edge settled renders are
  limited to 16,384 pixels per axis and 16,000,000 pixels total (about 4 MB and
  64 MB of RGBA pixels).
  Oversized artwork is proportionally downsampled. The cache keeps one active
  image and at most one rollback image until a replacement paints successfully,
  for a transient 128 MB two-settled-image ceiling, and clears both on document
  replacement. Pan and zoom reuse the cached image while selection/tool overlays
  remain native.
- Dashed strokes render correctly, but stroke hit testing does not exclude dash
  gaps.
- Images, patterns, masks, filters, blend modes, isolation, non-scaling strokes,
  context-dependent paints, CSS at-rules, SVG 2 focal radii, arcs joins, and
  non-sRGB gradient interpolation remain unsupported and must fail explicitly.

## Documentation lookup

Use Context7 whenever a task asks about a library, framework, SDK, API, CLI, or
cloud service. Start with `resolve-library-id`, choose the best exact and
version-relevant match, then call `query-docs` with the full focused question.
Use separate queries for separate concepts. Do not use Context7 for ordinary
refactoring, business-logic debugging, code review, or general programming.

## Pull request branch strategy

- Prefer independent pull requests based on the intended upstream base branch.
  Each branch contains only that PR's commits.
- A branch targeting `main` is still stacked if its history contains another
  unmerged PR.
- Before publishing multiple PRs, determine whether each change can be rebased
  or cherry-picked onto the common base and pass independently.
- Use a stacked PR only for a real code or semantic dependency, and document the
  dependency and merge/rebase order.
- If independent versus stacked is ambiguous, ask before creating or pushing.
