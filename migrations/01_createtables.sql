-- create_table.sql

CREATE TABLE IF NOT EXISTS repositories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    is_public BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS blobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    digest TEXT NOT NULL,
    file_path TEXT NOT NULL,
    upload_session_id TEXT,
    ref_count INTEGER NOT NULL DEFAULT 0,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    manifest_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    tag TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE,
    FOREIGN KEY (manifest_id) REFERENCES manifests(id) ON DELETE CASCADE,
    UNIQUE (repository_id, tag)
);

CREATE TABLE IF NOT EXISTS manifests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    digest TEXT NOT NULL UNIQUE,
    media_type TEXT NOT NULL,
    file_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS manifest_layers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    manifest_id INTEGER NOT NULL,
    digest TEXT NOT NULL,
    size INTEGER NOT NULL,
    media_type TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (manifest_id) REFERENCES manifests(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS uploads (
    uuid TEXT NOT NULL PRIMARY KEY,
    repository_id INTEGER NOT NULL,
    current_chunk INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id),
    UNIQUE (repository_id, uuid)
);


CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS repository_scopes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    push BOOLEAN NOT NULL DEFAULT FALSE,
    pull BOOLEAN NOT NULL DEFAULT FALSE,
    del BOOLEAN NOT NULL DEFAULT FALSE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS clients (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    secret TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE TABLE IF NOT EXISTS tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account INTEGER NOT NULL,
    token TEXT NOT NULL UNIQUE,
    client_id TEXT UNIQUE,
    expires TIMESTAMP NOT NULL DEFAULT (datetime('now', '+1 day')),
    FOREIGN KEY (client_id) REFERENCES clients(client_id) ON DELETE CASCADE,
    FOREIGN KEY (account) REFERENCES users(email)
);

CREATE INDEX IF NOT EXISTS idx_blobs_digest ON blobs (digest);
CREATE INDEX IF NOT EXISTS idx_upload_session_id ON blobs (upload_session_id);
CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags (tag);
CREATE INDEX IF NOT EXISTS idx_manifests_digest ON manifests (digest);
CREATE INDEX IF NOT EXISTS idx_manifest_layers_digest ON manifest_layers (digest);
CREATE INDEX IF NOT EXISTS idx_uploads_uuid ON uploads (uuid);
CREATE INDEX IF NOT EXISTS idx_users_email ON users (email);
CREATE INDEX IF NOT EXISTS idx_repository_scopes_user_id ON repository_scopes (user_id);
CREATE INDEX IF NOT EXISTS idx_clients_secret ON clients (secret);

CREATE TRIGGER IF NOT EXISTS add_scopes_on_new_user
AFTER INSERT ON users
BEGIN
    INSERT INTO repository_scopes (user_id, repository_id, push, pull, del)
    SELECT
        NEW.id,
        repositories.id,
        CASE WHEN NEW.is_admin THEN TRUE ELSE FALSE END,
        CASE WHEN repositories.is_public THEN TRUE ELSE FALSE END,
        CASE WHEN NEW.is_admin THEN TRUE ELSE FALSE END
    FROM repositories;
END;

CREATE TRIGGER IF NOT EXISTS add_scopes_on_new_repository
AFTER INSERT ON repositories
BEGIN
    INSERT INTO repository_scopes (user_id, repository_id, push, pull, del)
    SELECT
        users.id,
        NEW.id,
        CASE WHEN users.is_admin THEN TRUE ELSE FALSE END,
        CASE WHEN repositories.is_public THEN TRUE ELSE FALSE END,
        CASE WHEN users.is_admin THEN TRUE ELSE FALSE END
    FROM users JOIN repositories on 1=1;
END;

INSERT INTO repositories (name, is_public)
SELECT 'default', 1
WHERE NOT EXISTS (SELECT 1 FROM repositories);
