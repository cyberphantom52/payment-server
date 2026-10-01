# Design

<!-- Target roughly 800 to 1,500 words. Describe actual implementation choices and limitations. -->

## 1. Data Model

<!-- Add an ER diagram or table listing. For each table: shape, indexes, primary key strategy, why this shape over alternatives, and changes at 100x scale. Cover businesses, customers, invoices, line items, payment attempts, and supporting tables actually used. Explain integer USD cents and server-computed totals. -->

## 2. Invoice State Machine

<!-- Add an ASCII, Mermaid, or image diagram showing every state, valid transition, trigger, and terminal state. -->

### Reversible Transitions

### Invalid Transition Rejection

## 3. Payment Correctness & Failure Modes

### (a) Simultaneous Payments for the Same Invoice

<!-- Outcome and mechanism guaranteeing it. -->

### (b) PSP Timeout

<!-- tok_timeout takes 30 seconds: endpoint response, invoice/payment attempt states, and how the caller learns the eventual result. -->

### (c) Crash After PSP Success Before Persistence

<!-- Retry behavior, double-charge prevention, and any unresolved limitations. -->

### (d) Idempotency Key Reused With a Different Body

### (e) Payment Requested for an Already Paid Invoice

### Concurrency Mechanism and Alternatives

<!-- Name the mechanism used and explain why it was chosen over alternatives. Describe idempotency scope and persistence. -->

## 4. Webhook Design

### Events and Asynchronous Delivery

<!-- invoice.created, invoice.paid, invoice.payment_failed; why and how delivery is decoupled from API responses. -->

### Signing and Replay Protection

<!-- Algorithm, signed content, verification, and replay protection. -->

### Retry Policy

<!-- Specific intervals, maximum attempts, and total retry budget. -->

### Exhausted Retries and Missed-Event Reconciliation

## 5. API Key Model

<!-- Generation, storage (plaintext/hash/prefix), transmission, business scoping, rotation, revocation, and blast radius if leaked. -->

## 6. What You Cut and Why

<!-- List 3 to 5 deliberate omissions with reasons, including at least one thing you wanted to build. -->

1. <!-- Omission and reason. -->
2. <!-- Omission and reason. -->
3. <!-- Omission and reason. -->

## 7. Production Readiness Gap

<!-- Top 3 things missing if shipped tomorrow; explain priority. Discuss production rate limiting here. Mention refunds or partial payments if you would add them later. -->

1. <!-- Gap and priority. -->
2. <!-- Gap and priority. -->
3. <!-- Gap and priority. -->
