# Invoice & Payment Service

<!-- Fill in a brief project description. -->

A Rust invoice and payment service using Axum and PostgreSQL, with a separate
mock payment provider. This repository is under construction: the current step
implements the database model; the API, payment processing, webhook delivery,
administration CLI, and Compose setup still need implementation.

## Run Instructions

<!-- Prerequisites, docker compose up, service URLs, and how to obtain a business API key. Setup must require no manual steps. -->

## Configuration

<!-- Environment variables and relevant defaults. -->

The current application reads `DATABASE_URL` and applies `migrations/` through
SQLx on startup. The initial migration creates the assignment tables. Use a fresh
assignment database if an older database already applied the removed learning
migration. Provider migrations live in `provider_migrations/` and require a
separate database; provider startup integration is still pending.

## Curl Examples

<!-- Provide 3 to 4 runnable examples, including authentication and representative responses. -->

### Create a Customer

### Create an Invoice

### Attempt a Successful Payment

### Attempt a Failed Payment

<!-- Use tok_card_declined; show Idempotency-Key on payment requests. -->

## Tests

<!-- Commands to run tests and results. Explain any skipped required test. -->

Schema checks use a PostgreSQL role with `CREATEDB` access. SQLx creates isolated
test databases and applies the appropriate application or provider migration:

```sh
DATABASE_URL=postgresql://localhost/postgres cargo test --test schema
```

Set the URL to your test PostgreSQL instance and role. These checks cover cents
and state constraints, tenant foreign keys, payment uniqueness, webhook
destination constraints, and provider records. The required HTTP concurrency,
response replay, and timeout/recovery tests below remain pending until the
corresponding behavior is implemented.

Verified: all three schema tests passed against a disposable local PostgreSQL
instance. `cargo check --tests` and `cargo fmt --check` also passed. The current
application has an unused `AppState.db` warning until handlers use the pool.

### Concurrent Payments

<!-- N concurrent requests: at most one succeeds, no double charges, consistent final state. -->

### Idempotency

<!-- Same request and key: same response, no second PSP call. -->

### PSP Failure

<!-- tok_timeout or tok_network_error: invoice does not remain in a bad state. -->

## Demo Video

<!-- Add a publicly accessible, no-login link to a 5 to 10 minute recording. Cover in order: architecture (1 to 2 min), live demo (2 to 3 min), unscripted state machine walkthrough (1 to 2 min), unscripted failure-mode code walkthrough (1 to 2 min). Show docker compose up, customer/invoice creation, successful and declined payments, and webhook deliveries. -->

## Remaining Work and Known Limitations

<!-- Document unfinished work if the 4 to 6 hour budget was exceeded. If using a language other than Rust, briefly justify it here. -->

The database constraints do not yet implement the complete payment workflow.
Remaining work includes the API contract and handlers, API-key administration,
provider HTTP behavior and recovery, durable workers, webhook signing and
delivery, Compose bootstrapping, the three required behavioral tests, runnable
curl examples, and the demo recording. Documentation distinguishes agreed plans
from verified implementation as work progresses.

## Documentation

- [Design](DESIGN.md)
- [API](API.md)
- [AI Usage](AI_USAGE.md)
