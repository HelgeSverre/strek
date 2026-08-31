# Agent Control Plane

This document defines how Strek becomes an editor that an agent can understand,
control, verify, and improve without reconstructing hidden UI state or guessing
which commands are safe. It is a product architecture, not a second editor
model and not a project-management ledger.

Status: in progress. Protocol version 2 implements session identity, three
revision classes, guarded stale-write rejection, stable guard error codes, a
bounded sequenced-response retry cache with explicit expiry, compact
revision/effect receipts, and
a runtime operation manifest with typed parameter summaries, effect/cost/
authority classes, derived limits, current availability, and bounded entity
queries with opt-in style and geometry projections. Full generated result
schemas and changes-since-revision queries remain Phase B work.
[`automation.md`](automation.md) remains the authority for the exact current
surface and limitations. Each remaining contract below becomes a current claim
only after its named delivery phase passes the stated vertical gates.

The control plane is successful when the same semantic operation can be invoked
by a person in GPUI or by automation, produces the same core mutation and
history entry, and returns enough structured evidence to decide the next action
without a screenshot unless appearance is actually under review.

## The agent's control problem

An effective agent repeatedly answers six questions:

1. What document and editor state exists now?
2. What outcomes and operations are available in that state?
3. What will an operation read, change, invalidate, and record in history?
4. Did the requested change occur against the state the agent inspected?
5. Which evidence proves the intended semantic, persistence, and visual result?
6. What durable knowledge should make the next run cheaper and safer?

Today these answers are distributed across `get_state`, `get_document`, command
documentation, screenshots, exported files, tests, and source inspection. That
is workable, but it costs calls and encourages brittle coordinate automation.
The control plane joins those surfaces through stable identifiers, revisions,
capability descriptions, change receipts, and verification recipes.

## Tower of linked abstractions

```text
user outcome
    │ decomposes into named capabilities and observable acceptance conditions
    ▼
capability manifest
    │ advertises applicable queries, operations, preconditions, and evidence
    ▼
semantic query + operation protocol
    │ targets stable document entities and core domain commands
    ▼
editor transaction + history
    │ validates, previews, commits, cancels, and increments revisions
    ▼
document + derived scene
    │ is the single source for persistence, hit testing, display, and export
    ▼
verification recipe
    │ observes semantic state, round trips, render output, and bounded failures
    ▼
evidence bundle
      records inputs, revisions, receipts, artifacts, checks, and limitations
```

Every layer speaks in terms defined by the layer below it. A capability does not
claim a private implementation path; an operation does not mutate a second
automation-only model; a receipt does not claim evidence the verification layer
did not produce.

## Design principles

### Observe semantically, render selectively

Structured state is the default observation surface. Screenshots and raster
comparisons are reserved for spatial and visual questions. This keeps most
loops deterministic, compact, and platform-independent while retaining real
visual evidence where fidelity matters.

Queries support projections and change cursors so an agent can request the
selection, particular nodes, authored styles, bounds, or capabilities without
re-reading the whole document. A full snapshot remains available for recovery
and debugging.

### Prefer intent over gesture

Stable operations express outcomes such as setting bounds, applying a saved
color, performing a boolean union, or exporting a frame. Pointer input remains
for inherently spatial work and for exercising the actual interaction path. It
is not the only route to a domain behavior that has semantic parameters.

The UI and automation adapters call the same core operation. Each committed
authored operation creates the same undoable history unit regardless of caller.

### Make concurrency explicit

Every state snapshot reports three monotonic revisions:

- `document_revision`: committed authored document changes;
- `workspace_revision`: selection, viewport, tool, and presentation changes;
- `artwork_revision`: the settled renderable artwork snapshot.

The snapshot also reports an unguessable `session_id` that changes when the
document is replaced or the process restarts. A mutating request may include
that ID plus `if_document_revision` and, when relevant,
`if_workspace_revision`. A mismatch fails without mutation and returns the
current session and revisions. Agents can therefore use optimistic concurrency
rather than silently editing stale targets. Revisions are session-local and
never become part of the native document format.

### Return effects, not acknowledgements

Every successful mutation returns a structured receipt containing:

- operation name and normalized arguments;
- before and after revisions;
- created, updated, deleted, selected, and invalidated entity IDs;
- history outcome: committed entry, preview only, no-op, or workspace-only;
- warnings and lossy conversions, if any;
- compact projections of values needed to verify the postcondition.

Errors use stable machine codes plus human explanations and recovery hints.
Expected revision, disabled capability, invalid target, unsupported feature,
resource limit, and platform limitation are distinct failures.

### Batch atomically, stream bounded work

An agent can submit a list of semantic operations as one transaction with a
shared precondition. Either all operations commit as one undo entry or none do.
References may bind to IDs created earlier in the same batch. This eliminates
many inspect/mutate round trips without weakening validation.

Long-running import, export, and render work uses bounded jobs with progress,
cancellation, and final receipts. Jobs do not expose unbounded logs as state.

### Accrete executable knowledge

Durable knowledge belongs at the narrowest authoritative layer:

- invariants and domain semantics live in core types, validation, and tests;
- operation schemas and capability metadata live beside their handlers and are
  generated into CLI/MCP documentation;
- user workflows live as executable scenarios using public semantic operations;
- visual expectations live in reviewed fixtures or differential tests;
- product limits live once in code constants and are surfaced by capabilities.

An agent run should leave a reusable scenario, regression, schema annotation,
or clarified invariant when it discovers a durable fact. It should not append a
free-form memory file that can drift independently of code.

## Contract surfaces

### Session descriptor

The first query returns a compact `SessionDescriptor`:

- application and protocol versions;
- process and document identity;
- document, workspace, and artwork revisions;
- dirty state and active interaction;
- platform capabilities and unavailable evidence classes;
- complexity counters and applicable limits;
- links or names for the capability manifest and larger projections.

This is enough to resume safely after context loss. It does not include the
entire scene tree.

### Capability manifest

Capabilities are self-describing, versioned records generated from the command
and automation registry. A record includes:

- stable ID, summary, and semantic version;
- parameter and result schemas with units, ranges, defaults, and examples;
- applicability predicate and current enabled/disabled reason;
- whether it changes document, workspace, history, or filesystem state;
- atomicity, idempotency, cancellation, and expected cost class;
- required platform support and relevant resource limits;
- verification recipes and the evidence classes they can establish.

The manifest describes what the running binary can do. Documentation explains
why and how to combine those capabilities, but is not the discovery protocol.

### Addressing and queries

Runtime node IDs remain opaque and session-local. Queries additionally support
stable authored locators composed from explicit names or IDs plus ancestry and
type predicates. Ambiguous locators fail and return bounded candidates; they do
not choose the first match. Created entities can receive caller correlation IDs
for use within batches and receipts.

Query projections include authored properties, computed world geometry,
hierarchy, selection, available operations, validation diagnostics, and changes
since a known revision. Pagination and byte limits apply before serialization.

### Semantic operations

One operation envelope is shared by in-process UI actions, CLI, AppleScript, and
MCP adapters:

```text
request_id
operation
arguments
target locator(s)
document/workspace revision precondition(s)
mode: commit | preview | validate
```

`validate` performs the same parsing, target resolution, applicability, and
resource checks without mutating state. `preview` is limited to operations with
an explicit cancellable interaction contract. `commit` is the normal path.
Request IDs provide session-bounded deduplication so retrying after a transport
failure cannot accidentally apply the same mutation twice. The descriptor
advertises the bounded deduplication window; an expired request ID fails
explicitly instead of being treated as a new mutation.

### Verification recipes and evidence bundles

A verification recipe names observable postconditions rather than implementation
steps. Recipes compose checks from these evidence classes:

- core semantic state and history behavior;
- native save/load round trip;
- display-list structure;
- SVG geometry and style;
- raster geometry, alpha, and color comparison;
- interactive GPUI state or screenshot;
- platform-specific behavior;
- bounded rejection and resource behavior.

Running a recipe produces a machine-readable evidence bundle containing the
binary and protocol versions, starting artifact hash, requests and receipts,
resulting artifact hashes, checks actually run, outcomes, environment facts,
and explicit unverified layers. Large artifacts are referenced by path and
digest rather than embedded. A bundle is evidence, not truth: its claims cannot
exceed its recorded checks.

## Resource model

The protocol makes cost visible before execution. Capabilities classify likely
cost as metadata-only, document traversal, layout, render, filesystem, or
platform interaction. Query budgets bound returned nodes and bytes. Render and
export requests declare pixel or artifact budgets and return the applied scale
or truncation decision. Batches have operation, target, and total-complexity
limits.

Caching keys include session ID, document/artwork revision, and query
projection. A semantic mutation invalidates only affected projections. The
change journal is bounded by count and bytes; when a requested revision has
expired, the query returns `snapshot_required` instead of an incomplete delta.
Agents should be able to ask for changes since a recent revision instead of
polling screenshots or serializing the whole tree.

## Safety and authority

Introspection, validation, in-memory editing, filesystem writes, and external
effects are distinct authority classes. The manifest marks each operation's
class. File operations require normalized absolute paths and report overwrite
behavior before execution. Destructive document operations support validation
and atomic batches, remain undoable where the editor contract allows it, and
identify deleted entities in receipts.

The control plane never fetches external SVG resources, bypasses import limits,
or gains a more permissive data path than the UI. Automation is a caller of the
product contract, not a privileged escape hatch.

## Vertical delivery plan

Each phase must ship through the real core operation, automation transport,
documentation, and applicable tests. Later phases build only on evidence from
earlier ones.

### Phase A: revisions and receipts

Add session-local revisions, stable error codes, request IDs, optimistic
preconditions, and mutation receipts to existing state-changing operations.
Prove stale writes do not mutate, retries do not duplicate mutations, UI and
automation share history semantics, and existing clients receive a deliberate
protocol-version response rather than silently misreading fields. Bound and
test the request-deduplication cache and revision-change journal, including
expiry and document replacement.

### Phase B: capability and query discovery

Generate the manifest from one registry, expose compact session discovery,
finish revision-delta queries, and generate CLI/MCP reference material from the
same metadata. The bounded entity projection is implemented; generated result
schemas and change cursors remain. Prove disabled reasons, units, limits, and
schemas match runtime validation.

### Phase C: semantic coverage and atomic batches

Close the highest-value coordinate-only gaps required by the Native Brand Kit,
then add validated atomic batches with intra-batch references. Start with exact
geometry, native appearance editing, boolean construction, and export recipes;
do not build a generic macro language ahead of those workflows.

### Phase D: executable verification

Turn the Brand Kit acceptance workflow into public-operation scenarios and add
composable verification recipes for persistence, history, SVG, raster, and GPUI
evidence. Emit evidence bundles whose claims are mechanically bounded by the
checks recorded.

### Phase E: recovery and resumability

Expose autosave/recovery state, conflict information, and resumable bounded jobs
through the same session and receipt model. Prove that a fresh agent context can
inspect a recovered session, determine the last durable and committed revisions,
and continue without replaying already accepted requests.

## Acceptance conditions

The control plane is not complete because a schema or MCP tool exists. It must
demonstrate all of the following with the real Strek Native Brand Kit:

- a cold agent discovers the running binary and builds the project without
  source-code knowledge or fixture-specific IDs;
- the semantic path uses fewer state round trips than the current command flow
  and no pointer gestures for operations with semantic parameters;
- interrupting and retrying a request cannot duplicate a committed edit;
- a stale concurrent request is rejected before mutation and can be rebased
  from returned changes;
- the project remains undoable, persistable, editable, and visually equivalent
  within its documented render limits;
- another fresh context can consume the evidence bundle, reproduce the relevant
  checks, and distinguish verified from unverified claims;
- capability documentation and runtime validation are generated from the same
  definitions and fail tests when they diverge.

Until these cases pass, describe individual delivered contracts precisely; do
not call the entire editor agent-native or self-verifying.
