//! The door over a robot: best-peer's BEST surface, with this door's catalogue, its own approver and its data
//! folder, the robot's skills behind the owner's guard, and beside it the owner's routes and a camera's images, served
//! on a listener the caller binds.

use std::net::SocketAddr;
use std::sync::Arc;

use best_peer_door::{Auth, Catalogue, Door, EventLog, LocalApprover};
use robot::Skills;
use serde_json::json;

use crate::approver::OwnerApprover;
use crate::guard::{envelope::Envelope, hold::Hold, Guard};
use crate::owner::{self, Owner};
use crate::settings::Settings;
use crate::surface::{self, frames::Frames, Parts};

/// The service key in the manifest, and the `source` of every event.
const SERVICE: &str = "io.best.agents";

/// The door over the robot's `parts` and the owner's routes: its skills, when it has any, behind a guard that holds
/// them to `envelope` and to the owner's hold, each skill a scope a credential may be approved for.
pub fn build(
    settings: &Settings,
    mut parts: Parts,
    envelope: Option<Envelope>,
) -> Result<(Door, axum::Router), String> {
    let kept = |e: std::io::Error| format!("cannot keep the door's data in {}: {e}", settings.data.display());
    let hold = Arc::new(Hold::open(&settings.data).map_err(kept)?);
    let guard = match parts.skills.take() {
        Some(skills) => Some(Arc::new(Guard::new(
            skills,
            envelope,
            parts.topics.clone(),
            hold.clone(),
        )?)),
        None if envelope.is_some() => {
            return Err("BEST_ROBOT_ENVELOPE bounds a robot's skills, and this robot offers none".into())
        }
        None => None,
    };
    parts.skills = guard.clone().map(|g| g as Arc<dyn Skills>);
    let scopes: Vec<String> = parts
        .skills
        .iter()
        .flat_map(|s| s.skills().iter().map(|spec| spec.name.clone()))
        .collect();
    let description = surface::describe(&parts);
    let frames = Frames::new(&settings.endpoint);
    let catalogue = surface::catalogue(parts, &frames)?;
    let door = open(settings, &description, &scopes, catalogue).map_err(kept)?;
    let approver = Arc::new(LocalApprover::open(&settings.data).map_err(kept)?);
    Ok((
        door,
        owner::routes(Owner { hold, guard, approver }).merge(frames.routes()),
    ))
}

/// The door over `catalogue`, described as `description`, its credentials approved for any of `scopes`, with what a
/// run before a restart left unfinished closed as failed.
fn open(settings: &Settings, description: &str, scopes: &[String], catalogue: Catalogue) -> std::io::Result<Door> {
    let approver = OwnerApprover::open(&settings.data)?;
    let scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
    let auth = Auth::open(&settings.data, &settings.endpoint, Box::new(approver))?.with_scopes(&scopes);
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

/// Serves the door on `listener`, with the owner's routes beside it, until `stop` resolves.
pub async fn serve(
    listener: tokio::net::TcpListener,
    door: Door,
    owner: axum::Router,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let app = best_peer_door::router(Arc::new(door)).merge(owner);
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
            envelope: None,
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
            target: None,
        }]));
        let parts = crate::surface::Parts {
            state: None,
            skills: Some(skills.clone()),
            topics: None,
        };
        let frames = Frames::new(&settings.endpoint);
        let door = open(
            &settings,
            "test",
            &[],
            crate::surface::catalogue(parts, &frames).unwrap(),
        )
        .unwrap();
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
