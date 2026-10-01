use crate::{
    AppState,
    auth::BusinessId,
    entities::{customer::Customer, invoice::InvoiceSummary, invoice_line_item::LineItem},
    error::{ApiJson, ApiPath, ApiQuery, AppError},
    events::insert_event,
    models::{
        data::{
            invoice::Invoice,
            page::{Page, page},
        },
        forms::{customer::NewCustomer, invoice::NewInvoice, list::ListQuery},
    },
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::json;
use sqlx::PgConnection;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/customers", get(list_customers).post(create_customer))
        .route("/customers/{id}", get(get_customer))
        .route("/invoices", get(list_invoices).post(create_invoice))
        .route("/invoices/{id}", get(get_invoice))
}

async fn create_customer(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiJson(input): ApiJson<NewCustomer>,
) -> Result<(StatusCode, Json<Customer>), AppError> {
    let customer = sqlx::query_as(
        "INSERT INTO customers (business_id, name, email) VALUES ($1, $2, $3)
        RETURNING id, business_id, name, email, created_at",
    )
    .bind(business_id)
    .bind(nonempty(&input.name, "name")?)
    .bind(nonempty(&input.email, "email")?)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(customer)))
}

async fn get_customer(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Customer>, AppError> {
    Ok(Json(sqlx::query_as("SELECT id, business_id, name, email, created_at FROM customers WHERE business_id = $1 AND id = $2")
        .bind(business_id).bind(id).fetch_one(&state.db).await?))
}

async fn list_customers(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<Page<Customer>>, AppError> {
    let (limit, after_id) = query.pagination()?;
    if query.state.is_some() {
        return Err(AppError::invalid_input(
            "Customers do not have a state filter",
        ));
    }
    let customers = sqlx::query_as(
        "SELECT id, business_id, name, email, created_at FROM customers
        WHERE business_id = $1 AND id > $2 ORDER BY id LIMIT $3",
    )
    .bind(business_id)
    .bind(after_id)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(page(customers, limit, |customer: &Customer| {
        customer.id
    })))
}

async fn create_invoice(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiJson(mut input): ApiJson<NewInvoice>,
) -> Result<(StatusCode, Json<Invoice>), AppError> {
    if input.line_items.is_empty() {
        return Err(AppError::invalid_input(
            "At least one line item is required",
        ));
    }
    let mut total = 0_i64;
    for item in &mut input.line_items {
        item.description = nonempty(&item.description, "description")?.to_owned();
        if item.quantity <= 0 || item.unit_amount_cents < 0 {
            return Err(AppError::invalid_input(
                "quantity must be positive and unit_amount_cents must be nonnegative",
            ));
        }
        total = item
            .quantity
            .checked_mul(item.unit_amount_cents)
            .and_then(|amount| total.checked_add(amount))
            .ok_or_else(|| AppError::invalid_input("Invoice total exceeds integer cents range"))?;
    }
    if total == 0 {
        return Err(AppError::invalid_input("Invoice total must be positive"));
    }
    let mut transaction = state.db.begin().await?;
    sqlx::query_scalar::<_, i64>("SELECT id FROM customers WHERE business_id = $1 AND id = $2")
        .bind(business_id)
        .bind(input.customer_id)
        .fetch_one(&mut *transaction)
        .await?;
    let summary: InvoiceSummary = sqlx::query_as("INSERT INTO invoices (business_id, customer_id, due_date, total_cents)
        VALUES ($1, $2, $3, $4) RETURNING id, business_id, customer_id, due_date, total_cents, state, created_at, paid_at")
        .bind(business_id).bind(input.customer_id).bind(input.due_date).bind(total).fetch_one(&mut *transaction).await?;
    for (position, item) in input.line_items.iter().enumerate() {
        sqlx::query("INSERT INTO invoice_line_items (invoice_id, position, description, quantity, unit_amount_cents) VALUES ($1, $2, $3, $4, $5)")
            .bind(summary.id).bind(i32::try_from(position).map_err(AppError::internal)?).bind(&item.description)
            .bind(item.quantity).bind(item.unit_amount_cents).execute(&mut *transaction).await?;
    }
    insert_event(&mut transaction, business_id, summary.id, "invoice.created",
        &json!({"invoice_id":summary.id,"business_id":business_id,"customer_id":summary.customer_id,"state":summary.state,"total_cents":total})).await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(Invoice {
            summary,
            line_items: input.line_items,
        }),
    ))
}

async fn get_invoice(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Invoice>, AppError> {
    let mut connection = state.db.acquire().await?;
    let summary = sqlx::query_as(
        "SELECT id, business_id, customer_id, due_date, total_cents, state, created_at, paid_at
        FROM invoices WHERE business_id = $1 AND id = $2",
    )
    .bind(business_id)
    .bind(id)
    .fetch_one(&mut *connection)
    .await?;
    let line_items = line_items(&mut connection, id).await?;
    Ok(Json(Invoice {
        summary,
        line_items,
    }))
}

async fn line_items(
    connection: &mut PgConnection,
    invoice_id: i64,
) -> Result<Vec<LineItem>, AppError> {
    Ok(sqlx::query_as("SELECT description, quantity, unit_amount_cents FROM invoice_line_items WHERE invoice_id = $1 ORDER BY position")
        .bind(invoice_id).fetch_all(connection).await?)
}

async fn list_invoices(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<Page<InvoiceSummary>>, AppError> {
    let (limit, after_id) = query.pagination()?;
    if let Some(state) = &query.state
        && !matches!(state.as_str(), "open" | "paid")
    {
        return Err(AppError::invalid_input("state must be open or paid"));
    }
    let invoices = sqlx::query_as("SELECT id, business_id, customer_id, due_date, total_cents, state, created_at, paid_at
        FROM invoices WHERE business_id = $1 AND id > $2 AND ($3::text IS NULL OR state = $3) ORDER BY id LIMIT $4")
        .bind(business_id).bind(after_id).bind(query.state).bind(limit + 1).fetch_all(&state.db).await?;
    Ok(Json(page(invoices, limit, |invoice: &InvoiceSummary| {
        invoice.id
    })))
}

fn nonempty<'a>(value: &'a str, field: &str) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() || value.contains('\0') {
        return Err(AppError::invalid_input(format!(
            "{field} must be nonempty text"
        )));
    }
    Ok(value)
}
