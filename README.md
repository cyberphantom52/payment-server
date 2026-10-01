# Invoice & Payment Service

<!-- Fill in a brief project description. -->

A Rust invoice and payment service using Axum and PostgreSQL, with a separate
mock payment provider. The database model, API-key administration/authentication,
and health endpoint are implemented. Customer/invoice APIs and transactional
invoice-created events, the durable mock provider, and payment admission/recovery
and signed webhook delivery are implemented. Compose setup is the next stage.

## Run Instructions

Implementation proceeds in auditable commits: contracts, API-key administration,
customer/invoice APIs, durable provider, payment recovery, webhooks, Compose/demo
bootstrap, and verified submission docs. Each feature commit includes relevant
tests and documentation updates. No commits are pushed automatically.

The target command is `docker compose up`; Compose files are not implemented at
this checkpoint. Validation currently uses disposable PostgreSQL. Podman is
available on the development host with podman-compose 1.6.0; container validation
will use Podman.

For the implemented native development foundation, use a fresh application
database and provide its URL (the database must already exist):

```sh
export DATABASE_URL=postgresql://localhost/payment_app
cargo run -- admin create-business "Example Business"
cargo run -- admin create-key 1
cargo run -- admin list-keys 1
cargo run -- admin revoke-key 2
cargo run
```

Creation commands reveal the new key once; listing returns metadata only. Rotate
by creating a replacement, updating the client, and revoking the old key. IDs in
these examples must be replaced with the returned IDs. `GET /health` returns
`200 {"status":"ok"}` while the database is reachable, otherwise JSON `503`.
`BIND_ADDR` defaults to `127.0.0.1:3000` for native development.

<!-- Prerequisites, docker compose up, service URLs, and how to obtain a business API key. Setup must require no manual steps. -->

## Configuration

<!-- Environment variables and relevant defaults. -->

The current application reads `DATABASE_URL` and applies `migrations/` through
SQLx on startup. The initial migration creates the assignment tables. Use a fresh
assignment database if an older database already applied the removed learning
migration. Provider migrations live in `provider_migrations/` and require a
separate database. Run it with `DATABASE_URL=postgresql://localhost/payment_provider
cargo run --bin payment-provider` (on one line). It applies provider migrations
and listens on `127.0.0.1:3001` by default.

## Curl Examples

<!-- Provide 3 to 4 runnable examples, including authentication and representative responses. -->

### Create a Customer

```sh
curl -sS http://localhost:3000/customers \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  -d '{"name":"Ada","email":"ada@example.com"}'
```

Set `API_KEY` to a key returned by the admin CLI. Creation returns HTTP `201`.

### Create an Invoice

```sh
curl -sS http://localhost:3000/invoices \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  -d '{"customer_id":1,"due_date":"2026-10-31","line_items":[{"description":"Notebook","quantity":2,"unit_amount_cents":1000}]}'
```

Use the returned customer ID. The server returns `201`, an `open` invoice, and
`total_cents: 2000`. Creation/detail responses include ordered line items;
paginated invoice lists return summaries without line items.

### Attempt a Successful Payment

```sh
curl -i http://localhost:3000/invoices/1/pay \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: example-success' -d '{"card_token":"tok_success"}'
```

Returns `202` with `payment_attempt_id`, `invoice_id`, and `status: "pending"`.
Use the returned `Location` to read the eventual outcome. `PSP_URL` defaults to
`http://127.0.0.1:3001`; run the provider against its own database first.

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
destination constraints, and provider records. The payment integration suite
exercises HTTP concurrency, exact acceptance replay, and recovery through the
separately migrated provider database.

Verified: all three schema tests, two authentication tests, and the admin CLI
parser test passed against disposable PostgreSQL where needed. Authentication
checks include overlapping key rotation, revocation, business identity, malformed
credentials, JSON errors, and transactional business/key creation. Formatting
and compilation checks passed. Run the foundation checks with:

```sh
DATABASE_URL=postgresql://localhost/postgres cargo test --test auth --bin payment-server
```

The resource integration test also passed. It exercises business isolation,
pagination/state filtering, integer overflow rejection, line ordering, endpoint
capture, and rollback of invoice/items/event together after a forced failure:

```sh
DATABASE_URL=postgresql://localhost/postgres cargo test --test resources
```

Four provider HTTP tests passed: concurrent duplicates share one durable operation,
changed inputs conflict, all mock tokens behave as specified, and cancelled
requests recover through lookup after a server restart. The focused restart test
advances the stored timeout deadline; the payment suite separately runs the real
30-second provider delay.

```sh
DATABASE_URL=postgresql://localhost/postgres cargo test --test provider
```

Five payment integration tests passed, including the real 30-second scenario.

### Webhook Delivery

Nine focused tests passed: HMAC verification against an independent vector,
timestamp/signature rejection, durable duplicate receipt handling, endpoint URL
policy, exponential retry scheduling, payload stability, remaining-lifetime
timeouts, disabled redirects, and cancellation of a reserved fifth dispatch.
Recovery exhausts that event without sending a sixth request.

```sh
DATABASE_URL=postgresql://localhost/postgres cargo test --test webhooks
```

Set `WEBHOOK_ALLOWED_URLS` to comma-separated exact destination URLs. HTTPS is
required except for an explicitly configured `DEMO_WEBHOOK_URL`, which must also
be allowlisted. Default: no destinations allowed. The provider's demo receiver
is `/demo/webhooks`; set its `DEMO_WEBHOOK_SECRET` to the endpoint's 64-character
hex secret. Verified new receipts are logged and stored in the provider database.

### Concurrent Payments

`tests/payments.rs` sends 20 simultaneous requests with distinct keys for one
invoice: one is admitted, one provider operation exists, and the invoice becomes
paid. Twenty same-key requests share an identical acceptance and attempt.

### Idempotency

The suite compares status, raw response bytes, and Location after settlement,
using a provider request counter to verify replays make no additional HTTP call.
Changed input conflicts; a confirmed decline permits a fresh key.

### PSP Failure

The real 30-second timeout scenario verifies prompt acceptance, a two-second
worker HTTP timeout, pending-state exclusion, and eventual success. Other cases
cover simulated network loss, provider restart with a committed success, malformed
results, and transactional settlement rollback when event persistence fails.

```sh
DATABASE_URL=postgresql://localhost/postgres cargo test --test payments
```

## Demo Video

<!-- Add a publicly accessible, no-login link to a 5 to 10 minute recording. Cover in order: architecture (1 to 2 min), live demo (2 to 3 min), unscripted state machine walkthrough (1 to 2 min), unscripted failure-mode code walkthrough (1 to 2 min). Show docker compose up, customer/invoice creation, successful and declined payments, and webhook deliveries. -->

## Remaining Work and Known Limitations

The agreed deployment is one app instance with separate sequential payment and
webhook workers. Multiple instances require durable job claims; higher throughput
requires bounded worker concurrency. Webhook retries use exponential waits of
5, 10, 20, and 40 seconds after the initial attempt, with five attempts and a
15-minute lifetime. State APIs provide missed-notification reconciliation.

<!-- Document unfinished work if the 4 to 6 hour budget was exceeded. If using a language other than Rust, briefly justify it here. -->

Remaining work includes Compose bootstrapping, the full
acceptance run, complete curl examples, and the demo recording. Documentation
distinguishes agreed plans from verified implementation as work progresses.

## Documentation

- [Design](DESIGN.md)
- [API](API.md)
- [AI Usage](AI_USAGE.md)
