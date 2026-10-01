CREATE TABLE businesses (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name text NOT NULL CHECK (length(trim(name)) > 0)
);

CREATE TABLE api_keys (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    business_id bigint NOT NULL REFERENCES businesses(id),
    key_hash text NOT NULL UNIQUE CHECK (length(key_hash) > 0),
    display_prefix text NOT NULL CHECK (length(display_prefix) > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz,
    CHECK (revoked_at IS NULL OR revoked_at >= created_at)
);

CREATE TABLE customers (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    business_id bigint NOT NULL REFERENCES businesses(id),
    name text NOT NULL CHECK (length(trim(name)) > 0),
    email text NOT NULL CHECK (length(trim(email)) > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (business_id, id)
);

CREATE TABLE invoices (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    business_id bigint NOT NULL REFERENCES businesses(id),
    customer_id bigint NOT NULL,
    due_date date NOT NULL,
    total_cents bigint NOT NULL CHECK (total_cents > 0),
    state text NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'paid')),
    created_at timestamptz NOT NULL DEFAULT now(),
    paid_at timestamptz,
    UNIQUE (business_id, id),
    FOREIGN KEY (business_id, customer_id) REFERENCES customers(business_id, id),
    CHECK ((state = 'paid') = (paid_at IS NOT NULL)),
    CHECK (paid_at IS NULL OR paid_at >= created_at)
);

CREATE TABLE invoice_line_items (
    invoice_id bigint NOT NULL REFERENCES invoices(id),
    position integer NOT NULL CHECK (position >= 0),
    description text NOT NULL CHECK (length(trim(description)) > 0),
    quantity bigint NOT NULL CHECK (quantity > 0),
    unit_amount_cents bigint NOT NULL CHECK (unit_amount_cents >= 0),
    PRIMARY KEY (invoice_id, position),
    -- Numeric multiplication avoids overflow while checking each line's i64 ceiling.
    CHECK (quantity::numeric * unit_amount_cents <= 9223372036854775807)
);

CREATE TABLE payment_attempts (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    business_id bigint NOT NULL REFERENCES businesses(id),
    invoice_id bigint NOT NULL,
    idempotency_key text NOT NULL CHECK (length(trim(idempotency_key)) > 0),
    mock_card_token text NOT NULL CHECK (length(trim(mock_card_token)) > 0),
    state text NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'succeeded', 'failed')),
    psp_ref uuid,
    failure_code text CHECK (length(trim(failure_code)) > 0),
    -- Preserve the original serialized HTTP 202 response, even after settlement.
    accepted_response_body text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    next_check_at timestamptz,
    UNIQUE (business_id, idempotency_key),
    FOREIGN KEY (business_id, invoice_id) REFERENCES invoices(business_id, id),
    CHECK (state <> 'succeeded' OR psp_ref IS NOT NULL),
    CHECK ((state = 'failed') = (failure_code IS NOT NULL)),
    CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX payment_attempts_one_active_per_invoice
    ON payment_attempts(invoice_id) WHERE state IN ('pending', 'succeeded');
CREATE INDEX payment_attempts_due ON payment_attempts(next_check_at)
    WHERE state = 'pending' AND next_check_at IS NOT NULL;
CREATE INDEX invoices_customer ON invoices(business_id, customer_id);
CREATE INDEX invoices_business_state ON invoices(business_id, state, id);

CREATE TABLE webhook_endpoints (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    business_id bigint NOT NULL UNIQUE REFERENCES businesses(id),
    url text NOT NULL CHECK (length(trim(url)) > 0),
    signing_secret text NOT NULL CHECK (length(signing_secret) > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (business_id, id)
);

CREATE TABLE webhook_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    business_id bigint NOT NULL REFERENCES businesses(id),
    invoice_id bigint NOT NULL,
    endpoint_id bigint,
    event_type text NOT NULL CHECK (event_type IN
        ('invoice.created', 'invoice.paid', 'invoice.payment_failed')),
    payload text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    next_attempt_at timestamptz,
    attempt_count bigint NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_attempt_at timestamptz,
    delivered_at timestamptz,
    exhausted_at timestamptz,
    FOREIGN KEY (business_id, invoice_id) REFERENCES invoices(business_id, id),
    FOREIGN KEY (business_id, endpoint_id) REFERENCES webhook_endpoints(business_id, id),
    CHECK (endpoint_id IS NOT NULL OR (next_attempt_at IS NULL AND attempt_count = 0
            AND last_attempt_at IS NULL AND delivered_at IS NULL AND exhausted_at IS NULL)),
    CHECK (delivered_at IS NULL OR exhausted_at IS NULL),
    CHECK ((delivered_at IS NULL AND exhausted_at IS NULL) OR next_attempt_at IS NULL)
);
CREATE INDEX webhook_events_due ON webhook_events(next_attempt_at)
    WHERE next_attempt_at IS NOT NULL AND delivered_at IS NULL AND exhausted_at IS NULL;
