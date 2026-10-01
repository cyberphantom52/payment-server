# AI Usage

## Tools Used and What They Did

Codex helped explain Axum, PostgreSQL, transactions, migrations, and webhooks,
read the assignment, and discuss the schema and failure modes. It also drafted
and updated these documentation files during design and implementation. At my request,
GPT-6.1 Sol subagents with low reasoning implemented the database migrations and
schema checks. I then requested GPT-6.1 Sol subagents with high reasoning for the
remaining implementation. The main agent reviewed, validated, and committed their
work in the agreed sequence. Implementation stages are complete; final author
review and the recording remain.

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

A final Clippy pass requested two style-only changes: collapsing the invoice
state-filter guard and using a boolean assertion directly. The implementation
agent applied these without changing the API or persistence behavior; formatting and strict Clippy checks then passed.

The final implementation stage added the Compose image/services, separate database
roles, health checks, and transactional demo bootstrap. I installed podman-compose
through Homebrew. The main agent validated release builds and the fresh three-service
stack with Podman Engine 5.7.1, client 6.1.3, and podman-compose 1.6.0. It executed
the README examples and verified payment recovery, durable webhook receipts, and
revocation across full stack recreation. Docker Engine itself was unavailable.

## Three Decisions I Made Independently or Against AI Suggestions

### Decision 1

After reviewing the assignment, Codex recommended limiting business setup to
bootstrapping and documented database administration. I chose to retain a small
administration CLI because I wanted API-key generation, rotation, and revocation
demonstrated by working behavior, rather than only described in the design.

### Decision 2

I selected an always-asynchronous payment API after discussing it with Codex.
My reason was that a hybrid synchronous/asynchronous contract creates two result
paths for both the service and its caller. Accepted requests return `202`;
status lookup and webhooks expose the eventual result. This was a discussed
choice, not a rejection of an AI recommendation.

### Decision 3

Codex proposed hand-picked retry intervals of 5, 30, 120, and 600 seconds. I asked
for a standard backoff policy instead. We chose exponential waits of 5, 10, 20,
and 40 seconds. I preferred a recognized policy to a hand-picked sequence.

Separately, I challenged the network-error mock recommendation and asked Codex to
reason from client expectations and idempotency. We chose stored provider success
with an HTTP 500 response, then recovery by lookup. Codex recommended durable
provider idempotency from the outset; I did not introduce it against a contrary
recommendation.

## Something AI Got Wrong and How I Corrected It

The webhook retry recommendation in Decision 3 is a concrete example. Codex
proposed the custom sequence 5, 30, 120, and 600 seconds. I pushed back and asked
for a standard backoff policy, such as exponential or linear backoff. We changed
the design to five total attempts with exponential waits of 5, 10, 20, and 40
seconds after failures.

This was a design recommendation I corrected, not a claim that the original
sequence could not work. I wanted a standard policy whose intervals follow a
clear rule. Verification came from integration tests inspecting the persisted
retry schedule and enforcing the five-attempt limit, including recovery after
an interrupted fifth dispatch. Codex ran those tests successfully as part of the
final verification suite.
