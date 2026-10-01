#!/bin/sh
set -eu
psql --set=ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" \
    --set=app_password="$APP_DB_PASSWORD" --set=provider_password="$PROVIDER_DB_PASSWORD" <<'SQL'
CREATE ROLE payment_app LOGIN PASSWORD :'app_password';
CREATE ROLE payment_provider LOGIN PASSWORD :'provider_password';
CREATE DATABASE payment_app OWNER payment_app;
CREATE DATABASE payment_provider OWNER payment_provider;
REVOKE CONNECT ON DATABASE payment_app FROM PUBLIC;
REVOKE CONNECT ON DATABASE payment_provider FROM PUBLIC;
GRANT CONNECT ON DATABASE payment_app TO payment_app;
GRANT CONNECT ON DATABASE payment_provider TO payment_provider;
SQL
