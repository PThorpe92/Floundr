use crate::app::{
    AppResult, RepositoryList, ACTIVE_KEYS, CLIENT, GLOBAL_REPO_LIST, HEADERS, MANIFESTS, USERS,
};
use reqwest::Response;
use shared::{AuthClient, ImageManifest, RegisterUserRequest, UserResponse};
use tracing::{debug, info};

pub async fn get_manifests(url: &str) -> AppResult<()> {
    let repos = GLOBAL_REPO_LIST.read().unwrap().repositories.clone();
    for repo in repos.iter() {
        for tag in repo.tags.clone() {
            info!("fetching manifest for {}/{}", repo.name, tag);
            let res =
                send_get_request(format!("{}/v2/{}/manifests/{}", url, repo.name, tag)).await?;
            let manifests: ImageManifest = res.json().await?;
            MANIFESTS.insert(repo.name.clone(), manifests);
        }
    }
    Ok(())
}

pub async fn get_repositories(url: &str) -> AppResult<()> {
    let resp = send_get_request(format!("{}/repositories", url)).await?;
    let repos: RepositoryList = resp.json().await?;
    debug!("{:?}", repos.repositories);
    *GLOBAL_REPO_LIST.write().unwrap() = repos;
    Ok(())
}

pub async fn create_repository(url: String, name: String, public: bool) -> AppResult<()> {
    let client = CLIENT.get().unwrap();
    let res = client
        .post(format!("{}/repositories/{}/{}", url, name, public))
        .send()
        .await?;
    if res.status().is_success() {
        let new = client.get(format!("{}/repositories", url)).send().await?;
        let repos: RepositoryList = new.json().await.expect("failed to parse repos");
        *GLOBAL_REPO_LIST.write().unwrap() = repos;
        info!("Repository created successfully");
        Ok(())
    } else {
        debug!("{:?}", res);
        info!("Failed to create repository");
        Err("Failed to create repository".into())
    }
}

pub async fn get_all_users(url: &str) -> AppResult<()> {
    let res = send_get_request(format!("{}/users", url)).await?;
    let users: Vec<UserResponse> = res.json().await?;
    info!("Users received: {:?}", users);
    *USERS.write().unwrap() = users;
    Ok(())
}

pub async fn create_new_api_key(url: String, name: String) -> AppResult<()> {
    let url = format!("{}/users/{}/tokens", url, &name);
    let resp = send_post_request(url, String::new()).await?;
    if resp.status().is_success() {
        info!("API key created successfully");
        Ok(())
    } else {
        debug!("{:?}", resp);
        info!("Failed to create API key");
        Err("Failed to create API key".into())
    }
}

pub async fn send_post_request<T>(url: String, json: T) -> AppResult<Response>
where
    T: serde::Serialize + std::fmt::Debug,
{
    let client = CLIENT.get().unwrap();
    let headers = HEADERS.get().unwrap();
    let req = client
        .post(url)
        .headers(headers.to_owned())
        .json(&json)
        .build()?;
    info!("sending request {:?}", req);
    let res = client.execute(req).await?;
    if res.status().is_success() {
        Ok(res)
    } else {
        Err("Failed to fetch data".into())
    }
}
pub async fn send_get_request(url: String) -> AppResult<Response> {
    let client = CLIENT.get().unwrap();
    let headers = HEADERS.get().unwrap();
    let req = client.get(url).headers(headers.to_owned()).build()?;
    info!("sending request {:?}", req);
    let res = client.execute(req).await?;
    if res.status().is_success() {
        Ok(res)
    } else {
        Err("Failed to fetch data".into())
    }
}

pub async fn send_delete_request(url: String) -> AppResult<Response> {
    let client = CLIENT.get().unwrap();
    let headers = HEADERS.get().unwrap();
    let req = client.delete(url).headers(headers.to_owned()).build()?;
    info!("sending request {:?}", req);
    let res = client.execute(req).await?;
    if res.status().is_success() {
        Ok(res)
    } else {
        Err("Failed to fetch data".into())
    }
}

pub async fn create_new_user(url: String, user: RegisterUserRequest) -> AppResult<Response> {
    info!("sending user {:?}", user);
    let res = send_post_request(format!("{}/v2/auth/register", &url), user).await?;
    info!("response is {:?}", res);
    if res.status().is_success() {
        get_all_users(&url).await?;
        Ok(res)
    } else {
        Err("Failed to create user".into())
    }
}

pub async fn delete_user(url: String, email: String) -> AppResult<()> {
    let url = format!("{}/users/{}", url, email);
    let res = send_delete_request(url).await?;
    if res.status().is_success() {
        info!("User deleted successfully");
        Ok(())
    } else {
        Err("Failed to delete user".into())
    }
}

pub async fn delete_repository(url: String, repo: String) -> AppResult<()> {
    let url = format!("{}/repositories/{}", url, repo);
    let res = send_delete_request(url).await?;
    if res.status().is_success() {
        info!("Repository deleted successfully");
        Ok(())
    } else {
        Err("Failed to delete repository".into())
    }
}

pub async fn get_tokens(url: &str) -> AppResult<()> {
    let url = format!("{}/v2/auth/clients", url);
    let res = send_get_request(url).await?;
    let tokens: Vec<AuthClient> = res.json().await?;
    info!("Tokens received: {:?}", tokens);
    *ACTIVE_KEYS.write().unwrap() = tokens;
    Ok(())
}
