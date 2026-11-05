//! The owner's routes on the door, on the machine's admin token, as `best-robot hold` and `best-robot release` send
//! them: a hold takes the robot over, so no skill starts, and with `now` stops the runs in flight, each told to its
//! holder; a release gives it back.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Form, Json, Router};
use best_peer_door::{Approver, LocalApprover};
use serde::Deserialize;
use serde_json::json;

use crate::guard::hold::Hold;
use crate::guard::Guard;

#[derive(Clone)]
pub struct Owner {
    pub hold: Arc<Hold>,
    /// The guard of the robot's skills, when it has any: what a hold with `now` stops.
    pub guard: Option<Arc<Guard>>,
    pub approver: Arc<LocalApprover>,
}

#[derive(Deserialize)]
struct HoldForm {
    proof: String,
    #[serde(default)]
    now: bool,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct ReleaseForm {
    proof: String,
}

pub fn routes(owner: Owner) -> Router {
    Router::new()
        .route("/owner/hold", post(hold))
        .route("/owner/release", post(release))
        .with_state(owner)
}

async fn hold(State(o): State<Owner>, Form(f): Form<HoldForm>) -> Response {
    if !trusted(&o, &f.proof).await {
        return untrusted();
    }
    let reason = f
        .reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "its owner gave no reason".into());
    let since = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let held = match o.hold.hold(&reason, &since) {
        Ok(h) => h,
        Err(e) => {
            return failed(&format!(
                "the hold could not be written down, so it is not in force: {e}"
            ))
        }
    };
    tracing::warn!(reason = %held.reason, now = f.now, "the robot is held by its owner");
    let mut stopped = vec![];
    if let (true, Some(guard)) = (f.now, &o.guard) {
        let why = format!("a person took this robot over: {}", held.reason);
        for (run, took) in guard.stop_all(&why).await {
            stopped.push(match took {
                Ok(took) => json!({ "run": run, "cancelled": took }),
                Err(e) => json!({ "run": run, "cancelled": false, "error": e.to_string() }),
            });
        }
    }
    Json(json!({ "held": { "reason": held.reason, "since": held.since }, "stopped": stopped })).into_response()
}

async fn release(State(o): State<Owner>, Form(f): Form<ReleaseForm>) -> Response {
    if !trusted(&o, &f.proof).await {
        return untrusted();
    }
    match o.hold.release() {
        Ok(released) => {
            tracing::info!(released, "the robot is released by its owner");
            Json(json!({ "released": released })).into_response()
        }
        Err(e) => failed(&format!("the hold could not be removed, so it stays: {e}")),
    }
}

/// Whether `proof` is the machine's admin token.
async fn trusted(o: &Owner, proof: &str) -> bool {
    o.approver.attest(proof.trim()).await.is_ok()
}

fn untrusted() -> Response {
    let message = "this is not the machine's admin token: run best-robot hold or release on the machine the door runs on, with the same BEST_ROBOT_DATA";
    let body = json!({ "error": { "code": "UNAUTHORIZED", "message": message } });
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

fn failed(message: &str) -> Response {
    let body = json!({ "error": { "code": "HOLD_NOT_WRITTEN", "message": message } });
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}
