# API Documentation

Status: this is a design draft. Assignment endpoints are not implemented yet;
the current implementation work covers database migrations and schema checks.

## Base URL and Conventions

<!-- Base URL, content type, IDs, timestamps, USD integer cents, list response conventions, and invoice/payment attempt state values. -->

- Monetary fields are integer USD cents: `unit_amount_cents` per item and
  server-computed `total_cents` per invoice. No floating-point money.
- Invoice states: `open`, `paid`. Payment attempt states: `pending`, `succeeded`,
  `failed`.
- Database IDs use `bigint`; public response shapes, pagination, and timestamp
  formatting will be finalized with the API contract.

## Authentication

<!-- API key header format, business scope, and authentication/authorization errors. -->

Planned authentication uses `Authorization: Bearer <key>`. The authenticated key
determines the business; clients cannot select another business by supplying its
ID. Revoked keys will fail authentication.

## Consistent Error Format

<!-- Define the JSON error shape and example; document HTTP statuses and error codes. -->

## Customers

### Create Customer

<!-- Method/path, request shape, response shape, status codes, validation, business scope. -->

### Get Customer

<!-- Method/path, parameters, response shape, status codes, business scope. -->

### List Customers

<!-- Method/path, query parameters, response shape, status codes, business scope. -->

## Invoices

### Create Invoice

<!-- Method/path, customer ID, due date, line items {description, quantity, unit_amount_cents}, server-computed total, response shape, statuses, validation. -->

Creation will accept a customer, due date, and line items containing
`description`, `quantity`, and `unit_amount_cents`. The server calculates the
total and creates an `open` invoice. Invoice financial contents will be immutable.

### Get Invoice

<!-- Method/path, parameters, response shape, status codes, business scope. -->

### List Invoices

<!-- Method/path, state filter and other supported query parameters, response shape, status codes, business scope. -->

### Invoice State Transition Operations

<!-- Document any implemented transition endpoints: method/path, request/response shapes, valid transitions, and invalid-transition errors. -->

## Payments

### POST /invoices/{id}/pay

<!-- Path parameter, mock card token request shape, required Idempotency-Key header, response shapes and statuses for success/failure/pending, and business scope. -->

Agreed behavior: after durably accepting a payment attempt, return HTTP `202` and
process it asynchronously. `Idempotency-Key` is required. Validation,
authentication, and conflicting requests can still fail before acceptance. The
exact response body and status lookup endpoint remain to be specified.

### Idempotency and Concurrent Requests

<!-- Key scope, retention, replay behavior, different-body conflicts, concurrent payments, already paid invoices. -->

Keys are scoped to a business. A matching retry must replay the original accepted
response even if the attempt has since completed; a reused key with a different
invoice or token is a conflict. Check for an existing matching request before
rejecting payment of an already-paid invoice. The schema stores request identity
and the original response, and prevents multiple pending/successful attempts for
one invoice. Handler and worker behavior remain to be implemented and tested.

### Timeouts, Network Errors, and Eventual Results

<!-- Returned status/body, resulting states, and how callers obtain eventual results. Document any supporting endpoints with request/response shapes. -->

## Webhooks

### Register Endpoint

<!-- Method/path, URL and other request fields, response shape, statuses, secret provisioning, business scope. -->

### Event Payloads

<!-- Shapes and examples for invoice.created, invoice.paid, and invoice.payment_failed. -->

### Signature Verification and Replay Protection

<!-- Headers, algorithm, exact signed bytes, receiver verification, and replay checks. -->

### Delivery and Retries

<!-- Asynchronous delivery, acknowledgment expectations, backoff intervals, attempt limit, exhausted retries, and reconciliation. -->

## Mock PSP

<!-- Base URL, method/path, request/response shapes, and implemented behavior for tok_success, tok_insufficient_funds, tok_card_declined, tok_timeout, tok_network_error, and unknown tokens. -->
