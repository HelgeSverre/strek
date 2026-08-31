# Strek Gauntlet Run

This is the proposed post-`v0.2.0` Gauntlet Loop for Strek. It adapts the
method from [How to Run a Gauntlet Loop](https://somethingbig.ai/gauntlet-loop):
set an inspectable bar, split the product into independently judgeable slices,
keep builders and critics separate, inspect real artifacts, and continue until
the result clears the bar or the run is stopped deliberately.

## Why this target

Strek already has substantial foundations: native drawing and text, hierarchy,
history, precision tools, persistence, import/export, clipboard support, and
semantic automation. The `v0.2.0` work also imports, persists, renders, and
exports gradients, advanced strokes, compound fill rules, opacity, and clip
paths within documented limits.

The largest remaining gap is not another isolated SVG primitive. It is the
inability to start with an empty document and finish a production-ready asset
family entirely inside Strek. In particular:

- Boolean path construction is missing.
- Imported gradients, advanced strokes, and clips have no dedicated native
  authoring controls.
- Several exact dimensions and transform values are readouts rather than a
  complete direct-entry workflow.
- Export operates on visible artwork rather than selected objects or frames,
  and reusable multi-format export recipes do not exist.
- The editor does not yet prove crash recovery of unsaved work.

Those gaps converge on one concrete product outcome: make Strek a native
logo-and-icon production system, not merely an editor that can ingest and
display sophisticated SVG.

## North star

Starting from an empty document, without importing prebuilt SVG, a user can
construct a polished multi-variant brand kit using native vector construction,
editable appearances, exact geometry, reusable export recipes, and the existing
Color Library. The project can be saved, undone and redone, force-quit and
recovered, reopened and edited, then batch-exported deterministically through
both the GPUI interface and semantic automation.

## Inspectable acceptance artifact

The run must produce a **Strek Native Brand Kit** containing:

- a 1024×256 horizontal lockup with semantic editable text;
- a 256×256 full-color icon;
- a 32×32 monochrome glyph;
- nontrivial native boolean geometry, including counters or holes and
  transformed inputs;
- authored linear and radial gradients with multiple editable stops and alpha;
- authored cap, join, dash, and clipping behavior;
- exact editable dimensions and transforms;
- saved frame or selection export recipes for standard SVG, outlined-text SVG,
  PNG, JPEG, and WebP at explicit scales or dimensions;
- recovery of unsaved work after a simulated abrupt termination.

Completion is judged from this real project and its generated artifacts, not
from parser success, isolated types, screenshots of controls, or hand-built
display-list fixtures.

## Concrete quality bars

- [Figma boolean operations](https://help.figma.com/hc/en-us/articles/360039957534-Boolean-operations)
  are the interaction bar for union, subtract, intersect, exclude, predictable
  style ownership, and continued editing of inputs. Strek may choose destructive
  or non-destructive semantics, but the choice must be explicit, consistent,
  persisted where applicable, and undoable.
- [Figma gradient editing](https://help.figma.com/hc/en-us/articles/34208860210199-Use-gradients-as-a-fill-or-stroke)
  is the bar for discovering, adding, removing, moving, recoloring, reversing,
  and directly manipulating gradient stops.
- [Figma export settings](https://help.figma.com/hc/en-us/articles/13402894554519-Export-formats-and-settings)
  and [Affinity Export Persona](https://affinity.help/designer2ipad/English.lproj/pages/Introduction/about_Personas.html)
  are the bars for object/frame targets, reusable recipes, suffixes, explicit
  scales or dimensions, previews, and batch export.
- Strek's automation showcase is the repeatability bar: every stable semantic
  operation needed to build the acceptance project must be automatable without
  depending solely on canvas coordinates.
- Strek's actual GPUI, display-list, SVG, and raster pipelines are the fidelity
  oracle. Visual evidence uses meaningful geometry, bounds, alpha, and color
  assertions or reviewed differential renders.

The external products are behavioral comparators, not architecture mandates or
a demand to clone their complete feature sets.

## Agent operating model

The run also exercises the coherent control system defined in
[`agent-control-plane.md`](agent-control-plane.md). Its purpose is not to add an
automation side project: it makes every Brand Kit slice cheaper to discover,
safer to drive, and easier to verify through the same editor semantics.

For each slice, define one outcome contract containing the user scenario,
semantic operation(s), applicability and revision preconditions, expected
receipt, and verification recipe. The durable artifact is executable behavior
plus evidence; do not maintain a second prose status matrix.

The lead starts from a compact session descriptor and capability manifest,
requests only the document projections needed by the slice, prefers atomic
semantic batches to repeated pointer gestures, and uses screenshots only for
visual or spatial judgments. After interruption, resume from revisions,
receipts, and artifact hashes rather than relying on conversational memory.

## Loop contract

1. Do not start until `v0.2.0` is committed, pushed, tagged, published, and
   verified; upstream `main` must contain the release and the worktree must be
   clean.
2. The lead reads `AGENTS.md`, the current specification and architecture docs,
   the implementation, and baseline tests before decomposing work.
3. Split the north star into the smallest user-visible vertical slices that can
   be built and judged independently. Model, UI, persistence, tests, and docs
   for one behavior are one slice, not separate completion units.
4. Before building a slice, define its outcome contract: user scenario,
   observable result, positive and negative cases, resource bounds, applicable
   persistence and history behavior, semantic operations and preconditions,
   expected receipts, render/export evidence, and focused verification recipe.
5. Give each builder explicit artifact ownership. Builders preserve concurrent
   work and may not weaken limits, assertions, or expected output to pass.
6. After every builder iteration, use a separate fresh-context critic. Give the
   critic the goal, quality bar, base/head refs, repository rules, and actual
   artifacts, but not the builder's rationale or summary.
7. The critic inspects the running editor, generated files, raster output,
   persisted project, code, tests, and—after control-plane Phase D—its evidence
   bundle. A critique names the largest actionable gap with severity,
   reproduction, expected and actual behavior, and evidence class. Otherwise it
   returns `PASS` and states what remains unverified.
8. Return findings to the builder and send the revised artifacts to another
   fresh critic. Do not choose an arbitrary round count.
9. After a major integration wave, a fresh smoothing critic may fix conflicts,
   interaction inconsistencies, accessibility issues, naming drift, performance
   regressions, and accidental coupling. It does not broaden scope.

Prefer independent PR branches from the latest upstream default branch. Use a
stack only for a real semantic dependency; if independent versus stacked is
ambiguous, ask before publishing branches.

## Workstream lenses

The lead chooses the actual decomposition from repository evidence. These are
useful lenses, not a prescribed architecture:

- **Construct geometry:** boolean/path construction and editing under compound
  paths and transforms.
- **Author appearance:** native gradient, advanced-stroke, and clip creation and
  editing, including selection and hit testing.
- **Control geometry precisely:** editable dimensions, position, rotation,
  scale, skew, mixed states, keyboard entry, and cancelled previews.
- **Ship asset families:** frame/selection targets, reusable export recipes,
  preview, exact scales/dimensions, naming, and batch export.
- **Survive interruption:** bounded autosave, recovery, conflict/staleness
  handling, and protection of the last explicit save.
- **Prove the product workflow:** reproduce the complete acceptance project via
  the UI and semantic automation, then verify persisted and exported artifacts.

## Stop conditions

The Gauntlet stops only when:

- every acceptance workflow is positively exercised with real document data;
- every authored change is undoable and redoable, and cancellation leaks no
  history entry;
- new saved data has a deliberate format-version decision, validation and
  complexity limits, migration/default behavior, and save/load evidence;
- the Brand Kit survives save/load and recovery while remaining intentionally
  editable;
- every required export has exact target bounds and dimensions and passes
  semantic and visual checks;
- GPUI, SVG, and raster output preserve the required appearance within stated
  limits;
- semantic automation can reproduce the project without fixture-specific
  hardcoding;
- the control-plane contracts used by the finished workflow pass their own
  phased acceptance gates; in particular, stale and retried requests do not
  cause unintended edits, and receipts identify actual history/entity effects;
- after control-plane Phase D, a fresh context can discover applicable
  capabilities and reproduce every claimed check from bounded evidence without
  the builder's narrative;
- every latest builder artifact has a fresh critic `PASS`, and an integrated
  smoothing critic has no unresolved Critical or High findings;
- format, strict workspace Clippy, the full workspace tests, Nextest, and audit
  gates pass, with platform claims limited to platforms actually exercised;
- documentation says precisely what is authored, editable, persisted, rendered,
  exported, normalized, rasterized, unsupported, or unverified.

Do not expand this run into general-purpose illustration, speculative
abstractions, or additional rejection messages detached from a positive product
workflow. Images, patterns, masks, filters, and other explicitly unsupported SVG
features are not part of this run.

## Launch prompt

```text
You are the lead agent for Strek's post-v0.2.0 Gauntlet Loop.

First enforce the entry gate: do not edit files until v0.2.0 is committed,
pushed, tagged, published with every claimed platform artifact, upstream main
contains it, required CI/release jobs are green, and the worktree is clean.

Transform Strek into a native logo-and-icon production system. Starting from an
empty document, without importing prebuilt SVG, build the Strek Native Brand Kit
defined in docs/GAUNTLET-RUN.md. The finished project must remain natively
editable, survive save/load and abrupt-termination recovery, and batch-export
deterministic production assets through both the GPUI and semantic automation.

Use the concrete Figma, Affinity, automation, and real-rendering bars in that
document. Decide the architecture and decomposition yourself. Split the goal
into the smallest independently judgeable user-visible vertical slices. For
each important slice, use a builder with explicit ownership and a separate
fresh-context critic that sees the real running output and artifacts but not the
builder's explanation. When our result loses against the bar, fix the largest
meaningful gap and send it to another fresh critic. Keep looping without a fixed
round count until every stop condition in docs/GAUNTLET-RUN.md passes or I stop
the run.

Maintain a compact live evidence ledger showing artifacts, critic verdicts,
tests actually run, and known limitations. Use a fresh smoothing critic after
major integration waves. Follow AGENTS.md exactly; never weaken a gate, inflate
an evidence claim, or count scaffolding and rejection behavior as product
delivery.
```
