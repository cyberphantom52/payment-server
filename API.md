# API Documentation

## Base URL and Conventions

<!-- Base URL, content type, IDs, timestamps, USD integer cents, list response conventions, and invoice/payment attempt state values. -->

## Authentication

<!-- API key header format, business scope, and authentication/authorization errors. -->

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

### Get Invoice

<!-- Method/path, parameters, response shape, status codes, business scope. -->

### List Invoices

<!-- Method/path, state filter and other supported query parameters, response shape, status codes, business scope. -->

### Invoice State Transition Operations

<!-- Document any implemented transition endpoints: method/path, request/response shapes, valid transitions, and invalid-transition errors. -->

## Payments

### POST /invoices/{id}/pay

<!-- Path parameter, mock card token request shape, required Idempotency-Key header, response shapes and statuses for success/failure/pending, and business scope. -->

### Idempotency and Concurrent Requests

<!-- Key scope, retention, replay behavior, different-body conflicts, concurrent payments, already paid invoices. -->

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
