use crate::database::DbConn;
use axum::{extract::Path, http::StatusCode, response::IntoResponse, Json};
use shared::User;
use shared::{RepoScope, UserResponse};

pub async fn get_users(DbConn(mut conn): DbConn) -> impl IntoResponse {
    let users = sqlx::query_as!(User, "SELECT * FROM users")
        .fetch_all(&mut *conn)
        .await
        .expect("unable to fetch users");
    let repositories = sqlx::query!("SELECT * FROM repositories")
        .fetch_all(&mut *conn)
        .await
        .expect("unable to fetch repositories");
    let mut user_resp = vec![];
    for user in users.iter() {
        let mut user_scopes = vec![];
        for repo in repositories.iter() {
            let scopes = match user.is_admin {
                true => vec!["pull".to_string(), "push".to_string(), "delete".to_string()],
                false => {
                    let scopes = sqlx::query!("SELECT push, pull, del FROM repository_scopes WHERE user_id = ? AND repository_id = ?", user.id, repo.id)
                        .fetch_one(&mut *conn)
                        .await
                        .expect("unable to fetch permissions");
                    let mut permissions = vec![];
                    if scopes.push {
                        permissions.push("push".to_string());
                    }
                    if scopes.pull {
                        permissions.push("pull".to_string());
                    }
                    if scopes.del {
                        permissions.push("delete".to_string());
                    }
                    permissions
                }
            };
            user_scopes.push(RepoScope {
                repo: repo.name.clone(),
                scope: scopes,
            });
        }
        user_resp.push(UserResponse {
            user: user.clone(),
            scopes: user_scopes,
        });
    }
    (StatusCode::OK, Json(user_resp)).into_response()
}

pub async fn delete_user(Path(email): Path<String>, DbConn(mut conn): DbConn) -> impl IntoResponse {
    let _ = sqlx::query!("DELETE FROM users WHERE email = ?", email)
        .execute(&mut *conn)
        .await
        .expect("unable to delete user");
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn add_scope(
    Path((email, repo, scope)): Path<(String, String, String)>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    let scopes = sqlx::query!(
        "SELECT * FROM repository_scopes WHERE user_id = (SELECT id FROM users WHERE email = ?) AND repository_id = (SELECT id FROM repositories WHERE name = ?)",
        email,
        repo,
    )
    .fetch_one(&mut *conn)
    .await
    .expect("unable to find permissions");
    let mut push = scopes.push;
    let mut pull = scopes.pull;
    let mut del = scopes.del;
    match scope.as_str() {
        "push" => {
            push = true;
            pull = true;
        }
        "pull" => {
            pull = true;
        }
        "delete" => {
            del = true;
            push = true;
            pull = true;
        }
        _ => {}
    }
    let _ = sqlx::query!("UPDATE repository_scopes SET push = ?, pull = ?, del = ? WHERE user_id = ? AND repository_id = ?", push, pull, del, scopes.user_id, scopes.repository_id)
        .execute(&mut *conn)
        .await
        .expect("unable to update permissions");
    (StatusCode::NO_CONTENT, "").into_response()
}

pub async fn generate_token(
    Path(email): Path<String>,
    DbConn(mut conn): DbConn,
) -> impl IntoResponse {
    let token = crate::database::generate_secret(&mut conn, None, &email)
        .await
        .unwrap();
    (
        StatusCode::OK,
        serde_json::to_string(&crate::auth::TokenResponse::new(&token)).unwrap(),
    )
        .into_response()
}
