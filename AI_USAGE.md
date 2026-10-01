# AI Usage

## Tools Used and What They Did

I used Codex to learn Axum, SQLx/PostgreSQL, and webhook signing, then to discuss
the data model, payment state machine, and failure handling. It also drafted the
documentation from those discussions.

Codex and GPT-6.1 Sol subagents implemented the schemas and migrations,
authentication and administration, customer/invoice APIs, payment provider and recovery worker,
webhook delivery, integration tests, and Compose setup. AI generated most of the
implementation code. My contributions were scope, design decisions, and the
challenges to recommendations described below.

Codex reviewed the generated changes and ran compilation, formatting, Clippy,
integration tests, and container acceptance checks. [README.md](README.md)
summarizes the verification evidence and environment.

## Three Decisions I Made Independently or Against AI Suggestions

### Decision 1: An always-asynchronous payment API

I selected an always-asynchronous payment API after discussing it with Codex.
My reason was that a hybrid synchronous/asynchronous contract creates two result
paths for both the service and its caller. Every durably accepted request returns
`202`; a background worker handles the provider interaction, and status lookup
and webhooks expose the eventual result. The tradeoff is that even fast payments
require the caller to observe completion separately. I preferred that consistent
contract across normal responses, slow providers, and recovery. This choice was
made through discussion with Codex.

### Decision 2: Defer horizontal scaling and use one application instance

During planning, I pointed out that the proposed worker design would prevent us
from safely scaling the application across multiple instances. I wanted
horizontal scaling, but accepted one application instance with separate sequential
payment and webhook workers to keep the assignment manageable and focus on
payment correctness. Concurrent HTTP requests are still supported; background
throughput and application availability are limited by that single instance.
I asked that we explicitly document this tradeoff and the changes needed at
100x scale: durable work claims with expiring leases, bounded worker concurrency,
and then multiple instances.

### Decision 3: Resolve uncertain provider outcomes before allowing another payment

I challenged the network-error mock recommendation and asked Codex to reason
from client expectations and idempotency. We chose to model a payment succeeding
at the provider while its submission response returns HTTP `500`. The application
keeps the attempt pending and resolves it through provider lookup using the same
durable attempt identity. An unresolved attempt blocks a fresh payment; only a
confirmed decline permits another attempt.

My concern was that a communication failure does not establish whether money
moved. Allowing a new payment immediately could charge the customer twice. The
tradeoff is that an invoice can remain blocked while the provider is unavailable.
Codex had already recommended durable provider idempotency; my contribution was
challenging the failure semantics and choosing this recovery behavior with Codex.

## Something AI Got Wrong and How I Corrected It

The webhook retry recommendation is a concrete example. Codex proposed the custom
sequence 5, 30, 120, and 600 seconds. I pushed back and asked for a standard exponential
backoff policy.

The original sequence could work, but I preferred intervals derived from a clear
rule. Integration tests inspect the persisted retry schedule and enforce the
five-attempt limit, including recovery after an interrupted fifth dispatch.
Codex ran these tests successfully.
