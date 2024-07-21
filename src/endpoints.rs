#[derive(Debug)]
pub enum Endpoint {
    GetV2,
    GetHeadBlobs,
    GetHeadManifests,
    PostBlobsUploads,
    PostBlobsUploadsWithDigest,
    PatchBlobsUploads,
    PutBlobsUploadsWithDigest,
    PutManifests,
    GetTagsList,
    GetTagsListWithParams,
    DeleteManifests,
    DeleteBlobs,
    PostBlobsUploadsMount,
    GetReferrers,
    GetReferrersWithArtifactType,
    GetBlobsUploads,
}

impl Endpoint {
    pub fn from_end_id(end_id: &str) -> Option<Endpoint> {
        match end_id {
            "end-1" => Some(Endpoint::GetV2),
            "end-2" => Some(Endpoint::GetHeadBlobs),
            "end-3" => Some(Endpoint::GetHeadManifests),
            "end-4a" => Some(Endpoint::PostBlobsUploads),
            "end-4b" => Some(Endpoint::PostBlobsUploadsWithDigest),
            "end-5" => Some(Endpoint::PatchBlobsUploads),
            "end-6" => Some(Endpoint::PutBlobsUploadsWithDigest),
            "end-7" => Some(Endpoint::PutManifests),
            "end-8a" => Some(Endpoint::GetTagsList),
            "end-8b" => Some(Endpoint::GetTagsListWithParams),
            "end-9" => Some(Endpoint::DeleteManifests),
            "end-10" => Some(Endpoint::DeleteBlobs),
            "end-11" => Some(Endpoint::PostBlobsUploadsMount),
            "end-12a" => Some(Endpoint::GetReferrers),
            "end-12b" => Some(Endpoint::GetReferrersWithArtifactType),
            "end-13" => Some(Endpoint::GetBlobsUploads),
            _ => None,
        }
    }

    pub fn method(&self) -> &str {
        match self {
            Endpoint::GetV2 => "GET",
            Endpoint::GetHeadBlobs => "GET / HEAD",
            Endpoint::GetHeadManifests => "GET / HEAD",
            Endpoint::PostBlobsUploads => "POST",
            Endpoint::PostBlobsUploadsWithDigest => "POST",
            Endpoint::PatchBlobsUploads => "PATCH",
            Endpoint::PutBlobsUploadsWithDigest => "PUT",
            Endpoint::PutManifests => "PUT",
            Endpoint::GetTagsList => "GET",
            Endpoint::GetTagsListWithParams => "GET",
            Endpoint::DeleteManifests => "DELETE",
            Endpoint::DeleteBlobs => "DELETE",
            Endpoint::PostBlobsUploadsMount => "POST",
            Endpoint::GetReferrers => "GET",
            Endpoint::GetReferrersWithArtifactType => "GET",
            Endpoint::GetBlobsUploads => "GET",
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Endpoint::GetV2 => "/v2/",
            Endpoint::GetHeadBlobs => "/v2/<name>/blobs/<digest>",
            Endpoint::GetHeadManifests => "/v2/<name>/manifests/<reference>",
            Endpoint::PostBlobsUploads => "/v2/<name>/blobs/uploads/",
            Endpoint::PostBlobsUploadsWithDigest => "/v2/<name>/blobs/uploads/?digest=<digest>",
            Endpoint::PatchBlobsUploads => "/v2/<name>/blobs/uploads/<reference>",
            Endpoint::PutBlobsUploadsWithDigest => {
                "/v2/<name>/blobs/uploads/<reference>?digest=<digest>"
            }
            Endpoint::PutManifests => "/v2/<name>/manifests/<reference>",
            Endpoint::GetTagsList => "/v2/<name>/tags/list",
            Endpoint::GetTagsListWithParams => "/v2/<name>/tags/list?n=<integer>&last=<integer>",
            Endpoint::DeleteManifests => "/v2/<name>/manifests/<reference>",
            Endpoint::DeleteBlobs => "/v2/<name>/blobs/<digest>",
            Endpoint::PostBlobsUploadsMount => {
                "/v2/<name>/blobs/uploads/?mount=<digest>&from=<other_name>"
            }
            Endpoint::GetReferrers => "/v2/<name>/referrers/<digest>",
            Endpoint::GetReferrersWithArtifactType => {
                "/v2/<name>/referrers/<digest>?artifactType=<artifactType>"
            }
            Endpoint::GetBlobsUploads => "/v2/<name>/blobs/uploads/<reference>",
        }
    }

    pub fn success_status(&self) -> Vec<u16> {
        match self {
            Endpoint::GetV2 => vec![200],
            Endpoint::GetHeadBlobs => vec![200],
            Endpoint::GetHeadManifests => vec![200],
            Endpoint::PostBlobsUploads => vec![202],
            Endpoint::PostBlobsUploadsWithDigest => vec![201, 202],
            Endpoint::PatchBlobsUploads => vec![202],
            Endpoint::PutBlobsUploadsWithDigest => vec![201],
            Endpoint::PutManifests => vec![201],
            Endpoint::GetTagsList => vec![200],
            Endpoint::GetTagsListWithParams => vec![200],
            Endpoint::DeleteManifests => vec![202],
            Endpoint::DeleteBlobs => vec![202],
            Endpoint::PostBlobsUploadsMount => vec![201],
            Endpoint::GetReferrers => vec![200],
            Endpoint::GetReferrersWithArtifactType => vec![200],
            Endpoint::GetBlobsUploads => vec![204],
        }
    }

    pub fn error_status(&self) -> Vec<u16> {
        match self {
            Endpoint::GetV2 => vec![404, 401],
            Endpoint::GetHeadBlobs => vec![404],
            Endpoint::GetHeadManifests => vec![404],
            Endpoint::PostBlobsUploads => vec![404],
            Endpoint::PostBlobsUploadsWithDigest => vec![404, 400],
            Endpoint::PatchBlobsUploads => vec![404, 416],
            Endpoint::PutBlobsUploadsWithDigest => vec![404, 400],
            Endpoint::PutManifests => vec![404],
            Endpoint::GetTagsList => vec![404],
            Endpoint::GetTagsListWithParams => vec![404],
            Endpoint::DeleteManifests => vec![404, 400, 405],
            Endpoint::DeleteBlobs => vec![404, 405],
            Endpoint::PostBlobsUploadsMount => vec![404],
            Endpoint::GetReferrers => vec![404, 400],
            Endpoint::GetReferrersWithArtifactType => vec![404, 400],
            Endpoint::GetBlobsUploads => vec![404],
        }
    }
}
