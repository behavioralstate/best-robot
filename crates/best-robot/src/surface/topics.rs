//! Each topic as a query, `get-<topic>`: its latest message and when it arrived, both null until one has.

use std::sync::Arc;

use best_peer_door::{Catalogue, QueryDef};
use robot::{Snapshot, TopicSpec, Topics};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub fn query_name(topic: &str) -> String {
    format!("get-{topic}")
}

pub fn add(mut c: Catalogue, topics: Arc<dyn Topics>) -> Catalogue {
    for spec in topics.topics() {
        c = c.query(query(spec, topics.clone()));
    }
    c
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
    async fn nothing_heard_yet_is_null_and_meets_the_schema() {
        let def = query(&spec(), Arc::new(FakeTopics::new(vec![spec()])));
        let answer = (def.handler)(who(), HashMap::new()).await.unwrap();
        assert_eq!(answer, json!({ "message": null, "received_at": null }));
        assert!(jsonschema::is_valid(&def.response, &answer), "{answer}");
    }
}
