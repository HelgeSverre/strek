# SVG Optimization and Export Gauntlet

Status: research and Gauntlet setup complete; implementation has not started. This is a separate run from the editor-product Gauntlet in [`GAUNTLET-RUN.md`](GAUNTLET-RUN.md).

## Objective

Determine how Strek can produce smaller, cleaner SVG and raster exports without changing their rendered appearance or destroying useful editability. The research will compare the current SVG optimization ecosystem, explain what happened to SVGOMG, and choose an implementation strategy and measurable Gauntlet bar for Strek rather than assuming that porting one existing project is the right answer.

## Decision questions

1. Which optimizer is the strongest maintained reference today, and is its architecture suitable for embedding in a native Rust editor?
2. What happened to SVGOMG, what did its interface contribute beyond SVGO, and which parts remain worth adapting?
3. Should optimization operate on Strek's typed scene graph before serialization, on serialized SVG, or in both stages?
4. Which rewrites are provably appearance-preserving, and which must be explicit lossy or structure-changing options?
5. How should Strek prove semantic and visual equivalence across filters, masks, clipping, text, gradients, strokes, transforms, external references, and malformed/adversarial input?
6. Which size, speed, determinism, compatibility, metadata, and editability measurements form a bar an independent critic cannot explain away?
7. Can a maintained Rust implementation be adopted directly, should selected passes be ported, or is an SVGO subprocess/plugin boundary the safer first release?
8. What export workflow would make optimization understandable and reversible instead of a black-box “minify” switch?

## Systems and adjacent references to investigate

| System | Category | Initial reason for inclusion |
| --- | --- | --- |
| SVGO | JavaScript SVG optimizer | Established plugin architecture and de facto optimization reference |
| SVGOMG | Browser UI for SVGO | Best-known interactive explanation and preview of optimizer passes |
| OXVG and maintained Rust alternatives | Native optimizer candidates | Potentially embeddable implementation with a better deployment fit |
| Scour | Python SVG optimizer | Independent rewrite set and CLI behavior |
| usvg/resvg | Parser, normalizer, renderer | Strek's current semantic/rendering boundary and likely equivalence oracle |
| Browser engines and SVG conformance corpora | Independent rendering references | Catch optimizer bugs that a single renderer or parser would miss |

The first research pass may replace or add systems when maintenance status and source quality are verified.

## Evidence to collect, in priority order

1. Maintainer and repository status, release recency, license, and supported runtimes.
2. Transformation/pass inventory and whether each pass is safe, configurable, deterministic, or lossy.
3. Parser and serialization model, extensibility, embedding API, and streaming or whole-document constraints.
4. Handling of SVG 2, CSS, IDs/references, namespaces, text/fonts, animation, filters, masks, clipping, and external resources.
5. Security/resource bounds for adversarial SVG.
6. Benchmark methodology: bytes, runtime, memory, visual equivalence, DOM/editability retention, and cross-renderer compatibility.
7. UX patterns for presets, per-pass controls, before/after inspection, warnings, undo, and reproducibility.

## Per-system report contract

Each report must include:

1. Overview and current maintenance status.
2. Architecture and transformation model.
3. Capability and safety profile.
4. Integration options for Strek.
5. Strengths, limitations, and failure modes.
6. A confidence marker for each section: high for primary sources/code, medium for demos or indirect documentation, low for inference.
7. Direct links to every material source.

## Required synthesis

The completed document will add:

- a cross-system feature matrix;
- a recommended Strek architecture with explicit rejected alternatives;
- a corpus and differential-testing strategy;
- exact optimization, fidelity, determinism, and performance thresholds;
- an independent builder/critic Gauntlet loop and a short copy-paste run prompt;
- open questions that require prototypes rather than speculation.

## First-wave findings

### What is actually missing

Confidence: **high** for observed status; **low** for cause.

SVGOMG and SVGO are separate projects:

- [SVGOMG](https://github.com/jakearchibald/svgomg) is Jake Archibald's MIT-licensed browser interface for SVGO. It is public, not archived, and its [GitHub Pages app](https://jakearchibald.github.io/svgomg/) is live.
- [SVGO](https://github.com/svg/svgo) is the optimizer engine SVGOMG wraps. On 2026-08-02, the repository, other repositories in the `svg` organization, and `svgo.dev` returned 404 to direct GitHub/API requests.
- The npm package still serves [SVGO 4.0.2](https://www.npmjs.com/package/svgo), published 2026-07-11. Homebrew also packages 4.0.2. Maintainer Seth Falco's public [SVGO fork](https://github.com/SethFalco/svgo) contains the canonical history, the 4.0.2 commit, and activity from 2026-08-01.

No primary source explains the 404. Deletion, privatization, suspension, and temporary account state can look the same from outside, so this document does not invent a cause. The recent package and maintainer activity is evidence against calling SVGO abandoned. The maintainer fork is a useful source snapshot, not a new canonical home unless the maintainers say so.

### SVGOMG is a UX reference, not a porting base

Confidence: **high**, based on its repository and deployed application.

SVGOMG 1.17 is a small framework-free web application built with Gulp, Rollup, Nunjucks, and Sass. It sends optimization to a Web Worker, cancels superseded work, caches results by a settings fingerprint, persists settings locally, and previews an SVG data URL in a sandboxed iframe. Its repository exposes the [worker](https://github.com/jakearchibald/svgomg/blob/main/src/js/svgo-worker/index.js), [settings inventory](https://github.com/jakearchibald/svgomg/blob/main/src/config.json), [controller/cache](https://github.com/jakearchibald/svgomg/blob/main/src/js/page/main-controller.js), and [preview](https://github.com/jakearchibald/svgomg/blob/main/src/js/page/ui/svg-output.js).

The deployed app currently identifies its engine as SVGO 4.0.0. That version predates fixes for [entity-expansion denial of service](https://github.com/advisories/GHSA-xpqw-6gx7-v673) in 4.0.1 and an incomplete `removeScripts` sanitization transformation in [GHSA-2p49-hgcm-8545](https://github.com/advisories/GHSA-2p49-hgcm-8545) in 4.0.2. Its tests are primarily lint/build checks, not cross-renderer equivalence tests.

Adapt these SVGOMG ideas:

- immediate before/after preview, original toggle, image/markup views, byte counts, pan/zoom, background switch, copy/download, presets, cancellation, and cache;
- per-pass controls and a visible precision control.

Extend them with:

- side-by-side, blink, overlay, and heat-map comparison;
- raw, gzip, and Brotli size, with per-pass byte attribution;
- pass risk labels and a DOM/resource-reference diff;
- warnings for IDs, `viewBox`, dimensions, title/description, semantic text, and accessibility;
- an optimization manifest recording engine version, profile, passes, measurements, and verification;
- optimization of an exported copy only—never silent mutation of the native document.

Do not port SVGOMG's frontend stack, reuse its deployed bundle, or copy its script-capable preview policy.

### SVGO remains the benchmark incumbent

Confidence: **high**, based on the 4.0.2 npm snapshot and current documentation mirrored by package/CDN sources.

SVGO is an MIT-licensed JavaScript whole-document optimizer:

```text
SVG string -> SAX/XAST parser -> ordered visitor plugins -> serializer
```

Its public `optimize(input, config)` API uses `preset-default` unless a caller supplies a plugin list. Multipass reparses and reruns the pipeline until size stops improving, with a bounded pass count. The preset contains CSS, ID, numeric, path, transform, group, and dead-content rewrites. SVGO 4 removed `removeViewBox` and `removeTitle` from the default preset because those defaults harmed scalability and accessibility, but the remaining preset still changes document structure and must not be described as universally lossless. See the surviving [4.0.2 package metadata](https://unpkg.com/svgo@4.0.2/package.json), [default preset](https://unpkg.com/svgo@4.0.2/plugins/preset-default.js), and [optimizer](https://unpkg.com/svgo@4.0.2/lib/svgo.js).

Use pinned SVGO as a benchmark and differential critic. Do not ship Node, a JavaScript runtime, or an SVGO subprocess as Strek's primary export path.

### Optimization is not sanitization

Confidence: **high**.

An optimizer may preserve scripts, external references, CSS behavior, animation, and event-bearing structures—or transform them incompletely. Strek's strict source validation, complexity limits, and external-resource policy remain a separate boundary before any optimizer sees untrusted input. “Optimized” must never be used as a synonym for “safe.”

Prefer optimization of Strek's own validated export representation rather than arbitrary original source. The importer can preserve fidelity through the native model; the optimizer can then make claims about the exact structures the exporter owns.

### Risk classes

Confidence: **high** for the classification; an individual pass still requires corpus evidence.

| Risk class | Examples | What can change |
| --- | --- | --- |
| Exporter-safe | Deterministic serialization, generated whitespace, exact number formatting, generated-ID shortening, unused generated namespaces | Textual form only |
| Static-visual | Shape-to-path, group collapse, CSS inlining, ID rewriting, transform/path rewriting | DOM structure, editability, selectors, animation behavior |
| Numerically lossy | Coordinate rounding, curve/arc fitting, transform precision reduction | Geometry and rendered pixels |
| Destructive | Removing title/description, `viewBox`, dimensions, raster images, scripts, styles, or off-canvas geometry | Accessibility, layout, appearance, interaction, content |
| Context-sensitive | ID cleanup, hidden/default removal, path merging, defs deduplication | External CSS/JS, inheritance, references, paint order |

Strek should expose three profiles:

- **Safe**: exporter-proven exact rewrites; preserves accessibility, IDs not owned by Strek, structure, geometry, dimensions, and `viewBox`.
- **Web**: allows verified static-visual structural changes; must pass renderer and semantic-reference gates.
- **Maximum**: explicit opt-in for numerical or destructive changes, with a visible diff and per-change warning. It must not claim exact fidelity.

Every pass reports its risk class, changed nodes, bytes saved, and verification result.

## Recommended architecture

Confidence: **high** for the boundary; individual passes require prototypes.

```text
Native document
  -> validated export scene
  -> typed SVG export AST
  -> profile-specific measured passes
  -> compact deterministic serializer
  -> semantic and renderer verifier
  -> file + optimization manifest
```

The optimizer works on a disposable export copy. Import validation and sanitization stay before it. SVG text is serialized after typed passes, not reparsed for the ordinary Strek path.

Start with passes that can exploit exporter knowledge generic tools lack:

1. Emit only necessary namespaces and attributes.
2. Use shortest deterministic number formatting that round-trips to the same value.
3. Compact equivalent colors.
4. Generate minimal deterministic resource IDs.
5. Deduplicate gradients and clips by canonical typed-resource hashes.
6. Remove defs proven unreachable in Strek's reference graph.
7. Choose shorter equivalent transform and path spellings without changing geometry.
8. Remove only groups known to have been introduced by Strek's exporter.
9. Produce pretty and minified text from the same AST.

Run pinned SVGO 4.0.2 and the strongest native candidates only in the development benchmark harness. A dependency is adopted only after its safety profile, license provenance, determinism, resource bounds, and corpus results beat the typed implementation.

## Ecosystem comparison

| System | Current role | Architecture | Main strength | Release-blocking concern | Strek decision |
| --- | --- | --- | --- | --- | --- |
| SVGO 4.0.2 | Incumbent optimizer | JavaScript XAST plugin pipeline | Mature pass inventory and ecosystem | Canonical hosting currently unavailable; structural passes are context-sensitive | Pin as benchmark, do not embed |
| SVGOMG 1.17 | Optimizer UI | Browser UI + worker + SVGO | Excellent interactive before/after workflow | Deployed engine is behind security fixes; no fidelity suite | Adapt UX concepts only |
| OXVG 0.0.6 | Rust challenger | Typed AST, Rust jobs, CLI/N-API/WASM | Native integration and broad ambitions | Compiled GPL-derived file conflicts with declared MIT surface; “Safe” prechecks are disabled | Hold; benchmark explicit allowlist only |
| SVGM 0.3.8 | Rust challenger | Arena AST and bounded fixed-point pass loop | Small native core and attractive convergence model | Open malformed-path hang, million-fold CSS growth, and truncated-input acceptance | Isolated benchmark only; do not embed |
| svg-hush 0.9.6 | Rust sanitizer | Streaming allowlist filter | Maintained, fuzzed, explicit parser limits | Intentionally may break SVG; sanitization is destructive | Optional separate “Sanitize for web” profile |
| svgcleaner | Legacy Rust optimizer | Native pass pipeline | Prior art | Archived, GPL-2.0, inactive since 2021 | Do not adopt |
| Scour 0.38.2 | Python optimizer | Monolithic minidom rewrite pipeline | Independent historical comparison | Dormant, unbounded recursion, correctness bugs, and default URL fetching for raster embedding | Do not use in product; sandboxed historical baseline only |
| usvg/resvg 0.45.1 in Strek; 0.47 current | Normalizer/renderer | Rust normalized tree + static renderer | Reproducible rendering and broad regression corpus | Not an optimizer; static subset; normalization discards source structure | Keep as verifier/oracle; assess upgrade separately |

### Native-candidate verdicts

Confidence: **high** for source observations and current issue status; **medium** for future adoption potential.

#### OXVG 0.0.6

[OXVG](https://github.com/noahbald/oxvg) is the strongest long-term Rust candidate. It has an arena-backed typed AST, Lightning CSS integration, roughly 55 jobs, Rust/CLI/N-API/WASM surfaces, active 2026 development, and an SVGO-config conversion layer. Parcel uses it as an SVG minifier. Its own documentation still calls the libraries unstable and recommends SVGO when stability is required.

It is not currently shippable inside Strek:

- the repository and crate declare MIT, but compiled [`precheck.rs`](https://github.com/noahbald/oxvg/blob/main/crates/oxvg_optimiser/src/jobs/precheck.rs) declares GPLv2-derived provenance from SVG Cleaner;
- `Jobs::safe()` enables `Precheck::default()`, while that default sets `preclean_checks` to false, so the checks for scripts, animation, conditions, and external references do not run;
- the CLI enables DTD parsing, and no Strek-grade byte/node/attribute/pass-work/output-growth budgets were found;
- published [benchmarks](https://github.com/noahbald/oxvg/wiki/Benchmarks) are self-reported single runs, not equivalence evidence.

Ask upstream to resolve the license provenance and make the safe preset meaningful. Until then, run an explicit pass allowlist only in the benchmark harness.

#### SVGM 0.3.8

[SVGM](https://github.com/madebyfrmwrk/svgm) has an appealing small Rust core, 34 passes, an arena AST, and a fixed-point loop bounded to ten iterations. It is also too young for untrusted input. Current open issues show [invalid paths can hang](https://github.com/madebyfrmwrk/svgm/issues/22), [non-ASCII CSS can amplify output roughly a million-fold](https://github.com/madebyfrmwrk/svgm/issues/23), and [truncated input can return an empty success](https://github.com/madebyfrmwrk/svgm/issues/24). Its custom CSS handling uses raw delimiter scans, and no input/node/depth/output budgets were found. Version 0.3.8 itself fixed inherited fill/stroke removal that made elements invisible.

Use its convergence architecture as prior art. If it remains in a bakeoff, execute it in a killed, memory-limited subprocess until the blockers are fixed.

#### usvg/resvg

Strek embeds usvg/resvg 0.45.1; [0.47.0](https://github.com/linebender/resvg/releases/tag/v0.47.0) is current. usvg resolves CSS/inheritance/references, expands uses and markers, converts geometry, and deliberately normalizes the static subset. resvg provides deterministic rendering and the public regression corpus. Neither is a source-preserving optimizer. Keep the existing no-external-resource resolver and caller-owned byte, node, image, canvas, font, and work budgets. Pin fonts in cross-platform render tests. Treat a 0.47 upgrade as a separate compatibility project because it changes the Rust floor and parser/render behavior.

#### svg-hush

[svg-hush 0.9.6](https://github.com/cloudflare/svg-hush) is a maintained MIT/Apache-2.0 streaming sanitizer with an allowlist, fuzz targets, and explicit parser limits. It removes scripts, unsafe namespaces, cross-origin URLs, hyperlinks, and dangerous CSS URLs. Its README correctly warns that safety filtering may break SVG. That makes it a plausible engine for a separately named, explicit “Sanitize for web” export—not for normal optimization and never as a hidden step.

#### Scour and SVG Cleaner

[Scour](https://github.com/scour-project/scour) is Apache-2.0 but its last release was in 2020. It uses a minimal CSS parser, has no meaningful work/depth limits, has open recursion and reference-deletion bugs, and its default raster-embedding path can call `urllib.request.urlopen` on file or remote references. Do not use it in product.

[SVG Cleaner](https://github.com/RazrFalcon/svgcleaner) is archived, GPL-2.0, and built on obsolete dependencies. Its pass taxonomy and precheck methodology are useful prior art, but its source must not be ported into Strek.

## Corpus and acceptance scoreboard

The critic inspects actual files and renders, never a builder summary.

### Corpus

- the public [resvg test suite](https://github.com/linebender/resvg-test-suite), currently roughly 1,600 SVG-to-PNG regression cases;
- relevant W3C SVG cases for Strek's declared static subset;
- checked-in exports from Figma, Illustrator, Affinity, and Inkscape;
- real logos, icon systems, gradients, clipping, masks, patterns, filters, text, nested resources, and CSS/reference cases;
- adversarial entity, nesting, CSS, cycle, extreme-number, list-amplification, and external-resource cases;
- Strek-native documents exported through every recipe, then saved/reopened and exported again.

### Hard gates

| Dimension | Safe/Web bar | Maximum profile bar |
| --- | --- | --- |
| Parse and references | Both original and optimized exports parse; every local reference resolves with the same allowed type | Same |
| Raster fidelity | Zero differing pixels in pinned resvg and Chromium renders at 1x, 2x, and 4x on supported static cases | Difference must be shown; threshold is corpus-calibrated and never labeled exact |
| Semantics | Same `viewBox`, dimensions, accessible name/description, semantic text, and protected IDs; no external resource introduced | Every change explicitly listed and approved by profile |
| Determinism | Byte-identical output across 100 runs and supported OS/architecture CI | Same |
| Idempotence | A second optimization is byte-identical to the first | Same |
| Bounds | Hostile cases reject before unbounded parser/normalizer work; no panic, hang, stack overflow, or partial file | Same |
| Size | Geometric-mean raw/gzip/Brotli size matches or beats the best pinned competitor on Strek-generated exports; regressions are attributed per file | Must beat Web profile or explain the explicit fidelity trade |
| Performance | Typical export remains interactive; candidate thresholds are p95 <= 25 ms for 1 MiB and <= 250 ms for 10 MiB on the pinned benchmark host | Same |
| Memory | Candidate threshold: peak incremental memory <= 4x input + 32 MiB on accepted corpus | Same |

Performance and memory numbers are provisional until the harness records a baseline on the release runner. The run must replace them with measured thresholds before the first builder wave; it may not remove the dimension.

Each result records import outcome, documented normalization, native save/reopen, exported parse, resvg/Chromium differentials, semantic invariants, raw/gzip/Brotli bytes, elapsed time, peak allocation, determinism, idempotence, and per-pass attribution.

## Gauntlet workstreams

1. Corpus, minimizer, and blind scorer.
2. Unsupported static-SVG feature expansion.
3. Typed export AST and deterministic serializer.
4. Resource, ID, and defs optimization.
5. Geometry, path, and transform optimization.
6. Security and complexity bounds.
7. Native SVGOMG-inspired export workbench.
8. Batch recipes and automation.
9. Fresh smoothing critic over the complete editor.

Each workstream gets a builder and a separate critic with fresh context. A critic sees the artifact, pinned competitors, render/semantic diffs, byte attribution, runtime, and peak allocation. It returns the largest remaining gap. There is no fixed round count.

## Copy-paste run prompt

```text
Make Strek the most trustworthy and capable native static-SVG editor and
export optimizer.

The bar is the committed resvg/W3C/real-editor corpus and a pinned comparison
against SVGO 4.0.2 plus the strongest verified native challengers. Any SVG
Strek accepts must survive import, native save/reopen, canvas rendering, and
SVG export without silent appearance changes. Optimized Strek exports must be
deterministic, idempotent, bounded on hostile input, and match or beat the
smallest competitor output whenever fidelity and supported semantics remain
intact.

Divide the goal into the smallest independently judgeable pieces. Give each
important piece a builder and a separate fresh critic. Critics must inspect
the real SVG, native document, raster diffs, semantic invariants, byte counts,
timings, peak memory, and actual editor UI. A pass gets no credit for making
files smaller unless it proves what it preserved and attributes the savings.

Adapt the strongest ideas from SVGOMG into a native export workbench, but let
the evidence choose the optimizer architecture. Keep optimization separate
from sanitization and never mutate the native document silently. Maintain a
live progress page and keep looping until Strek wins the bar or I stop the run.
```

## Open questions and required prototypes

1. Does a minimal typed export AST outperform text-level passes enough to justify a new crate boundary?
2. Can exact float round-tripping produce competitive bytes without a numerical-loss profile?
3. Which cross-renderer disagreements are optimizer regressions versus existing engine differences?
4. What is the smallest useful semantic invariant set for externally embedded SVG without pretending arbitrary scripts/CSS are supported?
5. Can OXVG's license provenance and safe-profile behavior be cleared well enough for optional development-harness use?
6. Should Maximum-profile output be downloadable after a failed fidelity gate, or require a second explicit confirmation?
