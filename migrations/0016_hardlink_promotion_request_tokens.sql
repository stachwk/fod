SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS hardlink_promotion_request_tokens (
    request_token TEXT PRIMARY KEY,
    id_file INTEGER NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    did_promote BOOLEAN NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);
