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
    ref_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    manifest_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    tag TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id),
    FOREIGN KEY (manifest_id) REFERENCES manifests(id),
    UNIQUE (repository_id, tag)
);

CREATE TABLE IF NOT EXISTS manifests (
    id TEXT PRIMARY KEY,
    repository_id INTEGER NOT NULL,
    digest TEXT NOT NULL UNIQUE,
    media_type TEXT NOT NULL,
    file_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    schema_version INTEGER NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);

CREATE TABLE IF NOT EXISTS manifest_layers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    manifest_id INTEGER NOT NULL,
    digest TEXT NOT NULL,
    size INTEGER NOT NULL,
    media_type TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (manifest_id) REFERENCES manifests(id)
);

CREATE TABLE IF NOT EXISTS uploads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id INTEGER NOT NULL,
    uuid TEXT NOT NULL,
    blob_id INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (repository_id) REFERENCES repositories(id),
    FOREIGN KEY (blob_id) REFERENCES blobs(id),
    UNIQUE (repository_id, uuid, blob_id)
);


CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS repository_permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    repository_id INTEGER NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id),
    FOREIGN KEY (repository_id) REFERENCES repositories(id)
);

CREATE TABLE IF NOT EXISTS clients (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id TEXT NOT NULL UNIQUE,
    secret TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account INTEGER NOT NULL,
    client_id TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    expires TIMESTAMP NOT NULL DEFAULT (datetime('now', '+1 day')),
    FOREIGN KEY (account) REFERENCES users(email)
);


-- if a repository is public, we automatically add each user into the permissions table for that repository

CREATE TRIGGER IF NOT EXISTS add_user_to_repo_permissions
AFTER INSERT ON repositories
  FOR EACH ROW WHEN NEW.is_public = 1
  BEGIN
      INSERT INTO repository_permissions (user_id, repository_id)
      SELECT id, NEW.id FROM users;
  END;
-- if there are no repositories, we add a default public repository
INSERT INTO repositories (name, is_public)
SELECT 'default', 1
WHERE NOT EXISTS (SELECT 1 FROM repositories);

INSERT INTO clients (client_id, secret) SELECT 'harbor_tui', '6f9da386-c2b9-43e2-af17-bf685d981287' WHERE NOT EXISTS (SELECT 1 FROM clients);
