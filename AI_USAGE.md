# AI Usage

<!-- Fill this honestly and specifically before submission. Do not invent independent decisions or corrections. -->

## Tools Used and What They Did

<!-- For each tool: name, concrete tasks, output used or rejected, and your revisions. Include documentation-template assistance if applicable. -->

Codex helped explain Axum, PostgreSQL, transactions, migrations, and webhooks,
read the assignment, and discuss the schema and failure modes. It is maintaining
these documentation files as design and implementation progress. At my request,
GPT-6.1 Sol subagents with low reasoning implement the database migrations and
schema checks; the main agent reviews and validates their work. API handlers,
workers, and the provider protocol are not implemented at this stage.

The schema review removed unnecessary copies of webhook destination URLs and
secrets from event rows: the agreed fixed endpoint can be referenced directly.
Validation applied both migration sets to isolated PostgreSQL test databases and
passed all three schema tests, covering money/state constraints, tenant
boundaries, payment uniqueness, webhook payload preservation, and provider data.
These tests do not yet verify the unimplemented HTTP payment workflow.

## Three Decisions I Made Independently or Against AI Suggestions

### Decision 1

After reviewing the assignment, Codex recommended limiting business setup to
bootstrapping and documented database administration. I chose to retain a small
administration CLI because I wanted API-key generation, rotation, and revocation
demonstrated by working behavior, rather than only described in the design.

<!-- AI proposal, if any: -->

<!-- My choice: -->

<!-- Why: -->

### Decision 2

I selected an always-asynchronous payment API after discussing it with Codex.
My reason was that a hybrid synchronous/asynchronous contract creates two result
paths for both the service and its caller. Accepted requests will return `202`;
status lookup and webhooks will expose the eventual result. This was a discussed
choice, not a rejection of an AI recommendation.

<!-- AI proposal, if any: -->

<!-- My choice: -->

<!-- Why: -->

### Decision 3

Pending: record another concrete decision and my reasoning as implementation
continues. Do not count simple acceptance of a suggestion as an independent
decision or invent a disagreement. Codex recommended durable provider idempotency
from the outset; we retained it after discussing its scope.

<!-- AI proposal, if any: -->

<!-- My choice: -->

<!-- Why: -->

## Something AI Got Wrong and How I Corrected It

<!-- Describe the issue, correction, and verification. If nothing needed correction, explicitly say so and explain how correctness was verified. -->
