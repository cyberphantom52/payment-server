# API Documentation

Status: agreed contract. Schemas are implemented; endpoint implementation and
verification will be recorded in subsequent commits.

## Conventions and authentication

Local base URL: http://localhost:3000. JSON bodies, numeric IDs, integer USD cents,
YYYY-MM-DD dates, UTC RFC 3339 timestamps. Protected routes require
Authorization: Bearer <key>. Derive business scope from that key; another business's
resources return 404.

Lists accept limit (default20, maximum100) and after_id (exclusive ascending ID).
Return {"data":[],"next_after_id":null}. Lists describe current state, not a change feed.

Errors: {"error":{"code":"invalid_input","message":"..."}}. Statuses:400 invalid
input,401 invalid/revoked key,404 missing resource,405 wrong method,409 conflict,
500 unexpected error with generic message. Readiness may return503.
Public GET /health checks database availability.

## Customers

- POST /customers: {"name":"Ada","email":"ada@example.com"}, returning201 with ID
  and creation timestamp. Require nonempty name/email; reject unknown fields.
- GET /customers/{id}:200 customer or404.
- GET /customers: paginated business-scoped list.

## Invoices

POST /invoices accepts:

```json
{"customer_id":1,"due_date":"2026-10-31","line_items":[{"description":"Notebook","quantity":2,"unit_amount_cents":1000}]}
```

Return201 with invoice, computed total_cents, and ordered line items. Require
nonempty items/descriptions, positive integer quantities, nonnegative integer
prices, and positive total within i64. Reject client totals, unknown fields,
and arithmetic overflow. Past due dates are allowed.

GET /invoices/{id} returns the invoice with line items. GET /invoices lists
invoices, optionally filtered by state=open|paid. Financial contents are immutable;
creation starts open and only confirmed success moves to terminal paid.

## Payments

POST /invoices/{id}/pay accepts {"card_token":"tok_success"} and a nonempty
Idempotency-Key header. Accepted requests return202:

```json
{"payment_attempt_id":42,"invoice_id":1,"status":"pending"}
```

Location points to /payment-attempts/42. Matching business/key/invoice/token retries
replay original status/body bytes even after settlement. Changed input returns409.
New attempts conflict with a pending attempt or paid invoice. Confirmed failure
permits a new key/attempt. Retain idempotency records.

GET /payment-attempts/{id} returns current pending|succeeded|failed state, invoice,
timestamps, provider reference or failure code. GET /invoices/{id}/payment-attempts
lists attempt history. Never expose stored card tokens.

Timeouts or HTTP errors leave attempts pending while recovery checks actual
provider state. Repeating acceptance does not create another payment; query the
attempt or receive webhooks for completion. Unsupported tokens return400.

## Webhooks

POST /webhook-endpoints accepts {"url":"https://approved.example/webhook"},
returning201 with endpoint identity and signing secret. A second registration
returns409. URLs must match the operator allowlist and use HTTPS except for the
explicit internal demo URL. No URL/secret update endpoint.

Event types: invoice.created, invoice.paid, invoice.payment_failed. Payload contains
stable event ID, creation timestamp, and invoice/payment information; exact fields
will be documented against the implementation.

Webhook-Timestamp contains Unix seconds. Webhook-Signature contains v1=<hex>.
Sign timestamp ASCII + "." + exact body bytes using HMAC-SHA256. Verify before
JSON decoding, allow ±300 seconds, durably deduplicate event ID together with
accepted work, and acknowledge with2xx. Retry ID/body remains unchanged while
timestamp/signature refresh. Ordering is not guaranteed.

Five total attempts: immediate then exponential delays5/10/20/40 seconds after
failure. Timeout5 seconds; absolute lifetime15 minutes from event creation.
Non-2xx and transport failures retry; redirects disabled. Exhausted events stay
stored. Reconcile through invoice and attempt APIs. No event-history/manual-replay
API; endpoint registration does not deliver earlier events.

## Mock PSP

Internal POST /payments requires Idempotency-Key:<application attempt ID> and
{"invoice_id":1,"amount_cents":2000,"card_token":"tok_success"}.
GET /payments/{attempt_id} returns pending/final result or404 if unknown.
Repeated submissions must match invoice, amount, and token.

| Token | Submission behavior |
| --- | --- |
| tok_success | Success after about100ms, stable UUID psp_ref. |
| tok_insufficient_funds | Failure after about100ms, code insufficient_funds. |
| tok_card_declined | Failure after about100ms, code card_declined. |
| tok_timeout | Success after30 seconds; lookup exposes progress/result. |
| tok_network_error | Persist success but return500; lookup reveals success. |

Unknown tokens return400. Persistence precedes delayed responses and survives
client cancellation/provider restart. Public GET /health checks provider DB.
