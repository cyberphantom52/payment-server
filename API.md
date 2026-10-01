# API Reference

## Conventions and authentication

Local base URL: `http://127.0.0.1:3000`. Bodies are JSON. IDs are integers, money
is integer USD cents, dates use `YYYY-MM-DD`, and timestamps use UTC RFC 3339.
Protected routes require `Authorization: Bearer <key>`. Resources belonging to
another business return `404`. Unknown JSON fields are rejected.

Lists accept `limit` (default 20, range 1–100) and `after_id` (nonnegative,
exclusive). Results are ordered by ascending ID. `next_after_id: null` marks the
last page; an empty result is `{"data":[],"next_after_id":null}`.
Invoice reads and payment-attempt history provide current state for reconciliation
after missed or out-of-order webhooks.

Errors use one envelope:

```json
{"error":{"code":"invalid_input","message":"Expected a valid JSON request body"}}
```

Public `GET /health` returns `200 {"status":"ok"}` when the database is reachable,
otherwise `503` with the error envelope.

## Customers

`POST /customers` accepts `{"name":"Ada","email":"ada@example.com"}` and returns
`201`. Name and email must contain non-whitespace text; surrounding whitespace is
trimmed. Email delivery/verification is outside this service.

```json
{"id":1,"business_id":1,"name":"Ada","email":"ada@example.com","created_at":"2026-10-01T10:00:00Z"}
```

`GET /customers/{id}` returns the same shape with `200`. `GET /customers` returns
`{"data":[<customer>],"next_after_id":null}`. Customers have no state filter.

## Invoices

`POST /invoices` accepts:

```json
{"customer_id":1,"due_date":"2026-10-31","line_items":[{"description":"Notebook","quantity":2,"unit_amount_cents":1000}]}
```

It returns `201`:

```json
{"id":1,"business_id":1,"customer_id":1,"due_date":"2026-10-31","total_cents":2000,"state":"open","created_at":"2026-10-01T10:00:00Z","paid_at":null,"line_items":[{"description":"Notebook","quantity":2,"unit_amount_cents":1000}]}
```

Validation requires nonempty items/descriptions, positive integer quantities,
nonnegative integer prices, and a positive total within signed 64-bit range.
Client-supplied totals and arithmetic overflow return `400`. Past due dates are
allowed. The customer must belong to the authenticated business.

`GET /invoices/{id}` returns the same shape with `200`.
`GET /invoices?state=open&limit=20` returns a page of invoice summaries: the same
fields **without** `line_items`. The optional state filter accepts `open` or
`paid`. Financial contents are immutable. Only confirmed payment success changes
an open invoice to terminal `paid`, setting `paid_at`.

## Payments

`POST /invoices/{id}/pay` accepts `{"card_token":"tok_success"}` and exactly one
nonempty `Idempotency-Key` header, at most 255 ASCII characters. Accepted requests
return `202`, `Content-Type: application/json`, and `Location: /payment-attempts/42`:

```json
{"payment_attempt_id":42,"invoice_id":1,"status":"pending"}
```

Matching business/key/invoice/token retries replay the originally stored body
bytes, status, and location even after settlement. Changed input returns `409`.
New attempts conflict with a pending attempt or paid invoice. A confirmed decline
permits a new key/attempt. Keys and attempt history are retained indefinitely in
this version. Unsupported tokens return `400`.

`GET /payment-attempts/{id}` returns `200`:

```json
{"id":42,"business_id":1,"invoice_id":1,"status":"succeeded","psp_ref":"fe270971-8091-4ccf-9a72-9a90987a056e","failure_code":null,"created_at":"2026-10-01T10:00:00Z","updated_at":"2026-10-01T10:00:01Z"}
```

`status` is `pending`, `succeeded`, or `failed`. Pending attempts have both result
fields `null`; successful attempts have a UUID `psp_ref`; failures have
`failure_code: "card_declined"` or `"insufficient_funds"` and a null reference.
`GET /invoices/{id}/payment-attempts` returns a page of these objects, without a
state filter. Neither endpoint exposes the stored mock card token.

Timeouts, HTTP errors, and malformed provider responses leave attempts pending
while the worker checks provider state again. The invoice remains open, with new
attempts blocked until resolution. Completion is available through attempt reads
and webhooks; repeating acceptance does not trigger another provider request.

## Webhooks

`POST /webhook-endpoints` accepts `{"url":"https://approved.example/webhook"}` and
returns `201`:

```json
{"id":1,"url":"https://approved.example/webhook","signing_secret":"<64 hex characters>"}
```

The signing secret is returned only at registration; there is no retrieval or
update endpoint. A second registration for the same business returns `409`.
URLs must exactly match a canonicalized operator allowlist and require HTTPS,
except for the explicitly configured internal demo URL. Credentials and URL
fragments are rejected. Registration affects future events only. The demo
business already has its endpoint configured.

The stable event envelope is:

```json
{"id":7,"type":"invoice.paid","created_at":"2026-10-01T10:00:01Z","data":{"business_id":1,"invoice_id":1,"payment_attempt_id":42,"state":"paid","total_cents":2000,"status":"succeeded","psp_ref":"fe270971-8091-4ccf-9a72-9a90987a056e","failure_code":null}}
```

| Event | Data |
| --- | --- |
| `invoice.created` | `invoice_id`, `business_id`, `customer_id`, `state: "open"`, `total_cents`. |
| `invoice.paid` | Fields shown above; `state: "paid"`, `status: "succeeded"`. |
| `invoice.payment_failed` | Same payment fields; `state: "open"`, `status: "failed"`, null `psp_ref`, populated `failure_code`. |

`Webhook-Timestamp` contains decimal Unix seconds. `Webhook-Signature` contains
`v1=<hex>`. Decode the secret's hex to 32 bytes, then compute HMAC-SHA256 over
`timestamp + "." + exact_body_bytes`. Verify using a constant-time MAC check
before JSON decoding, allow at most ±300 seconds of clock difference, and durably
deduplicate the signed event ID with accepted work before acknowledging `2xx`.

Retries preserve event ID/body and generate fresh signature timestamps. Delivery
order is unspecified. Five attempts total: immediate, then exponential delays of
5, 10, 20, and 40 seconds after failures. Each request times out after five seconds,
bounded by an absolute 15-minute lifetime from event creation. Non-2xx and
transport errors retry; redirects are disabled. Crash recovery consumes the
already reserved attempt, so a sixth dispatch is never allowed.

Exhausted events remain stored. Reconcile using invoice and attempt reads; there
is no event-history or manual-redelivery endpoint. The internal provider route
`POST /demo/webhooks` verifies signatures and durably deduplicates event IDs,
returning `204` for new or identical duplicate receipts and `409` for a signed,
changed body reusing an ID. Invalid/stale signatures return `400`.

## Mock PSP

This internal service listens on port 3001. `POST /payments` requires
`Idempotency-Key: <application attempt ID>` and accepts:

```json
{"invoice_id":1,"amount_cents":2000,"card_token":"tok_success"}
```

`GET /payments/{attempt_id}` returns `200` with a pending or final result, or `404`
when absent. Repeated submissions must match invoice, amount, and token; changed
input returns `409`. Each operation has one stable UUID reference.

| Token | Submission behavior |
| --- | --- |
| `tok_success` | Success after about 100 ms. |
| `tok_insufficient_funds` | Failure after about 100 ms, code `insufficient_funds`. |
| `tok_card_declined` | Failure after about 100 ms, code `card_declined`. |
| `tok_timeout` | Success after 30 seconds; lookup exposes progress/result. |
| `tok_network_error` | Persist success but return `500`; lookup reveals success. |

Example result shapes:

```json
{"attempt_id":42,"status":"pending"}
{"attempt_id":42,"status":"succeeded","psp_ref":"fe270971-8091-4ccf-9a72-9a90987a056e"}
{"attempt_id":43,"status":"failed","code":"card_declined"}
```

Unknown tokens return `400`. Persistence precedes delayed responses and survives
client cancellation/provider restart. Public `GET /health` checks the provider DB.
