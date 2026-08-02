# Design-to-HTML Generation Gauntlet

Status: research and Gauntlet setup complete; implementation has not started. Implementation must not begin until the current release is shipped.

## Objective

Build a design-to-code system that turns Strek layout intent into responsive, accessible, production-quality HTML and CSS. It must infer web layout systems instead of preserving canvas coordinates, produce both Tailwind CSS and framework-independent semantic HTML/CSS targets, identify reusable components and tokens, and emit code a human team could maintain. A per-node absolute-positioned or inline-style transcription is a failed result even when one screenshot happens to match.

## Non-negotiable output targets

1. **Semantic HTML + reusable CSS**: meaningful landmarks and elements, shared classes, custom properties and design tokens, deterministic naming, no repeated style blobs, and no inline `style` attributes except a narrowly documented runtime-data case.
2. **Tailwind CSS**: statically discoverable classes, theme-backed values where repetition establishes a token, arbitrary values only when the design truly has a one-off value, and responsive/container variants that compile without a hidden safelist.
3. **Structured machine representation**: an intermediate layout/component model that explains why each container became block flow, Flexbox, Grid, Subgrid, positioning, or an intrinsic-size construct before either code generator runs.

BEM naming is a supported CSS naming preset, not the semantic model itself. The system must also support a lower-noise component-scoped naming strategy and measure which output is more maintainable.

## Decision questions

1. How should Strek distinguish semantic document structure from purely visual grouping?
2. Which constraints reliably imply normal flow, Flexbox, Grid, Subgrid, or intentional positioning?
3. How should Auto Layout map to Flexbox when the source and CSS sizing models differ?
4. How should the model infer explicit/implicit Grid tracks, named areas, `repeat()`, `minmax()`, `auto-fit`/`auto-fill`, spanning, dense packing restrictions, and nested Grid?
5. When should aligned descendants become CSS Subgrid rather than duplicated tracks or hard-coded dimensions?
6. How should fixed, hug-content, fill, min/max, gap, padding, alignment, wrapping, aspect ratio, and overflow constraints map to intrinsic web sizing?
7. How are breakpoints or container-query thresholds inferred from multiple design frames without overfitting those exact widths?
8. How are repeated subtrees recognized as components while preserving meaningful differences as variants, slots, content, or data?
9. How are typography, colors, spacing, radii, shadows, and breakpoints deduplicated into tokens without inventing a noisy design system?
10. How should semantic elements, headings, landmarks, lists, links, buttons, forms, tables, figures, navigation, and accessible names be inferred and confirmed?
11. How should class names be stable, readable, collision-free, and traceable back to design nodes across BEM, scoped CSS, and Tailwind targets?
12. What fidelity, responsiveness, semantics, accessibility, code quality, deduplication, determinism, and performance measurements form a critic-proof acceptance bar?

## Systems and references to investigate

| System or source | Category | Why it matters |
| --- | --- | --- |
| Figma Auto Layout and Dev Mode | Source constraint model and incumbent handoff | Defines the layout semantics users already expect |
| Penpot Grid Layout and code inspection | Open-source design/editor reference | Direct Grid concepts and inspectable implementation |
| Builder.io Visual Copilot | Design-to-code system | Component/framework generation and responsiveness claims |
| Anima | Design-to-code system | Auto Layout, responsive code, and framework targets |
| Locofy | Design-to-code system | Tagging, components, responsive behavior, and code export |
| Webflow | Visual web authoring and code export | Native semantic/layout authoring and CSS reuse workflow |
| TeleportHQ and other open implementations | Generator architecture reference | Inspectable intermediate representations and serializers |
| Tailwind CSS | Utility output target | Static class detection, theme variables, variants, and container queries |
| Browser layout and accessibility standards | Execution oracle | The output must work as a document, not only resemble an image |
| Design2Code benchmarks and recent research | Independent evaluation ideas | Prevent vendor demos from becoming the only quality bar |

The survey will verify maintenance and evidence quality, add stronger systems, and remove weak references before implementation begins.

## Current Strek starting point

Confidence: **high**, from the current source tree.

Strek currently has a small design-layout vocabulary in [`layout.rs`](../crates/editor_core/src/layout.rs): horizontal or vertical stacks, one gap, physical-side padding, main/cross alignment, and measured fixed child sizes. It has no wrapping, per-child grow/shrink/basis, min/max/intrinsic sizing, Grid, Subgrid, responsive conditions, web semantics, component/slot/variant model, design tokens, or HTML exporter. `Stretch` is currently treated like start alignment rather than implemented. Children in [`node.rs`](../crates/editor_core/src/node.rs) are stored in paint order, with the last child topmost; that cannot silently become DOM reading/focus order.

The honest initial mapping is small:

| Strek today | Possible CSS interpretation |
| --- | --- |
| Horizontal / Vertical | `flex-direction: row / column` |
| `spacing` | `gap` |
| main-axis alignment | `justify-content` |
| cross-axis alignment | `align-items`, except current Stretch is not truthful |
| frame width/height | fixed `inline-size` / `block-size` only |
| text auto width | intrinsic sizing or omitted width, depending on context |
| free transforms/artwork | inline SVG or a justified positioned region, not semantic layout |

This Gauntlet is therefore a new vertical capability, not a serializer bolted onto the current stack enum. It needs native model, persistence, commands/history, UI authoring, Web Layout IR, generators, preview, and browser/a11y verification.

## Evidence to collect, in priority order

1. Public source, schemas, APIs, documentation, generated examples, and reproducible demos.
2. The actual intermediate representation between design nodes and emitted code.
3. Layout inference rules and failure behavior for Flexbox, Grid, Subgrid, intrinsic sizing, wrapping, overflow, and responsive changes.
4. Semantic, accessibility, componentization, tokenization, naming, and style-deduplication behavior.
5. Whether generated output is deterministic, editable, buildable, and maintainable after a human changes it.
6. Visual fidelity across several viewport and container sizes, not only the source frame.
7. Unsupported states and evidence of reward hacking: absolute positioning, screenshot tracing, inline styles, DOM soup, arbitrary-value floods, duplicated markup, or visually hidden semantic failures.

## Per-system report contract

Each report must include:

1. Current product/repository status and primary-source links.
2. Input model, intermediate representation, and output targets.
3. Layout, responsive, component, style, and semantic capabilities.
4. A generated-code sample inspected for structure—not a marketing screenshot.
5. Strengths, limitations, and reproducible failure cases.
6. Lessons Strek should adopt and approaches it should reject.
7. High/medium/low confidence markers for every section.

## Initial implementation principles from current web guidance

- Use Flexbox for one-dimensional, content-driven rows or columns.
- Use Grid for two-dimensional, relationship-driven layouts; use named areas and flexible tracks when they express the design better than line numbers.
- Use Subgrid when grandchildren must align to ancestor tracks; do not emulate it by copying brittle pixel tracks.
- Prefer logical properties, intrinsic sizing, `aspect-ratio`, `gap`, `minmax()`, `fit-content()`, and flexible tracks over hard-coded dimensions.
- Preserve DOM reading and focus order. Never use Flex/Grid reordering or dense packing to fake a visual order for interactive content.
- Use container queries for component context and media queries for global viewport or user preferences.
- Generate native semantic elements before reaching for ARIA or generic containers.
- Treat responsive behavior as a continuum tested between reference frames, not a set of screenshots to memorize.

### Browser-native target rules

The generator should prefer current, broadly available browser primitives and record the target browser policy in its manifest:

- organize emitted CSS into explicit cascade layers such as `reset`, `tokens`, `elements`, `components`, and `utilities`;
- use component scoping where the selected browser policy admits `@scope`; put a deterministic class boundary around the same component for older-policy output;
- use logical properties and values (`inline-size`, `padding-block`, `inset-inline-start`, `start`) so layout survives writing-mode and direction changes;
- use intrinsic sizing (`min-content`, `max-content`, `fit-content()`), `aspect-ratio`, and flexible Grid tracks before fixed dimensions;
- use ranged media/container queries without overlapping conditions;
- make motion opt-in under `prefers-reduced-motion: no-preference`;
- preserve native control and landmark semantics instead of reimplementing behavior with generic elements and ARIA;
- use relational selectors such as `:has()` only when the browser policy admits them and the relationship already exists in the DOM;
- use `oklch()` and token relationships for generated design-system colors when conversion does not change the authored color; preserve authored color syntax when exact round-trip fidelity is the selected contract.

Grid/Subgrid is a semantic layout decision, not a compression trick. A nested component should emit Subgrid when its internal rows or columns intentionally align to ancestor tracks. The child must span the tracks it inherits; otherwise `subgrid` is meaningless.

## Tailwind CSS findings

Confidence: **high**, from current Tailwind documentation.

Current Tailwind scans source files as plain text, recognizes complete class-like tokens, and emits CSS only for recognized utilities. It can generate arbitrary-value utilities, but it cannot infer dynamically constructed fragments. Strek therefore emits complete, static class strings in the generated HTML; it must not produce runtime concatenation such as `"grid-cols-" + count`.

Tailwind v4 supports explicit source registration and `@source inline(...)` safelisting in CSS. Generated Strek projects should not need a hidden safelist because every selected class is present literally in the exported files. Explicit `@source` is allowed only to make a multi-folder build boundary reproducible. See [detecting classes in source files](https://github.com/tailwindlabs/tailwindcss.com/blob/main/src/docs/detecting-classes-in-source-files.mdx).

Theme variables generate corresponding utility classes. Repeated authored values should therefore become named theme tokens; a unique value may use an arbitrary utility when that is more honest than inventing a one-use design-system token. See [theme variables](https://github.com/tailwindlabs/tailwindcss.com/blob/main/src/docs/theme.mdx) and [utility/arbitrary values](https://github.com/tailwindlabs/tailwindcss.com/blob/main/src/docs/styling-with-utility-classes.mdx).

Responsive prefixes apply to layout utilities, and current Tailwind includes container-query variants. A generator can emit a small-layout default plus `sm:`/`md:` viewport changes or `@container`/`@max-*` component changes without writing inline CSS. It should prefer the same Flow/Flex/Grid/Subgrid inference as the CSS serializer; Tailwind is only a syntax target, not a separate layout model.

Tailwind acceptance requires:

- a clean production compile from the exported files alone;
- no dynamically synthesized, missing, or secretly safelisted classes;
- no flood of arbitrary values where repeated values establish a token;
- exact correspondence between the intermediate layout model and emitted utilities;
- a generated CSS size report and unused-class check.

## Source-layout research

### Figma

Confidence: **high** for official layout/API fields; **low** for production-code quality claims not backed by export artifacts.

Figma Auto Layout provides horizontal, vertical, wrap, gap, four-sided padding, alignment, fixed/hug/fill sizing, and independent min/max bounds. Grid Auto Layout remains beta but exposes row/column counts, CSS-shaped track-sizing strings, gaps, spans, anchors, manual versus row-auto-flow positioning, and automatic row creation. It does not expose named areas, implicit columns, or Subgrid as first-class fields. See [Auto Layout](https://help.figma.com/hc/en-us/articles/360040451373-Explore-auto-layout-properties), [Grid Auto Layout](https://help.figma.com/hc/en-us/articles/31289469907863-Use-the-grid-auto-layout-flow), and [REST node fields](https://developers.figma.com/docs/rest-api/file-node-types/).

Dev Mode and `getCSSAsync()` produce property snippets, not standalone semantic pages. Code Connect's more valuable idea is mapping design components/variants to existing repository components and props. See [Dev Mode](https://help.figma.com/hc/en-us/articles/15023124644247-Guide-to-Dev-Mode), [`getCSSAsync`](https://developers.figma.com/docs/plugins/api/PageNode/), and [Code Connect](https://developers.figma.com/docs/code-connect/).

Adopt fixed/hug/fill plus min/max, explicit flow escape, CSS-shaped track input, and code-component mappings. Do not treat Dev Mode CSS snippets or layer-name guesses as semantic production output.

### Penpot

Confidence: **high**, based on official docs and open source at commit [`f023c09`](https://github.com/penpot/penpot/commit/f023c090187b4f7c847fc11e5da37e008b8b0265).

Penpot is the closest open design-model reference. Flex/Grid use CSS terminology. Grid tracks are typed as percent, flex, auto, or fixed; cells carry row/column, spans, auto/manual/area placement, area names, and self-alignment. Its CSS generator emits real Flex/Grid, logical padding/margins, templates, areas, spans, and auto-flow. See the [layout schema](https://github.com/penpot/penpot/blob/f023c090187b4f7c847fc11e5da37e008b8b0265/common/src/app/common/types/shape/layout.cljc), [layout guide](https://help.penpot.app/user-guide/designing/flexible-layouts/), and [CSS generator](https://github.com/penpot/penpot/blob/f023c090187b4f7c847fc11e5da37e008b8b0265/frontend/src/app/util/code_gen/style_css_values.cljs).

Its model does not provide typed `minmax()`, `repeat(auto-fit/auto-fill)`, `fit-content()`, Subgrid, media conditions, or container conditions. Its HTML generator is also not the output bar: it relies heavily on `div` wrappers and UUID-suffixed selectors and does not compile editor components/tokens into reusable semantic components. See its [HTML generator](https://github.com/penpot/penpot/blob/f023c090187b4f7c847fc11e5da37e008b8b0265/frontend/src/app/util/code_gen/markup_html.cljs). Penpot is MPL-2.0, so Strek may study its model and clean-room adapt ideas; copied/modified files require a separate licensing decision.

### Webflow

Confidence: **high** for documented features and export contract; **medium** for absence claims.

Webflow proves visual authoring can retain [semantic tags](https://help.webflow.com/hc/en-us/articles/33961369965715-Semantic-HTML5-tags), reusable [classes](https://help.webflow.com/hc/en-us/articles/33961311094419-Classes), [components/variants/slots](https://help.webflow.com/hc/en-us/articles/33961303934611-Components-overview), responsive [variables](https://help.webflow.com/hc/en-us/articles/33961268146323-Variables), and separate [HTML/CSS/JS export](https://help.webflow.com/hc/en-us/articles/33961386739347-How-do-I-export-my-Webflow-site-code). Its Grid supports explicit/implicit tracks, spans, negative lines, min/max tracks, `auto-fit`, and named areas; Flex supports wrap, basis/grow/shrink, gap, and alignment. See [Grid](https://help.webflow.com/hc/en-us/articles/33961365794451-Grid), [Grid areas](https://help.webflow.com/hc/en-us/articles/33961389959059-Grid-areas), and [Flexbox](https://help.webflow.com/hc/en-us/articles/33961260795155-Flexbox).

No native Designer UI for Subgrid or container queries was verified. Current APIs can set broad CSS properties, but that is an escape hatch, not a typed visual model. Strek should adopt explicit tags, reusable class storage, components/slots/variants, breakpoint cascade, areas, `minmax()`, `auto-fit`, and full-project export while avoiding runtime metadata and platform dependencies in static output.

### Source-model matrix

`Full` means a documented native model; `Partial` means beta, bridge, or incomplete coverage.

| Capability | Figma | Penpot | Webflow | Strek target |
| --- | --- | --- | --- | --- |
| Semantic HTML | Component bridge only | Missing | Full | Full |
| Normal flow | Missing | Partial | Full | Full |
| Flex wrap/gap/alignment | Full | Full | Full | Full |
| Grow/shrink/basis | Partial | Partial | Full | Full |
| Explicit Grid/spans | Beta | Full | Full | Full |
| Implicit tracks/auto-flow | Rows only | Partial | Full | Full |
| Named areas | Missing | Full | Full | Full |
| `minmax()` / auto-fit/fill | CSS-shaped string | Missing | Full | Full |
| Intrinsic sizing | Hug/fill vocabulary | Fit/fill vocabulary | Partial | Typed/full |
| Subgrid | Missing | Missing | No native UI verified | Full |
| Viewport conditions | Figma Sites | Missing | Full | Full |
| Container conditions | Missing | Missing | No native UI verified | Full |
| Components/variants/slots | Full | Full | Full | Full |
| Tokens | Variables | DTCG tokens | Variables/modes | DTCG + CSS/Tailwind output |
| Standalone clean HTML/CSS | Missing | Div-heavy snippets | Full but runtime-heavy | Full |
| Tailwind output | Plugin territory | Missing | Missing | Full |

## Generator and compiler research

No reviewed generator publicly proves the complete target: semantic HTML, reusable/deduplicated CSS or BEM, Tailwind, Flex, Grid, Subgrid, container queries, deterministic builds, and a guarantee against coordinate dumps. Subgrid and container-query inference are a clear opportunity for Strek.

| System | Input / architecture / output | Verified strengths | Missing or weak evidence | Strek use |
| --- | --- | --- | --- | --- |
| Builder Visual Copilot/Fusion | Figma + proprietary AI/import, open Mitosis compiler; many frameworks/CSS targets | Existing-component/token indexing and mapping | Current generated project corpus; Grid/Subgrid/container mapping; semantic claims | Workflow reference; Mitosis architecture reference |
| Anima | Figma/site/prompt + proprietary service; public SDK; HTML/React and CSS/Tailwind/inline modes | Auto Layout to Flex, multi-frame responsive inputs, design variables | Subgrid/container queries; initial semantic/a11y quality; SDK reuse license | Product workflow reference only |
| Locofy | Figma/Penpot + proprietary design IR/heuristics; framework/HTML and styling variants | Explicit Grid editor, multi-frame breakpoints, semantic tagging, component mapping | Current inspectable output; Subgrid/container queries | Workflow/failure-case reference |
| TeleportHQ | Typed UIDL + open deterministic generators | Strong IR/compiler split, semantic primitives, components/props/tokens, reusable CSS/media queries | Layout inference and first-class Subgrid/container-query generation | Primary open compiler reference |
| FigmaToCode | SceneNodes -> AltNodes IR -> several code targets | Inspectable normalization and Auto Layout to Flex | HTML uses inline styles; non-layout fallback is absolute; basic semantics; no Grid/Subgrid | Inspectable negative/architecture reference |
| Webflow | DOM-aware authoring -> static project export | Strong semantic/class/token/component contract | Not a design-inference engine; no Tailwind/Subgrid/container model verified | Human-authored output bar |
| Plasmic | Visual editor/import -> internal model -> React/CSS/runtime | Existing-code component integration, variants, slots | No verified Tailwind/Subgrid/container export | Adjacent component architecture reference |

Evidence:

- Builder documents [generation targets](https://www.builder.io/c/docs/generate-code), [component/token indexing](https://www.builder.io/c/docs/component-indexing), and [Figma component mapping](https://www.builder.io/c/docs/figma-components). Its [Mitosis compiler](https://github.com/BuilderIO/mitosis) is MIT and open; the quality generation stage is not.
- Anima's public [SDK](https://github.com/AnimaApp/anima-sdk) exposes responsive frame sets, HTML/React targets, Tailwind/plain/inline CSS choices, component splitting, and assets. Official docs state [Auto Layout becomes Flexbox](https://docs.animaapp.com/docs/starting-from-figma) and [Tailwind output derives configuration from tokens](https://docs.animaapp.com/docs/tailwind-css). No license file was found in the SDK repository, so source visibility is not permission to reuse.
- Locofy documents [Grid synthesis](https://www.locofy.ai/docs/classic/tagging/examples/working-with-grids/), [multi-frame breakpoints](https://www.locofy.ai/docs/lightning/multiple-breakpoints/), [semantic tagging](https://www.locofy.ai/docs/classic/tagging/code-with-semantic-tags/), and [mapping to real code components](https://www.locofy.ai/docs/cli/design-system/overview/). Its old public “unedited” [Tailwind example](https://github.com/ishivamshukla/locofy-ClarityUI/blob/main/src/pages/Component.tsx) is dominated by fixed absolute layers; it is too old to grade the current product but is an excellent anti-cheating fixture.
- TeleportHQ's MIT [UIDL/generator repository](https://github.com/teleporthq/teleport-code-generators) separates representation, component generation, project generation, and packing. Its [CSS generator](https://github.com/teleporthq/teleport-code-generators/blob/development/packages/teleport-plugin-css/src/index.ts) emits reusable classes, referenced styles, tokens, and media queries; its [HTML mapping](https://github.com/teleporthq/teleport-code-generators/blob/development/packages/teleport-uidl-resolver/src/html-mapping.ts) includes real form, link, list, media, and button primitives.
- The GPL-3 [FigmaToCode](https://github.com/bernaferrari/FigmaToCode) maps Auto Layout to Flex in its [backend](https://github.com/bernaferrari/FigmaToCode/blob/main/packages/backend/src/html/builderImpl/htmlAutoLayout.ts) and makes children absolute when a parent has no layout model in its [fallback](https://github.com/bernaferrari/FigmaToCode/blob/main/packages/backend/src/common/commonPosition.ts). It is an inspectable warning against direct canvas transcription, not source to port.
- [Plasmic](https://github.com/plasmicapp/plasmic) is MIT outside its AGPL `platform/` subtree. Any reuse requires a path-specific license audit.

### Cross-system decisions

1. Use TeleportHQ as the open compiler/IR reference, Penpot as the typed layout-model reference, Webflow as the semantic/class/token/export bar, Tailwind's own current compiler model as the utility contract, and hand-authored responsive reference projects as the judge.
2. Map authored/existing components before inferring new ones. Exact design-to-code component mapping wins; structural deduplication is the fallback.
3. Make semantic tag/role authoring explicit and auditable. Conservative inference may recommend, but not silently decide, ambiguous semantics.
4. Never make “AI cleanup” the only route to valid semantics, accessibility, responsiveness, or a clean build.
5. Produce separate first-class serializers. A Tailwind request must not silently fall back to inline CSS; a BEM request must not emit Tailwind-like per-value classes.

## Proposed intermediate representation

The exporter must not reason directly from canvas nodes to strings. It should first produce an inspectable, serializable Web Layout IR:

```text
WebDocument
  browser_policy
  pages[]
  component_definitions[]
  design_tokens[]
  assets[]

WebNode
  provenance: source node IDs and inference evidence
  semantic: element, accessible name, landmark, heading/list/form relationships
  content: text, rich spans, media, data/slot binding
  component: definition, instance, variant, slot, repeated-data identity
  layout:
    Flow
    Flex(axis, wrap, gap, distribution, alignment)
    Grid(explicit/implicit tracks, areas, auto-flow, spans)
    Subgrid(inherited axes, claimed spans)
    Positioned(anchor/containing block, reason)
  sizing: fixed/intrinsic/fill/min/max/aspect/overflow
  responsive_rules[]: viewport or named-container condition + structural/style delta
  visual_style: token references plus exact local values
  confidence: per semantic/layout/component inference
```

Provenance is mandatory. A critic and user must be able to select an emitted node and see which design nodes and constraints caused its semantic element, layout mode, component extraction, token, class, and responsive rule.

### Layout inference contract

1. Recover likely reading/DOM order from content relationships, then evaluate visual order. DOM order wins for keyboard and assistive technology.
2. Detect repeated alignment lines and two-axis relationships. Use Grid when row and column relationships constrain placement; use Flex only for a single content-driven axis.
3. Infer tracks from repeated boundaries and sizing behavior, not from a screenshot-only pixel partition. Express proportional tracks as `fr`, flexible bounds as `minmax()`, and repeatable card tracks as `repeat(auto-fit, minmax(...))` when the evidence supports reflow.
4. Infer Subgrid when nested descendants share ancestor track boundaries across sibling components. Require at least two aligned descendants and a claimed parent span; otherwise retain a normal nested Grid.
5. Map hug-content to intrinsic sizing, fill to flexible growth/tracks, fixed to an explicit bound only when content stress tests remain valid, and min/max to CSS constraints.
6. Use absolute/fixed/sticky positioning only when overlap or anchoring is intentional and cannot be represented by flow layout. Record the reason in provenance.
7. Infer responsive rules from multiple frames and content constraints. Prefer the smallest set of non-overlapping conditions that explains all references and all widths between them.
8. Use container queries for reusable component changes driven by local space; use media queries for page shell, viewport, input mode, and user preference.

Ambiguous semantics or layout receive a confidence score and a deterministic recommendation. The UI must offer explicit correction; it must not silently use an LLM guess as ground truth.

### Component and style deduplication contract

- Canonicalize subtrees by semantic structure, layout constraints, style-token references, and slot positions—not by screenshot similarity alone.
- Extract a component only when reuse reduces code and variants can describe the differences without conditional soup.
- Preserve authored component identities when available; inferred components cannot override an explicit source boundary.
- Promote recurring typography, color, spacing, radius, shadow, and breakpoint values to tokens when the recurrence is meaningful. Near values are not merged merely to improve a score.
- Fingerprint declaration sets and layout rules. Equivalent component rules share a class or component scope; repeated text values and user content remain data, not duplicated markup.
- Keep a manifest mapping component/token/class names to source nodes so names remain stable across regenerated exports.

## Serializer contracts

### Semantic HTML + scoped CSS

- native landmarks, headings, links, buttons, lists, forms, tables, figures, and media elements;
- deterministic readable component classes;
- design tokens as custom properties;
- cascade layers and component scopes under the chosen browser policy;
- no inline `style` attributes in ordinary output;
- no source-order changes for visual convenience.

### BEM preset

- stable `block`, `block__element`, and `block--modifier` names derived from semantic/component identities;
- no BEM element chain mirroring every design-layer nesting level;
- modifiers represent meaningful variants, not individual declarations;
- shared tokens and global element rules stay outside blocks.

### Tailwind CSS

- static complete utilities in source;
- theme-backed repeated values and sparing arbitrary values;
- the same IR and DOM as semantic CSS output;
- responsive and named-container variants where the IR requires them;
- extracted source components when repeated markup would otherwise dominate the output.

## Benchmark corpus

The checked-in corpus must include editable source designs plus reference content variants:

1. marketing page with hero, navigation, cards, pricing, testimonials, and footer;
2. application shell with responsive sidebar, toolbar, panels, dialog, menu, and data views;
3. editorial article with heading hierarchy, figures, quotations, lists, footnotes, and long-form type;
4. labeled forms with validation, help, disabled states, fieldsets, and dynamic status;
5. dense data table that must remain a table and work under narrow overflow;
6. repeated card system whose internal title/body/action rows require Subgrid alignment;
7. dashboard requiring explicit Grid areas and nested Grid/Flex composition;
8. gallery requiring intrinsic tracks and content-driven reflow;
9. internationalized/RTL and vertical-writing samples;
10. adversarial content: 200% text zoom, long unbroken tokens, missing/very long copy, one-to-many list data, absent images, and reordered variants;
11. visually similar layouts with different semantics, proving screenshots alone are insufficient;
12. intentionally overlapping artwork proving that justified positioning remains available.

For responsive references, include at least three authored widths but score a continuous sweep between and beyond them. Components are also tested inside narrow and wide containers independently of the viewport.

## Provisional hard gates

| Dimension | Required result |
| --- | --- |
| Build | Semantic CSS and Tailwind projects compile from a clean checkout with no manual edits or hidden configuration |
| Visual | Reference-width renders pass the corpus-calibrated pixel/SSIM gate; no unexplained region may be masked from comparison |
| Responsive | Automated sweep from 320 to 1920 CSS px plus container-width sweeps has no accidental overflow, overlap, clipping, unreadable line length, or control loss |
| Content stress | 200% text zoom, long/short/empty copy, repeated data, RTL, and missing media preserve content and operation |
| Semantics | Required native elements, landmark uniqueness, heading order, list/table/form relationships, names and descriptions match the annotated source truth |
| Accessibility | axe has zero serious/critical findings; keyboard order follows DOM/visual reading order; focus is visible and every control is operable |
| Layout quality | Every positioned node carries a justification; ordinary content uses Flow/Flex/Grid/Subgrid; Grid and Subgrid benchmark cases emit the required model |
| Inline styles | Zero ordinary inline `style` attributes; documented runtime-data exceptions are counted and separately approved |
| Deduplication | Repeated source components map to one definition; repeated declaration fingerprints map to shared tokens/rules; no copy-pasted subtree survives without an explicit reason |
| Tailwind | All emitted classes are statically detected; production CSS contains them; arbitrary-value and token counts are reported |
| Determinism | Same source and options produce byte-identical IR/HTML/CSS across 100 runs and supported CI platforms |
| Regeneration | A human-owned fixture change outside generated regions survives regeneration; generated diffs are stable and local |
| Provenance | Every emitted element, component, token, class, and responsive rule maps back to source evidence and inference confidence |

Exact visual and code-complexity thresholds must be calibrated on the human-authored reference corpus before the first builder wave. Calibration is allowed; deleting a hard-gate dimension is not.

## Benchmark research

No published benchmark is a production HTML quality bar. The useful parts become diagnostic critics inside Strek's larger gate.

| Benchmark | Evidence we should reuse | What it does not prove |
| --- | --- | --- |
| [Design2Code](https://github.com/NoviScl/Design2Code) | 484 real pages plus 80 hard cases; element block recall, text, normalized position, perceptual color, and human-ranking validation | One static screenshot; scripts/SVG/external resources removed; no responsiveness, semantics, accessibility, interactions, or CSS quality |
| [UIBenchKit](https://github.com/chinh02/UIBenchKit) | Reproducible model/method/render/metric wrapper and artifact-oriented comparison | Mostly wraps Design2Code/DCGen; CLIP and code-string similarity remain gameable; successful-only averages can hide render failure |
| [1D-Bench](https://arxiv.org/abs/2602.18548) | Fixed executable React scaffold, CSS Modules/components, no inline styles, and final score multiplied by render-success rate | Tight screenshot/root resizing can hide cropping; no semantic/accessibility/maintainability score; artifact was inaccessible during research |
| [Vision2Web](https://github.com/zai-org/Vision2Web) | Strongest open responsive/functional comparator: desktop/tablet/mobile references, one implementation, executable workflows | Uses visual-model judges and does not grade DOM/CSS architecture |
| [DesignBench](https://github.com/WebPAI/DesignBench) | Generation/edit/repair diversity across React, Vue, Angular, and vanilla output | Canonical-code/patch similarity is not maintainability |
| [FullFront](https://github.com/Mikivishy/FullFront), [FronTalk](https://github.com/shirley-wu/frontalk), [WebGen-Bench](https://github.com/mnluzimu/WebGen-Bench) | Interaction authoring, refinement, multi-turn forgetting, and multi-file workflow tests | Do not close the responsive/semantic/CSS-architecture gap |

Confidence is high for directly inspected public paper/source claims. FigmaBench and some 1D-Bench artifacts were inaccessible during research, so their claimed results are not release evidence.

Design2Code's own paper explains why one code-similarity score is wrong: many implementations can render the same valid design, while a small textual change can cause a large visual change. Its fine-grained Block-Match, text, position, and CIEDE2000 color diagnostics are better than CLIP alone. UIBenchKit likewise reports that global CLIP similarity can hide layout errors. Strek will use those diagnostics but add executable, semantic, responsive, and code-architecture gates.

### Anti-gaming defenses

- CLIP is diagnostic only. It is crop/resize/theme tolerant and Design2Code deliberately masks text.
- Global SSIM or pixel averages cannot dominate because large blank backgrounds hide local failures. Score named semantic regions and worst tiles.
- The reference screenshot is unavailable to generated runtime code. Reject page-covering raster/canvas reconstruction except explicitly authored art; hash and near-duplicate-check assets.
- Code-string similarity has zero release weight. Dead/copied code must not improve a result.
- Reject a hidden “semantic” shadow tree behind a visual raster. Every scored semantic node must be visible and, when interactive, hit-testable and mapped to a design node.
- Source keywords do not prove primitive selection. Inspect computed styles and metamorphic behavior on mapped visible nodes.
- A container-query fixture renders two copies at the same viewport inside different-width hosts and must produce different layouts. Media queries cannot fake it.
- A Subgrid fixture changes ancestor tracks and requires descendant alignment to follow without regenerated child tracks.
- Aggregate each case by its worst viewport/state. Publish p10 and every hard-fail case; averages cannot hide one catastrophic width.

### Full execution gate

Both external-CSS/BEM and Tailwind tracks pass independently:

1. Clean offline build with a pinned lockfile. Chromium, Firefox, and WebKit render with no page, console, or resource errors. Network is restricted to checked-in local assets.
2. Nu HTML Checker reports zero errors; warnings are zero or exact-version allowlisted. A current CSS parser/linter and successful target-browser parsing/computed behavior gate CSS because legacy validators can lag CSS modules.
3. External mode has zero `style` attributes, embedded `<style>` blocks, CSS-in-JS, `.style` mutation, or `setAttribute('style')`. Tailwind still emits a linked compiled stylesheet.
4. The set of computed absolute/fixed design nodes exactly matches the fixture's authored overlay/freeform allowlist. Normal content positioned by coordinate/inset dumps fails.
5. Flex, Grid, Subgrid, and container-query fixtures prove the expected primitive through computed style and resize behavior.
6. Use one DOM, not duplicated mobile/desktop trees. Test fixed viewports 240, 280, 320, 360, 390, 480, 640, 768, 1024, 1280, 1440, and 1920 px; 24 deterministic stratified random widths; and `breakpoint - 1`, breakpoint, `breakpoint + 1` for every emitted condition. Test container widths 240, 320, 480, 640, and 800 px.
7. At every sample, fail unintended overflow, clipping, overlap, missing elements, or off-screen focus. Repeat under long/short/empty/localized content, 200% text zoom, missing media, and RTL.
8. Authored tag, computed role, accessible name/state, relationships, and meaningful DOM order match fixture truth. No positive `tabindex`. Axe has zero violations and zero unadjudicated incomplete results; keyboard/focus/order/manual checks remain mandatory because automation is incomplete.
9. BEM/component mode enforces the configured naming grammar, with no hashes, layer IDs, numeric-only names, ID selectors, duplicate selectors, exact duplicate declaration blocks, or `!important`. Names remain stable under sibling reorder and style-only edits.
10. Labeled equivalent style/component signatures achieve at least 95% reuse recall and 100% precision. Every labeled token is emitted once; a non-allowlisted literal repeated at least three times becomes a token. Unused CSS is at most 10% across the union of tested states.
11. Tailwind emits complete static candidates; every candidate produces a compiled rule. No dynamic class fragments. Authored token values use theme utilities/variables; arbitrary values are limited to true off-theme values.
12. Export three times in clean directories with different absolute paths/locales. Relative file sets and bytes are identical. No timestamps, random suffixes, path leakage, or ordering drift.
13. Static fixtures run with no JavaScript. Compressed HTML+CSS+JS and DOM element count stay within 1.25x the expert reference; DOM depth stays within reference + 2. Pinned Lighthouse score is at least 90, with lab LCP <= 2.5 s, CLS <= 0.1, TBT <= 200 ms, and scripted interaction response <= 200 ms. INP remains a field metric rather than a falsely claimed lab result.

Use relevant pinned [Web Platform Tests reftests](https://web-platform-tests.org/writing-tests/reftests.html) to qualify each browser environment, [Nu HTML Checker](https://validator.github.io/validator/docs/vnu.1.html) for markup, [axe-core](https://github.com/dequelabs/axe-core/blob/develop/doc/API.md) plus WAI keyboard/manual checks for accessibility, and current [Core Web Vitals](https://web.dev/articles/vitals) definitions for performance claims.

### Fidelity calibration

Provisional candidate thresholds, per pinned browser/font/asset environment:

- visible element/text precision and recall: 1.00;
- bounding-box edge error: p95 <= 2 CSS px and max <= 4 px on key elements;
- required layout-relation/topology assertions: 100%;
- same-engine fuzzy pixel diff: max channel delta <= 8 and differing pixels <= 0.5% of canvas;
- global SSIM >= 0.985 and worst named semantic-region SSIM >= 0.95;
- text exact and CIEDE2000 p95 <= 3.

Calibrate with accepted alternate hand implementations and repeated gold renders, then inject must-fail mutations: missing/renamed control, semantic `div` swap, one broken intermediate width, screenshot raster, coordinate dump, inline style, duplicated CSS, dynamic Tailwind token, and unused fake Subgrid/container declarations. Choose per-engine thresholds with zero false accepts on mutations and at most 5% false rejects on accepted alternatives; lock them before the builder starts. Keep a hidden holdout/mutation set.

### Multi-critic scoreboard

Hard-gate eligibility stays separate from the score. Eligible output is scored out of 100:

| Critic | Points |
| --- | ---: |
| Visual raster fidelity | 20 |
| Element/layout geometry | 15 |
| Continuous responsive behavior and primitive choice | 15 |
| Semantics, accessibility, and keyboard behavior | 20 |
| Stylesheet architecture, naming, and deduplication | 15 |
| Execution, standards, and determinism | 10 |
| Performance and resilience | 5 |

Each case uses its worst viewport/state. Rollup uses a weighted geometric mean multiplied by render-success rate. “Production ready” requires every hard gate, 100% render success, every critic >= 85, overall >= 90, and corpus p10 >= 85. Publish the separate critics and failed artifacts, never only the composite.

## Gauntlet workstreams

1. Browser harness, annotated corpus, mutation suite, and live scoreboard.
2. Native web semantics, DOM order, and accessibility authoring.
3. Flow/Flex sizing model: wrap, grow/shrink/basis, intrinsic/min/max, overflow, logical axes.
4. Grid: typed tracks, areas, implicit/explicit placement, spans, `repeat()`, `minmax()`, auto-fit/fill.
5. Subgrid, container conditions, media conditions, and multi-frame inference.
6. Components, variants, slots, props/data, design tokens, and provenance.
7. Semantic HTML + external/scoped CSS serializer.
8. BEM serializer and naming-stability/dedup passes.
9. Tailwind serializer, theme generation, static-candidate proof, and isolated build.
10. Export preview/diagnostics UI, browser diffs, and ambiguity correction.
11. Project export, deterministic regeneration, automation, and round-trip persistence.
12. Fresh smoothing critic over the complete editor and all generated projects.

Each important workstream gets a builder and a separate fresh critic. Use independent visual, responsive/layout, semantic/accessibility, and code-architecture critics. Critics inspect generated projects and running browsers, not screenshots supplied by builders. They identify the largest remaining gap and return it for another round; there is no fixed round count.

## Entry gate and branch strategy

Do not start implementation until v0.2.0 is committed, pushed, tagged, released, and verified on Linux, macOS, and Windows. Start this Gauntlet from that released `main` commit in its own independent branch and worktree. Do not stack it on the editor-product or SVG-optimization Gauntlet unless a later concrete semantic dependency makes independent testing impossible.

## Copy-paste run prompt

```text
Turn Strek into the best design-to-production HTML system: native layout and
semantic intent must export as responsive, accessible, maintainable projects,
not canvas coordinates.

The bar is the committed expert-reference corpus and browser harness in this
document. One typed Web Layout IR must generate both semantic HTML with
external reusable CSS/BEM and semantic HTML with statically discoverable
Tailwind. Flex, Grid, Subgrid, intrinsic sizing, viewport conditions and
container queries are first-class. Repeated designs become components, slots,
variants and tokens. Inline-style dumps, coordinate-positioned normal content,
duplicated responsive DOM trees, div soup, dynamic Tailwind classes and
screenshot tracing fail regardless of visual score.

Divide the goal into the smallest independently judgeable pieces. Give every
important piece a builder and a separate fresh critic. Critics inspect the
actual generated projects in Chromium, Firefox and WebKit across the complete
viewport/container/content mutation matrix. Keep separate visual, responsive,
semantic/accessibility and code-architecture critics. Every hard gate must pass
in both output tracks; no critic can be averaged away. Maintain a live progress
page with source designs, generated code, browser renders, diffs, accessibility,
deduplication, build and performance evidence. Keep looping until Strek wins
the bar or I stop the run.
```

## Required synthesis before implementation

The completed document will add:

- a feature and architecture matrix across all researched systems;
- a proposed intermediate representation for semantics, layout constraints, components, variants, tokens, and provenance;
- deterministic inference rules with confidence scores and an explicit user-confirmation path for ambiguity;
- three serializer contracts: semantic HTML/CSS, BEM CSS, and Tailwind CSS;
- a benchmark corpus spanning landing pages, application shells, forms, editorial content, repeated cards, dense data, and deliberately adversarial layouts;
- exact visual, responsive, semantic, accessibility, deduplication, maintainability, and build thresholds;
- independent builder and critic loops, with separate visual, code-quality, accessibility, and responsive critics;
- a short copy-paste Gauntlet prompt.

## Provisional rejection conditions

Any candidate fails a round if it:

- uses absolute positioning for content that has a stable Flow, Flexbox, Grid, or Subgrid solution;
- emits inline styles as the normal representation;
- uses generic `div`/`span` markup where the design contains clear interactive or document semantics;
- changes DOM order to obtain the desired visual order;
- duplicates a repeated component or recurring declaration instead of factoring it;
- emits Tailwind class names that the compiler cannot statically discover;
- matches only the authored frame while overflowing, collapsing, or becoming unusable between test widths;
- passes screenshot comparison while failing keyboard navigation, accessible-name, heading-order, form-label, or landmark checks.
