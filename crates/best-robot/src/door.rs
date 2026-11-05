//! The door over a robot: best-peer's BEST surface, with this door's catalogue, its own approver and its data
//! folder, served on a listener the caller binds.

use std::net::SocketAddr;
use std::sync::Arc;

use best_peer_door::{Auth, Catalogue, Door, EventLog};
use serde_json::json;

use crate::approver::OwnerApprover;
use crate::settings::Settings;

/// The service key in the manifest, and the `source` of every event.
const SERVICE: &str = "io.best.agents";

/// The door over `catalogue`, described as `description`, with what a run before a restart left unfinished closed as
/// failed.
pub fn open(settings: &Settings, description: &str, catalogue: Catalogue) -> std::io::Result<Door> {
    let approver = OwnerApprover::open(&settings.data)?;
    let auth = Auth::open(&settings.data, &settings.endpoint, Box::new(approver))?;
    let log = EventLog::open(&settings.data)?;
    let door = Door::new(
        &settings.endpoint,
        SERVICE,
        description,
        catalogue,
        Arc::new(log),
        Arc::new(auth),
    );
    settle(&door);
    Ok(door)
}

/// Every command accepted before a restart and never finished ends in `CommandFailed`, never sent to the robot
/// again: a goal sent twice moves a robot twice.
fn settle(door: &Door) {
    for owed in door.log.unfinished() {
        let e = owed.envelope;
        tracing::warn!(command = %e.r#type, id = %e.id, "closing a command the door stopped during");
        door.emitter(&e.effective_correlation())
            .with_key(&format!("{}:{}", owed.sender, e.id))
            .emit("CommandFailed", json!({ "type": e.r#type, "reason": "the door stopped while this ran, so how it ended is not known and it is not sent again: read what the robot reports, then send it again under a new command id if it is still wanted" }));
    }
}

/// Serves the door on `listener` until `stop` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    door: Door,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = best_peer_door::router(Arc::new(door));
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(stop)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RobotKind;
    use best_peer_door::log::Filter;
    use best_peer_door::Envelope;

    #[test]
    fn a_command_the_door_stopped_during_ends_failed_and_is_not_run_again() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            listen: "127.0.0.1:7780".parse().unwrap(),
            endpoint: "http://127.0.0.1:7780".into(),
            data: dir.path().to_path_buf(),
            robot: RobotKind::Fake,
        };
        {
            let log = EventLog::open(dir.path()).unwrap();
            let command: Envelope = serde_json::from_value(json!({ "specversion": "1.0", "id": "c1", "source": "t",
                "type": "Fibonacci", "datacontenttype": "application/json", "time": "2026-11-05T00:00:00Z",
                "data": { "order": 4 } }))
            .unwrap();
            log.accept_command("sender", "c1", "hash", "c1", &command);
        }
        let skills = Arc::new(robot::FakeSkills::new(vec![robot::SkillSpec {
            name: "fibonacci".into(),
            description: "Counts.".into(),
            goal: json!({ "type": "object" }),
            feedback: json!({ "type": "object" }),
            result: json!({ "type": "object" }),
        }]));
        let parts = crate::surface::Parts {
            state: None,
            skills: Some(skills.clone()),
            topics: None,
        };
        let door = open(&settings, "test", crate::surface::catalogue(parts).unwrap()).unwrap();
        assert!(door.log.unfinished().is_empty());
        assert!(skills.goals().is_empty(), "the goal was not sent again");
        let filter = Filter {
            correlation_id: Some("c1".into()),
            ..Default::default()
        };
        let (events, _) = door.log.list_where(&filter, |_| true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].r#type, "CommandFailed");
        assert!(events[0].data["reason"].as_str().unwrap().contains("not sent again"));
    }
}
