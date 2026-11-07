//! Each topic as a query, `get-<topic>`: its latest message and when it arrived, both null until one has; for a topic
//! of a camera's frames, the latest image as an address and a digest, served off BEST ([`super::frames`]).

use std::sync::Arc;

use best_peer_door::{Catalogue, QueryDef};
use robot::{Snapshot, TopicSpec, Topics};

use super::frames::{Frames, KEPT};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub fn query_name(topic: &str) -> String {
    format!("get-{topic}")
}

pub fn add(mut c: Catalogue, topics: Arc<dyn Topics>, frames: &Arc<Frames>) -> Catalogue {
    for spec in topics.topics() {
        c = c.query(match spec.frames {
            true => frame_query(spec, topics.clone(), frames.clone()),
            false => query(spec, topics.clone()),
        });
    }
    c
}

fn frame_query(spec: &TopicSpec, topics: Arc<dyn Topics>, frames: Arc<Frames>) -> QueryDef {
    let name = spec.name.clone();
    QueryDef {
        schema: query_name(&spec.name),
        version: "1.0".into(),
        description: format!(
            "{} The latest image and when it arrived, both null until one has. The image is named by its address and the SHA-256 of its bytes, never sent: GET the address for the bytes, or hand the address and the digest to whatever looks at the image. The door keeps the latest {KEPT} images it named, so read it again for a newer one or once an address answers 404.",
            spec.description
        ),
        parameters: None,
        response: json!({
            "type": "object",
            "required": ["frame", "received_at"],
            "properties": {
                "frame": { "anyOf": [{
                    "type": "object",
                    "required": ["url", "sha256", "format", "bytes"],
                    "properties": {
                        "url": { "type": "string", "description": "Where the image's bytes are served, off BEST" },
                        "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$", "description": "The SHA-256 of the image's bytes, lowercase hex" },
                        "format": { "type": "string", "description": "The format the camera says the image is in, such as jpeg or png" },
                        "bytes": { "type": "integer", "description": "The image's size" }
                    }
                }, { "type": "null" }] },
                "received_at": { "type": ["string", "null"], "format": "date-time" }
            }
        }),
        handler: Arc::new(move |_who, _params| {
            let answer = match topics.frame(&name) {
                Some(f) => {
                    let named = frames.keep(&f);
                    json!({
                        "frame": { "url": named.url, "sha256": named.sha256, "format": f.format, "bytes": f.bytes.len() },
                        "received_at": OffsetDateTime::from(f.received_at).format(&Rfc3339).ok(),
                    })
                }
                None => json!({ "frame": null, "received_at": null }),
            };
            Box::pin(async move { Ok(answer) })
        }),
    }
}

fn query(spec: &TopicSpec, topics: Arc<dyn Topics>) -> QueryDef {
    let name = spec.name.clone();
    QueryDef {
        schema: query_name(&spec.name),
        version: "1.0".into(),
        description: format!(
            "{} The latest message and when it arrived; both null until one has.",
            spec.description
        ),
        parameters: None,
        response: json!({
            "type": "object",
            "required": ["message", "received_at"],
            "properties": {
                "message": { "anyOf": [spec.message, { "type": "null" }] },
                "received_at": { "type": ["string", "null"], "format": "date-time" }
            }
        }),
        handler: Arc::new(move |_who, _params| {
            let answer = answer(topics.latest(&name));
            Box::pin(async move { Ok(answer) })
        }),
    }
}

fn answer(latest: Option<Snapshot>) -> Value {
    match latest {
        Some(s) => json!({
            "message": s.message,
            "received_at": OffsetDateTime::from(s.received_at).format(&Rfc3339).ok(),
        }),
        None => json!({ "message": null, "received_at": null }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use best_peer_door::auth::{Credential, Grant};
    use robot::FakeTopics;
    use std::collections::HashMap;

    fn spec() -> TopicSpec {
        TopicSpec {
            name: "battery".into(),
            description: "The battery.".into(),
            frames: false,
            message: json!({ "type": "object", "properties": { "percentage": { "type": "number" } }, "additionalProperties": false }),
        }
    }

    fn who() -> Credential {
        Credential {
            name: "t".into(),
            hash: "h".into(),
            client_id: "t".into(),
            agent_label: None,
            issued_at: "2026-11-05T00:00:00Z".into(),
            expires_at: None,
            revoked_at: None,
            work_ended: false,
            grant: Grant::default(),
        }
    }

    #[tokio::test]
    async fn the_latest_message_is_served_with_when_it_arrived_and_meets_the_schema() {
        let topics = Arc::new(FakeTopics::new(vec![spec()]).with("battery", json!({ "percentage": 0.87 })));
        let def = query(&spec(), topics);
        let answer = (def.handler)(who(), HashMap::new()).await.unwrap();
        assert_eq!(answer["message"]["percentage"], 0.87);
        assert_eq!(answer["received_at"], "1970-01-01T00:00:00Z");
        assert!(jsonschema::is_valid(&def.response, &answer), "{answer}");
    }

    #[tokio::test]
    async fn a_camera_s_latest_image_is_named_by_address_and_digest_and_served_there_never_sent() {
        let camera = TopicSpec {
            name: "camera".into(),
            description: "The camera.".into(),
            message: json!({ "type": "object" }),
            frames: true,
        };
        let topics = Arc::new(FakeTopics::new(vec![camera.clone()]));
        let frames = Frames::new("http://127.0.0.1:7780");
        let def = frame_query(&camera, topics.clone(), frames.clone());
        let none = (def.handler)(who(), HashMap::new()).await.unwrap();
        assert_eq!(none, json!({ "frame": null, "received_at": null }));
        assert!(jsonschema::is_valid(&def.response, &none), "{none}");
        topics.frame_now("camera", "jpeg", &[0xff, 0xd8, 0xff, 1]);
        let answer = (def.handler)(who(), HashMap::new()).await.unwrap();
        assert!(jsonschema::is_valid(&def.response, &answer), "{answer}");
        let frame = &answer["frame"];
        assert_eq!(frame["format"], "jpeg");
        assert_eq!(frame["bytes"], 4);
        let sha = frame["sha256"].as_str().unwrap();
        assert_eq!(frame["url"], format!("http://127.0.0.1:7780/frames/{sha}"));
        assert!(!answer.to_string().contains("/9j/"), "no bytes on BEST: {answer}");
        let kept = frames.keep(&topics.frame("camera").unwrap());
        assert_eq!(kept.sha256, sha);
    }

    #[tokio::test]
    async fn nothing_heard_yet_is_null_and_meets_the_schema() {
        let def = query(&spec(), Arc::new(FakeTopics::new(vec![spec()])));
        let answer = (def.handler)(who(), HashMap::new()).await.unwrap();
        assert_eq!(answer, json!({ "message": null, "received_at": null }));
        assert!(jsonschema::is_valid(&def.response, &answer), "{answer}");
    }
}
