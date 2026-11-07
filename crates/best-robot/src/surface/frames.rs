//! A camera's images, off BEST (behavioralstate/spec#266). A topic of frames is read by query as an address and the
//! SHA-256 of the image's bytes, never the bytes, which would not fit an answer and should not be in a log; the door
//! keeps the latest few images it handed out and serves each at `/frames/<sha256>`, beside its BEST surface. The digest
//! is what reaches it: an image nobody was handed cannot be named, and one no longer kept is gone.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use robot::Frame;
use sha2::{Digest, Sha256};

/// How many images the door keeps for their addresses to answer.
pub const KEPT: usize = 8;

/// The images the door handed out, newest last, each with its digest.
pub struct Frames {
    endpoint: String,
    kept: Mutex<VecDeque<(String, Arc<Vec<u8>>)>>,
}

/// An image as the door names it: where it is served and the digest of its bytes.
pub struct Named {
    pub url: String,
    pub sha256: String,
}

impl Frames {
    /// The door's images, their addresses under `endpoint`.
    pub fn new(endpoint: &str) -> Arc<Self> {
        Arc::new(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            kept: Mutex::new(VecDeque::new()),
        })
    }

    /// Keeps `frame`, the oldest kept let go past [`KEPT`], and names it.
    pub fn keep(&self, frame: &Frame) -> Named {
        let sha256: String = Sha256::digest(&frame.bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut kept = self.kept.lock().unwrap();
        if !kept.iter().any(|(d, _)| *d == sha256) {
            kept.push_back((sha256.clone(), Arc::new(frame.bytes.clone())));
            while kept.len() > KEPT {
                kept.pop_front();
            }
        }
        Named {
            url: format!("{}/frames/{sha256}", self.endpoint),
            sha256,
        }
    }

    fn get(&self, sha256: &str) -> Option<Arc<Vec<u8>>> {
        let kept = self.kept.lock().unwrap();
        kept.iter().find(|(d, _)| d == sha256).map(|(_, b)| b.clone())
    }

    /// `GET /frames/<sha256>`: a kept image's bytes, typed as what they are.
    pub fn routes(self: Arc<Self>) -> axum::Router {
        axum::Router::new()
            .route("/frames/:sha256", axum::routing::get(serve))
            .with_state(self)
    }
}

async fn serve(State(frames): State<Arc<Frames>>, Path(sha256): Path<String>) -> Response {
    match frames.get(&sha256) {
        Some(bytes) => ([(header::CONTENT_TYPE, media_type(&bytes))], bytes.to_vec()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            format!("no image {sha256} is kept: the door keeps the latest {KEPT} it handed out; read the topic again"),
        )
            .into_response(),
    }
}

/// What image `bytes` are, by their first bytes.
fn media_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn frame(n: u8) -> Frame {
        Frame {
            format: "jpeg".into(),
            bytes: vec![0xff, 0xd8, 0xff, n],
            received_at: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    async fn get(frames: &Arc<Frames>, sha256: &str) -> (StatusCode, Option<String>, Vec<u8>) {
        let res = frames
            .clone()
            .routes()
            .oneshot(Request::get(format!("/frames/{sha256}")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let kind = res
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string());
        (
            status,
            kind,
            res.into_body().collect().await.unwrap().to_bytes().to_vec(),
        )
    }

    #[tokio::test]
    async fn a_kept_image_is_served_at_its_digest_and_the_oldest_goes_past_the_latest_few() {
        let frames = Frames::new("http://127.0.0.1:7780/");
        let first = frames.keep(&frame(0));
        assert_eq!(first.url, format!("http://127.0.0.1:7780/frames/{}", first.sha256));
        let (status, kind, bytes) = get(&frames, &first.sha256).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(kind.as_deref(), Some("image/jpeg"));
        assert_eq!(bytes, frame(0).bytes);
        // The same image again is kept once.
        frames.keep(&frame(0));
        for n in 1..KEPT as u8 {
            frames.keep(&frame(n));
        }
        assert_eq!(get(&frames, &first.sha256).await.0, StatusCode::OK);
        frames.keep(&frame(KEPT as u8));
        let (status, _, said) = get(&frames, &first.sha256).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(String::from_utf8(said).unwrap().contains("read the topic again"));
        assert_eq!(get(&frames, "0".repeat(64).as_str()).await.0, StatusCode::NOT_FOUND);
    }
}
