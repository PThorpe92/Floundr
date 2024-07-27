use std::{io, path::PathBuf};

use futures::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tracing::{debug, error};

pub static OCI_CONTENT_HEADER: &str = "application/vnd.oci.image.index.v1+json";
pub static DOCKER_DIGEST: &str = "Docker-Content-Digest";

pub fn calculate_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn validate_digest(data: &[u8], digest: &str) -> Result<(), Box<dyn std::error::Error>> {
    let calculated_digest = calculate_digest(data);
    if calculated_digest != digest {
        return Err("Digest mismatch".into());
    }
    Ok(())
}

pub fn path_is_valid(path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut components = path.components().peekable();

    if let Some(first) = components.peek() {
        if !matches!(first, std::path::Component::Normal(_)) {
            return false;
        }
    }

    components.count() == 1
}

fn visit(path: impl Into<PathBuf>) -> impl Stream<Item = io::Result<u64>> + Send + 'static {
    async fn one_level(path: PathBuf, to_visit: &mut Vec<PathBuf>) -> io::Result<Vec<u64>> {
        let mut dir = tokio::fs::read_dir(&path).await?;
        let mut files = Vec::new();

        while let Some(child) = dir.next_entry().await? {
            let metadata = child.metadata().await?;
            if metadata.is_dir() {
                to_visit.push(child.path());
            } else {
                let size = metadata.len();
                files.push(size);
            }
        }

        Ok(files)
    }
    futures::stream::unfold(vec![path.into()], |mut to_visit| async {
        let path = to_visit.pop()?;
        let file_stream = match one_level(path, &mut to_visit).await {
            Ok(files) => futures::stream::iter(files).map(Ok).left_stream(),
            Err(e) => futures::stream::once(async { Err(e) }).right_stream(),
        };

        Some((file_stream, to_visit))
    })
    .flatten()
}

pub async fn get_dir_size(path: PathBuf) -> u64 {
    debug!("calculating size of dir: {:?}", path);
    visit(path)
        .fold(0u64, |acc, entry| async move {
            acc + entry.unwrap_or_else(|e| {
                error!("error getting directory size: {e}");
                0
            })
        })
        .await
}
