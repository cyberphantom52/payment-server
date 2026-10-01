# Invoice & Payment Service

<!-- Fill in a brief project description. -->

## Run Instructions

<!-- Prerequisites, docker compose up, service URLs, and how to obtain a business API key. Setup must require no manual steps. -->

## Configuration

<!-- Environment variables and relevant defaults. -->

## Curl Examples

<!-- Provide 3 to 4 runnable examples, including authentication and representative responses. -->

### Create a Customer

### Create an Invoice

### Attempt a Successful Payment

### Attempt a Failed Payment

<!-- Use tok_card_declined; show Idempotency-Key on payment requests. -->

## Tests

<!-- Commands to run tests and results. Explain any skipped required test. -->

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

## Documentation

- [Design](DESIGN.md)
- [API](API.md)
- [AI Usage](AI_USAGE.md)
