CREATE TABLE demo_webhook_receipts (
    event_id bigint PRIMARY KEY CHECK (event_id > 0),
    payload text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);
