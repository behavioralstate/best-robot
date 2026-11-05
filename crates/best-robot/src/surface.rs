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
                let state = robot.state().await.map_err(|e| {
                    BestError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "ROBOT_UNREACHABLE",
                        format!("{e}: read it again in a moment, and if it stays so, tell the robot's owner that the door cannot reach the robot"),
                    )
                })?;
                within_schema(&state).map_err(|why| {
                    BestError::new(
                        StatusCode::BAD_GATEWAY,
                        "ROBOT_STATE_INVALID",
                        format!("the robot reported {why}, so its state is not served: read it again in a moment, and if it stays so, tell the robot's owner that its localisation or its battery reading is at fault"),
                    )
                })?;
                Ok(state_json(&state))
            })
        }),
    }
}

/// What the schema promises and an `f64` does not: every number finite, the charge within 0 to 100. A robot that
/// breaks it is refused, never served as `null` or as a value the schema forbids.
fn within_schema(s: &State) -> Result<(), String> {
    let pose = s
        .pose
        .iter()
        .flat_map(|p| [("pose.x", p.x), ("pose.y", p.y), ("pose.yaw", p.yaw)]);
    let battery = s.battery_percent.map(|b| ("battery_percent", b));
    for (field, v) in pose.chain(battery) {
        if !v.is_finite() {
            return Err(format!("{field} = {v}, not a finite number"));
        }
    }
    match s.battery_percent {
        Some(b) if !(0.0..=100.0).contains(&b) => Err(format!("battery_percent = {b}, outside 0 to 100")),
        _ => Ok(()),
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

    fn reporting(x: f64, yaw: f64, battery_percent: Option<f64>) -> Arc<dyn Robot> {
        Arc::new(Fake::new(State {
            name: "r1".into(),
            status: Status::Moving,
            pose: Some(robot::Pose {
                frame: "map".into(),
                x,
                y: 0.0,
                yaw,
            }),
            battery_percent,
        }))
    }

    #[tokio::test]
    async fn a_number_that_is_not_finite_is_refused_by_name_with_the_fix_never_served_as_null() {
        for (robot, field) in [
            (reporting(f64::NAN, 0.0, Some(50.0)), "pose.x"),
            (reporting(0.0, f64::INFINITY, Some(50.0)), "pose.yaw"),
            (reporting(0.0, 0.0, Some(f64::NAN)), "battery_percent"),
        ] {
            let e = ask(robot).await.unwrap_err();
            assert_eq!(e.status, StatusCode::BAD_GATEWAY);
            assert_eq!(e.code, "ROBOT_STATE_INVALID");
            assert!(e.message.contains(&format!("{field} = ")), "{}", e.message);
            assert!(e.message.contains("not a finite number"), "{}", e.message);
            assert!(e.message.contains("read it again"), "{}", e.message);
        }
    }

    #[tokio::test]
    async fn a_charge_outside_0_to_100_is_refused_by_name() {
        let e = ask(reporting(1.0, 0.0, Some(150.0))).await.unwrap_err();
        assert_eq!(e.code, "ROBOT_STATE_INVALID");
        assert!(
            e.message.contains("battery_percent = 150, outside 0 to 100"),
            "{}",
            e.message
        );
        let e = ask(reporting(1.0, 0.0, Some(-5.0))).await.unwrap_err();
        assert!(
            e.message.contains("battery_percent = -5, outside 0 to 100"),
            "{}",
            e.message
        );
        assert!(ask(reporting(1.0, 0.0, Some(100.0))).await.is_ok());
        assert!(ask(reporting(1.0, 0.0, Some(0.0))).await.is_ok());
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
        assert!(e.message.contains("rosbridge closed the socket"), "{}", e.message);
        assert!(e.message.contains("read it again"), "{}", e.message);
    }
}
