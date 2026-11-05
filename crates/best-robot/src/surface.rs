//! What the door offers, built over a [`Robot`] it is handed: the catalogue, and the robot's state as the agent reads
//! it. The wire shape lives here, next to its schema, so the robot abstraction knows nothing of JSON.

use std::sync::Arc;

use axum::http::StatusCode;
use best_peer_door::{BestError, Catalogue, QueryDef};
use robot::{Robot, State};
use serde_json::{json, Value};

pub fn catalogue(robot: Arc<dyn Robot>) -> Catalogue {
    Catalogue::new().query(robot_state(robot))
}

fn robot_state(robot: Arc<dyn Robot>) -> QueryDef {
    QueryDef {
        schema: "get-robot-state".into(),
        version: "1.0".into(),
        description: "The robot right now: its name, whether it is idle, moving or in fault, its pose in a named frame (metres and radians) when it knows it, and its charge when it reports one.".into(),
        parameters: None,
        response: state_schema(),
        handler: Arc::new(move |_who, _params| {
            let robot = robot.clone();
            Box::pin(async move {
                robot.state().await.map(|s| state_json(&s)).map_err(|e| {
                    BestError::new(StatusCode::SERVICE_UNAVAILABLE, "ROBOT_UNREACHABLE", e.to_string())
                })
            })
        }),
    }
}

fn state_json(s: &State) -> Value {
    json!({
        "name": s.name,
        "status": s.status.as_str(),
        "pose": s.pose.as_ref().map(|p| json!({ "frame": p.frame, "x": p.x, "y": p.y, "yaw": p.yaw })),
        "battery_percent": s.battery_percent,
    })
}

fn state_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "status", "pose", "battery_percent"],
        "properties": {
            "name": { "type": "string" },
            "status": { "type": "string", "enum": ["idle", "moving", "fault"] },
            "pose": {
                "type": ["object", "null"],
                "description": "Where the robot is, null when it does not know",
                "required": ["frame", "x", "y", "yaw"],
                "properties": {
                    "frame": { "type": "string", "description": "The frame the pose is in, such as map" },
                    "x": { "type": "number", "description": "Metres" },
                    "y": { "type": "number", "description": "Metres" },
                    "yaw": { "type": "number", "description": "Radians" }
                }
            },
            "battery_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use best_peer_door::auth::{Credential, Grant};
    use robot::{Fake, Status, Unreachable};
    use std::collections::HashMap;

    fn anyone() -> Credential {
        Credential {
            name: "test".into(),
            hash: "h".into(),
            client_id: "test".into(),
            agent_label: None,
            issued_at: "2026-11-05T00:00:00Z".into(),
            expires_at: None,
            revoked_at: None,
            work_ended: false,
            grant: Grant::default(),
        }
    }

    async fn ask(robot: Arc<dyn Robot>) -> Result<Value, BestError> {
        let def = robot_state(robot);
        (def.handler)(anyone(), HashMap::new()).await
    }

    #[tokio::test]
    async fn the_state_is_what_the_robot_says_and_meets_its_schema() {
        let answer = ask(Arc::new(Fake::default())).await.unwrap();
        assert_eq!(answer["name"], "fake");
        assert_eq!(answer["status"], "idle");
        assert_eq!(answer["pose"]["frame"], "map");
        assert!(jsonschema::is_valid(&state_schema(), &answer), "{answer}");
    }

    #[tokio::test]
    async fn what_the_robot_does_not_know_is_null_and_still_meets_the_schema() {
        let robot = Fake::new(State {
            name: "r1".into(),
            status: Status::Fault,
            pose: None,
            battery_percent: None,
        });
        let answer = ask(Arc::new(robot)).await.unwrap();
        assert_eq!(answer["pose"], Value::Null);
        assert_eq!(answer["battery_percent"], Value::Null);
        assert!(jsonschema::is_valid(&state_schema(), &answer), "{answer}");
    }

    struct Silent;

    #[async_trait]
    impl Robot for Silent {
        async fn state(&self) -> Result<State, Unreachable> {
            Err(Unreachable("rosbridge closed the socket".into()))
        }
    }

    #[tokio::test]
    async fn a_robot_that_does_not_answer_is_a_503_that_says_why() {
        let e = ask(Arc::new(Silent)).await.unwrap_err();
        assert_eq!(e.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(e.code, "ROBOT_UNREACHABLE");
        assert!(e.message.contains("rosbridge closed the socket"));
    }
}
