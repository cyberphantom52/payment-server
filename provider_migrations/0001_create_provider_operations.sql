-- Run only against the provider database; the application uses the provider over HTTP.
CREATE TABLE provider_operations (
    attempt_id bigint PRIMARY KEY CHECK (attempt_id > 0),
    invoice_id bigint NOT NULL CHECK (invoice_id > 0),
    amount_cents bigint NOT NULL CHECK (amount_cents > 0),
    mock_card_token text NOT NULL CHECK (length(trim(mock_card_token)) > 0),
    psp_ref uuid NOT NULL UNIQUE,
    -- The provider selects and persists its outcome before any delayed response.
    -- Keep outcome vocabulary open until network-error recovery semantics are agreed.
    planned_outcome text NOT NULL CHECK (length(trim(planned_outcome)) > 0),
    ready_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (ready_at >= created_at)
);
