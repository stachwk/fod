SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS data_object_request_tokens (
    request_token TEXT PRIMARY KEY,
    id_data_object INTEGER NOT NULL REFERENCES data_objects(id_data_object) ON DELETE CASCADE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);
