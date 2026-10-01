use sqlx::PgPool;

async fn rejects(pool: &PgPool, statement: &'static str, code: &str) {
    let error = sqlx::query(statement).execute(pool).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some(code),
        "{statement}"
    );
}

async fn seed(pool: &PgPool) {
    sqlx::raw_sql(
        "INSERT INTO businesses (id, name) OVERRIDING SYSTEM VALUE VALUES (1, 'One'), (2, 'Two');
         INSERT INTO customers (id, business_id, name, email) OVERRIDING SYSTEM VALUE
            VALUES (1, 1, 'Alice', 'alice@example.com'), (2, 2, 'Bob', 'bob@example.com');
         INSERT INTO invoices (id, business_id, customer_id, due_date, total_cents) OVERRIDING SYSTEM VALUE
            VALUES (1, 1, 1, CURRENT_DATE, 100), (2, 2, 2, CURRENT_DATE, 100),
                   (3, 1, 1, CURRENT_DATE, 100);",
    ).execute(pool).await.unwrap();
}

#[sqlx::test]
async fn amounts_and_states(pool: PgPool) {
    seed(&pool).await;
    for statement in [
        "UPDATE invoices SET total_cents = 0 WHERE id = 1",
        "UPDATE invoices SET total_cents = -1 WHERE id = 1",
        "UPDATE invoices SET state = 'cancelled' WHERE id = 1",
        "UPDATE invoices SET state = 'paid' WHERE id = 1",
        "INSERT INTO invoice_line_items VALUES (1, 0, 'Item', 0, 100)",
        "INSERT INTO invoice_line_items VALUES (1, 0, 'Item', 1, -1)",
        "INSERT INTO invoice_line_items VALUES (1, 0, 'Item', 2, 9223372036854775807)",
    ] {
        rejects(&pool, statement, "23514").await;
    }
    rejects(
        &pool,
        "UPDATE invoices SET total_cents = '1.5' WHERE id = 1",
        "22P02",
    )
    .await;
    // Both the zero-priced line and largest representable cents value are valid.
    sqlx::raw_sql("INSERT INTO invoice_line_items VALUES (1, 0, 'Free', 1, 0), (1, 1, 'Max', 1, 9223372036854775807);
        UPDATE invoices SET state = 'paid', paid_at = now() WHERE id = 1;
        INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
            VALUES (1, 3, 'payment', 'tok_success', '{}');")
        .execute(&pool).await.unwrap();
    for statement in [
        "UPDATE payment_attempts SET state = 'unknown'",
        "UPDATE payment_attempts SET state = 'failed'",
        "UPDATE payment_attempts SET state = 'succeeded'",
        "UPDATE payment_attempts SET failure_code = 'declined'",
    ] {
        rejects(&pool, statement, "23514").await;
    }
    sqlx::raw_sql(
        "UPDATE payment_attempts SET state = 'failed', failure_code = 'declined';
        INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
            VALUES (1, 3, 'retry', 'tok_success', '{}');
        UPDATE payment_attempts SET state = 'succeeded',
            psp_ref = '00000000-0000-4000-8000-000000000001' WHERE idempotency_key = 'retry';",
    )
    .execute(&pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn tenant_boundaries_and_payment_uniqueness(pool: PgPool) {
    seed(&pool).await;
    sqlx::raw_sql("INSERT INTO webhook_endpoints (id, business_id, url, signing_secret) OVERRIDING SYSTEM VALUE
        VALUES (1, 1, 'https://one.example/webhook', 'secret'), (2, 2, 'https://two.example/webhook', 'secret');
        INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
        VALUES (1, 1, 'same-key', 'tok_success', '{}'), (2, 2, 'same-key', 'tok_success', '{}');")
        .execute(&pool).await.unwrap();
    for statement in [
        "UPDATE invoices SET customer_id = 2 WHERE id = 1",
        "INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body, state, failure_code)
            VALUES (1, 2, 'cross-business', 'tok_success', '{}', 'failed', 'declined')",
        "INSERT INTO webhook_events (business_id, invoice_id, event_type, payload) VALUES (1, 2, 'invoice.created', '{}')",
        "INSERT INTO webhook_events (business_id, invoice_id, endpoint_id, event_type, payload)
            VALUES (1, 1, 2, 'invoice.created', '{}')",
    ] {
        rejects(&pool, statement, "23503").await;
    }
    rejects(
        &pool,
        "INSERT INTO webhook_events (business_id, invoice_id, event_type, payload, next_attempt_at)
        VALUES (1, 1, 'invoice.created', '{}', now())",
        "23514",
    )
    .await;
    let payload = "{ \"event\": \"invoice.created\", \"amount_cents\": 100 }\n";
    let stored: String = sqlx::query_scalar("INSERT INTO webhook_events (business_id, invoice_id, endpoint_id, event_type, payload, next_attempt_at)
        VALUES (1, 1, 1, 'invoice.created', $1, now()) RETURNING payload")
        .bind(payload).fetch_one(&pool).await.unwrap();
    assert_eq!(stored, payload);
    for statement in [
        "INSERT INTO webhook_endpoints (id, business_id, url, signing_secret) OVERRIDING SYSTEM VALUE VALUES (3, 1, 'https://other.example', 'secret')",
        "INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
            VALUES (1, 3, 'same-key', 'tok_success', '{}')",
        "INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
            VALUES (1, 1, 'second-key', 'tok_success', '{}')",
    ] {
        rejects(&pool, statement, "23505").await;
    }
    sqlx::raw_sql("UPDATE payment_attempts SET state = 'failed', failure_code = 'declined' WHERE business_id = 1;
        INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
            VALUES (1, 1, 'retry', 'tok_success', '{}');
        UPDATE payment_attempts SET state = 'succeeded', psp_ref = '00000000-0000-4000-8000-000000000001'
            WHERE business_id = 1 AND idempotency_key = 'retry';")
        .execute(&pool).await.unwrap();
    rejects(&pool, "INSERT INTO payment_attempts (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body)
        VALUES (1, 1, 'after-success', 'tok_success', '{}')", "23505").await;
}

#[sqlx::test(migrations = "./provider_migrations")]
async fn provider_operations_are_durable_and_unique(pool: PgPool) {
    sqlx::query("INSERT INTO provider_operations
        (attempt_id, invoice_id, amount_cents, mock_card_token, psp_ref, planned_outcome, ready_at)
        VALUES (1, 9, 100, 'tok_timeout', '00000000-0000-4000-8000-000000000001', 'succeeded', now() + interval '30 seconds')")
        .execute(&pool).await.unwrap();
    for statement in [
        "INSERT INTO provider_operations SELECT * FROM provider_operations",
        "INSERT INTO provider_operations SELECT 2, invoice_id, amount_cents, mock_card_token, psp_ref, planned_outcome, ready_at, created_at FROM provider_operations",
    ] {
        rejects(&pool, statement, "23505").await;
    }
    for statement in [
        "UPDATE provider_operations SET amount_cents = 0",
        "UPDATE provider_operations SET planned_outcome = ''",
        "UPDATE provider_operations SET ready_at = created_at - interval '1 second'",
    ] {
        rejects(&pool, statement, "23514").await;
    }
    rejects(
        &pool,
        "UPDATE provider_operations SET psp_ref = NULL",
        "23502",
    )
    .await;
    let persisted: (i64, String, String, String, bool) = sqlx::query_as(
        "SELECT amount_cents, mock_card_token, psp_ref::text, planned_outcome, ready_at > created_at
         FROM provider_operations WHERE attempt_id = 1",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(
        persisted,
        (
            100,
            "tok_timeout".into(),
            "00000000-0000-4000-8000-000000000001".into(),
            "succeeded".into(),
            true
        )
    );
    let app_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.invoices')::text")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(app_table.is_none());
}
