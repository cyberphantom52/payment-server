# AI Usage

<!-- Fill this honestly and specifically before submission. Do not invent independent decisions or corrections. -->

## Tools Used and What They Did

<!-- For each tool: name, concrete tasks, output used or rejected, and your revisions. Include documentation-template assistance if applicable. -->

Codex helped explain Axum, PostgreSQL, transactions, migrations, and webhooks,
read the assignment, and discuss the schema and failure modes. It is maintaining
these documentation files as design and implementation progress. At my request,
GPT-6.1 Sol subagents with low reasoning implemented the database migrations and
schema checks. I then requested GPT-6.1 Sol subagents with high reasoning for the
remaining implementation. The main agent reviews, validates, and commits their
work in the agreed sequence. Features are verified in stages before their commits; later stages remain in
progress.

The first high-reasoning implementation stage added key administration,
authentication, shared JSON errors, and readiness checks. Review corrected the
CLI to preserve the agreed `admin` command prefix. Its parser and authentication
tests passed before the feature commit.

The customer/invoice stage used the existing error and authentication helpers.
Review checked tenant-qualified queries and transaction boundaries; integration
tests verified checked cents arithmetic and rollback when event persistence fails.

The schema review removed unnecessary copies of webhook destination URLs and
secrets from event rows: the agreed fixed endpoint can be referenced directly.
Validation applied both migration sets to isolated PostgreSQL test databases and
passed all three schema tests, covering money/state constraints, tenant
boundaries, payment uniqueness, webhook payload preservation, and provider data.
The schema tests validate database constraints; separate HTTP tests exercise
the application workflow.

The provider stage added persisted operation identity and readiness before HTTP
delays. Review corrected the decline response field to the assignment’s `code`
contract and rejected duplicate idempotency headers. Four provider tests passed,
including concurrent submission, mismatches, cancellation/restart, and stored
success behind a simulated HTTP 500.

The payment stage added asynchronous acceptance, exact response replay, scoped
attempt reads, and provider recovery. Main-agent review checked lock ordering,
transaction boundaries, strict provider response decoding, and rejected redirects
as settlement evidence. The integration suite includes 20 concurrent clients,
a real 30-second timeout, and rollback of settlement when event persistence fails.

The webhook stage added HMAC signing, exact-byte delivery, durable retry budgets,
and a separate demo receipt table. Review required a real in-flight cancellation
check rather than only setting the delivery counter in SQL. Tests verify the
fifth dispatch is reserved before HTTP and restart recovery cannot send a sixth.
The signature test also checks a value independently computed with Python's HMAC.

A final Clippy pass requested two style-only changes: collapsing the invoice state-filter guard and using a boolean
assertion directly. The implementation agent applied these without changing the
API or persistence behavior; formatting and strict Clippy checks then passed.

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

Codex proposed hand-picked retry intervals of 5, 30, 120, and 600 seconds. I asked
for a standard backoff policy instead. We chose exponential waits of 5, 10, 20,
and 40 seconds. I preferred a recognized policy to a hand-picked sequence.

Separately, I challenged the network-error mock recommendation and asked Codex to
reason from client expectations and idempotency. We chose stored provider success
with an HTTP 500 response, then recovery by lookup. Codex recommended durable
provider idempotency from the outset; I did not introduce it against a contrary
recommendation.

<!-- AI proposal, if any: -->

<!-- My choice: -->

<!-- Why: -->

## Something AI Got Wrong and How I Corrected It

<!-- Describe the issue, correction, and verification. If nothing needed correction, explicitly say so and explain how correctness was verified. -->
