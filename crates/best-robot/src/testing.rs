//! A door over the parts a test hands it, in this process: an agent signed in through the device flow, commands sent
//! and events read through the same router the binary serves.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use best_peer_door::auth::{DeviceRequest, Grant, TokenRequest};
use best_peer_door::{Auth, Door, EventLog, FakeApprover};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::surface::{self, Parts};

const ENDPOINT: &str = "http://door.test";

pub struct Signed {
    pub router: axum::Router,
    key: String,
    /// Another agent's key on the same door.
    other: String,
}

impl Signed {
    pub async fn over(parts: Parts) -> Self {
        let auth = Auth::ephemeral(ENDPOINT, Box::new(FakeApprover { proof: "p".into() }));
        let key = sign_in(&auth, "test").await;
        let other = sign_in(&auth, "other").await;
        let catalogue = surface::catalogue(parts).unwrap();
        let door = Door::new(
            ENDPOINT,
            "io.best.agents",
            "test",
            catalogue,
            Arc::new(EventLog::ephemeral()),
            Arc::new(auth),
        );
        Self {
            router: best_peer_door::router(Arc::new(door)),
            key,
            other,
        }
    }

    /// The same door, as the other agent.
    pub fn other(&self) -> Self {
        Self {
            router: self.router.clone(),
            key: self.other.clone(),
            other: self.key.clone(),
        }
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// Sends the command `ty` (schema `schema`) with `data` as `id`, and says what the door answered.
    pub async fn send(&self, schema: &str, ty: &str, id: &str, data: Value) -> (StatusCode, Value) {
        let envelope = json!({ "specversion": "1.0", "id": id, "source": "test", "type": ty,
            "datacontenttype": "application/json", "dataschema": format!("{ENDPOINT}/commands/{schema}/1.0"),
            "time": "2026-11-05T00:00:00Z", "data": data });
        let req = Request::post("/commands")
            .header("X-Api-Key", &self.key)
            .header("content-type", "application/json")
            .body(Body::from(envelope.to_string()))
            .unwrap();
        self.call(req).await
    }

    pub async fn get(&self, path: &str) -> (StatusCode, Value) {
        let req = Request::get(path)
            .header("X-Api-Key", &self.key)
            .body(Body::empty())
            .unwrap();
        self.call(req).await
    }

    /// The types of the events under `correlation`, once one of `last` is among them (five seconds at most).
    pub async fn events_until(&self, correlation: &str, last: &[&str]) -> Vec<Value> {
        for _ in 0..100 {
            let (_, body) = self.get(&format!("/events?correlationId={correlation}")).await;
            let events = body["events"].as_array().cloned().unwrap_or_default();
            if events.iter().any(|e| last.iter().any(|l| e["type"] == *l)) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("none of {last:?} under {correlation} within five seconds");
    }
}

/// An agent `client` signed in through the device flow, its key.
async fn sign_in(auth: &Auth, client: &str) -> String {
    let device = auth
        .start_device(DeviceRequest {
            client_id: Some(client.into()),
            connection: Some(format!("{client}-robot")),
            ..Default::default()
        })
        .unwrap();
    auth.approve(&device.user_code, "p", Grant::default(), None)
        .await
        .unwrap();
    let token = auth
        .token(TokenRequest {
            grant_type: Some("urn:ietf:params:oauth:grant-type:device_code".into()),
            device_code: Some(device.device_code),
            client_id: Some(client.into()),
        })
        .unwrap();
    token["access_token"].as_str().unwrap().into()
}
