//! Each skill as a command whose data is its goal, and its run as events: `<Skill>Progressed` with its feedback, at
//! most one a second, then one of `<Skill>Succeeded`, `<Skill>Aborted`, `<Skill>Canceled` (with the result) or `<Skill>Refused` (with
//! why the robot never started it). `cancel-skill` ends a run its own sender started.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use best_peer_door::{Catalogue, CommandDef, Emitter, Envelope, EventDef};
use robot::{Outcome, SkillSpec, Skills};
use serde_json::{json, Value};

use super::pascal;

pub const CANCEL: &str = "cancel-skill";
/// The types `cancel-skill` and its events answer to.
pub const CANCEL_TYPES: [&str; 3] = ["CancelSkill", "SkillCancelRequested", "SkillCancelRefused"];
const VERSION: &str = "1.0";
/// The shortest time between two `<Skill>Progressed` of a run: a robot may send feedback every control tick, and an
/// event is a fact kept for good, not a sample.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);
/// The suffixes of a skill's events, in the order a run can produce them.
const SUFFIXES: [&str; 5] = ["Progressed", "Succeeded", "Aborted", "Canceled", "Refused"];

/// The types the skill `name` answers to: its command's, then its events'.
pub fn types(name: &str) -> Vec<String> {
    let ty = pascal(name);
    std::iter::once(ty.clone())
        .chain(SUFFIXES.iter().map(|s| format!("{ty}{s}")))
        .collect()
}

pub fn add(mut c: Catalogue, skills: Arc<dyn Skills>) -> Catalogue {
    for spec in skills.skills() {
        c = c.command(command(spec, skills.clone()));
        for e in events(spec) {
            c = c.event(e);
        }
    }
    c = c.command(cancel(skills));
    c.event(event(
        "skill-cancel-requested",
        "SkillCancelRequested",
        "The robot was asked to end the run; the run's own event says how it ended.",
        json!({ "command": { "type": "string" } }),
    ))
    .event(event(
        "skill-cancel-refused",
        "SkillCancelRefused",
        "No run of yours started by that command is in flight on this door: it ended, never started, or the door lost track of it.",
        json!({ "command": { "type": "string" }, "reason": { "type": "string" } }),
    ))
}

/// The outcome events of a skill, by suffix: what each carries.
fn outcomes(spec: &SkillSpec) -> [(&'static str, &'static str, Value); 5] {
    [
        (
            "Progressed",
            "Feedback while the skill runs, at most one a second: the latest the robot sent, and the last one before the run ends.",
            json!({ "feedback": spec.feedback }),
        ),
        (
            "Succeeded",
            "The skill reached its goal; the result is the robot's.",
            json!({ "result": spec.result }),
        ),
        (
            "Aborted",
            "The robot gave up on the goal; the result is what it had.",
            json!({ "result": spec.result }),
        ),
        (
            "Canceled",
            "The run ended on a cancel; the result is what it had.",
            json!({ "result": spec.result }),
        ),
        (
            "Refused",
            "The robot never started the goal; the reason says why.",
            json!({ "reason": { "type": "string" } }),
        ),
    ]
}

fn events(spec: &SkillSpec) -> Vec<EventDef> {
    let ty = pascal(&spec.name);
    outcomes(spec)
        .into_iter()
        .map(|(suffix, description, properties)| {
            let schema = format!("{}-{}", spec.name, suffix.to_ascii_lowercase());
            event(&schema, &format!("{ty}{suffix}"), description, properties)
        })
        .collect()
}

fn event(schema: &str, event_type: &str, description: &str, properties: Value) -> EventDef {
    let required: Vec<&String> = properties.as_object().map(|o| o.keys().collect()).unwrap_or_default();
    EventDef {
        schema: schema.into(),
        version: VERSION.into(),
        event_type: event_type.into(),
        description: description.into(),
        data_schema: json!({ "type": "object", "required": required, "properties": properties }),
    }
}

fn command(spec: &SkillSpec, skills: Arc<dyn Skills>) -> CommandDef {
    let ty = pascal(&spec.name);
    let named = |suffix: &str| format!("{ty}{suffix}");
    let name = spec.name.clone();
    CommandDef {
        schema: spec.name.clone(),
        version: VERSION.into(),
        command_type: ty.clone(),
        description: format!(
            "{} Its outcome is an event; cancel-skill naming this command's id ends it early.",
            spec.description
        ),
        data_schema: spec.goal.clone(),
        produces: outcomes(spec).iter().map(|(suffix, ..)| named(suffix)).collect(),
        completes: SUFFIXES[1..].iter().map(|s| named(s)).collect(),
        impact: None,
        admit: None,
        handler: Arc::new(move |envelope: Envelope, emitter: Emitter| {
            let (skills, name, ty) = (skills.clone(), name.clone(), ty.clone());
            Box::pin(async move {
                let run = emitter.command_key().to_string();
                let paced = Arc::new(Paced::new(emitter.clone(), format!("{ty}Progressed")));
                let progressed = {
                    let paced = paced.clone();
                    Box::new(move |feedback: Value| paced.feedback(feedback))
                };
                let end = skills.run(&run, &name, envelope.data, progressed).await;
                paced.flush();
                match end {
                    Ok(Outcome::Succeeded(r)) => emitter.emit(&format!("{ty}Succeeded"), json!({ "result": r })),
                    Ok(Outcome::Aborted(r)) => emitter.emit(&format!("{ty}Aborted"), json!({ "result": r })),
                    Ok(Outcome::Canceled(r)) => emitter.emit(&format!("{ty}Canceled"), json!({ "result": r })),
                    Ok(Outcome::Refused(why)) => emitter.emit(&format!("{ty}Refused"), json!({ "reason": why })),
                    Err(e) => emitter.emit(
                        "CommandFailed",
                        json!({ "type": ty, "reason": format!("{e}. It may have started the goal before that and still be running it: read what the robot reports before sending the goal again, under a new command id") }),
                    ),
                };
            })
        }),
    }
}

/// A run's feedback as events, at most one each [`PROGRESS_EVERY`]: one that comes sooner waits, replaced by any newer,
/// and the one still waiting when the run ends is told before its outcome.
struct Paced {
    emitter: Emitter,
    event_type: String,
    state: Mutex<(Option<Instant>, Option<Value>)>,
}

impl Paced {
    fn new(emitter: Emitter, event_type: String) -> Self {
        Self {
            emitter,
            event_type,
            state: Mutex::new((None, None)),
        }
    }

    fn feedback(&self, feedback: Value) {
        let mut state = self.state.lock().unwrap();
        let (told, waiting) = &mut *state;
        if told.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) {
            *waiting = Some(feedback);
        } else {
            *told = Some(Instant::now());
            *waiting = None;
            self.emitter.emit(&self.event_type, json!({ "feedback": feedback }));
        }
    }

    fn flush(&self) {
        if let Some(feedback) = self.state.lock().unwrap().1.take() {
            self.emitter.emit(&self.event_type, json!({ "feedback": feedback }));
        }
    }
}

fn cancel(skills: Arc<dyn Skills>) -> CommandDef {
    CommandDef {
        schema: CANCEL.into(),
        version: VERSION.into(),
        command_type: "CancelSkill".into(),
        description: "Ends a skill's run you started, named by the id of the command that started it. The run then ends with its own event, Canceled or whatever came first.".into(),
        data_schema: json!({ "type": "object", "required": ["command"], "additionalProperties": false,
            "properties": { "command": { "type": "string", "minLength": 1, "description": "The id of the command that started the run" } } }),
        produces: vec!["SkillCancelRequested".into(), "SkillCancelRefused".into()],
        completes: vec!["SkillCancelRequested".into(), "SkillCancelRefused".into()],
        impact: None,
        admit: None,
        handler: Arc::new(move |envelope: Envelope, emitter: Emitter| {
            let skills = skills.clone();
            Box::pin(async move {
                let command = envelope.data["command"].as_str().unwrap_or_default().to_string();
                // A run is filed under its command's key, `sender:id`: the same sender can name only its own runs.
                let sender = emitter.command_key().split_once(':').map(|(s, _)| s).unwrap_or_default();
                match skills.cancel(&format!("{sender}:{command}")).await {
                    Ok(true) => emitter.emit("SkillCancelRequested", json!({ "command": command })),
                    Ok(false) => emitter.emit(
                        "SkillCancelRefused",
                        json!({ "command": command, "reason": "no run of yours started by that command is in flight on this door: it ended, never started, or the door lost track of it. Read that command's events: if they end in CommandFailed, the robot may still be running the goal, and only the robot itself can stop it now" }),
                    ),
                    Err(e) => emitter.emit(
                        "CommandFailed",
                        json!({ "type": "CancelSkill", "reason": format!("{e}. The run was not cancelled: send cancel-skill again in a moment, under a new command id") }),
                    ),
                };
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Parts;
    use crate::testing::Signed;
    use axum::http::StatusCode;
    use robot::FakeSkills;

    fn spec(name: &str) -> SkillSpec {
        SkillSpec {
            name: name.into(),
            description: "Counts.".into(),
            goal: json!({ "type": "object", "properties": { "to": { "type": "integer" } }, "additionalProperties": false }),
            feedback: json!({ "type": "object" }),
            result: json!({ "type": "object" }),
        }
    }

    async fn door(skills: FakeSkills) -> (Signed, Arc<FakeSkills>) {
        let skills = Arc::new(skills);
        let parts = Parts {
            state: None,
            skills: Some(skills.clone()),
            topics: None,
        };
        (Signed::over(parts).await, skills)
    }

    fn types(events: &[Value]) -> Vec<&str> {
        events.iter().map(|e| e["type"].as_str().unwrap()).collect()
    }

    #[tokio::test]
    async fn a_skill_is_a_command_whose_run_is_told_as_events() {
        let fake = FakeSkills::new(vec![spec("count")]).ends(
            "count",
            vec![json!({ "at": 1 })],
            Outcome::Succeeded(json!({ "counted": 2 })),
        );
        let (door, skills) = door(fake).await;
        let (status, _) = door.send("count", "Count", "c1", json!({ "to": 2 })).await;
        assert_eq!(status, StatusCode::CREATED);
        let events = door.events_until("c1", &["CountSucceeded"]).await;
        assert_eq!(types(&events), ["CountProgressed", "CountSucceeded"]);
        assert_eq!(events[0]["data"], json!({ "feedback": { "at": 1 } }));
        assert_eq!(events[1]["data"], json!({ "result": { "counted": 2 } }));
        assert_eq!(skills.goals(), [("count".to_string(), json!({ "to": 2 }))]);
    }

    #[tokio::test]
    async fn feedback_faster_than_a_second_is_told_as_its_first_and_its_last() {
        let feedback = (1..=50).map(|at| json!({ "at": at })).collect();
        let fake = FakeSkills::new(vec![spec("count")]).ends("count", feedback, Outcome::Succeeded(json!({})));
        let (door, _) = door(fake).await;
        door.send("count", "Count", "c1", json!({})).await;
        let events = door.events_until("c1", &["CountSucceeded"]).await;
        assert_eq!(types(&events), ["CountProgressed", "CountProgressed", "CountSucceeded"]);
        assert_eq!(events[0]["data"]["feedback"]["at"], 1);
        assert_eq!(events[1]["data"]["feedback"]["at"], 50);
    }

    #[tokio::test]
    async fn a_goal_with_a_field_the_type_lacks_is_refused_at_the_door_naming_it() {
        let (door, skills) = door(FakeSkills::new(vec![spec("count")])).await;
        let (status, body) = door.send("count", "Count", "c1", json!({ "too": 2 })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.to_string().contains("too"), "{body}");
        assert!(skills.goals().is_empty(), "nothing reached the robot");
    }

    #[tokio::test]
    async fn each_way_a_run_ends_is_its_own_event() {
        for (outcome, event) in [
            (Outcome::Aborted(json!({})), "CountAborted"),
            (Outcome::Canceled(json!({})), "CountCanceled"),
            (Outcome::Refused("Action goal was rejected".into()), "CountRefused"),
        ] {
            let (door, _) = door(FakeSkills::new(vec![spec("count")]).ends("count", vec![], outcome)).await;
            door.send("count", "Count", "c1", json!({})).await;
            assert_eq!(types(&door.events_until("c1", &[event]).await), [event]);
        }
    }

    #[tokio::test]
    async fn the_sender_cancels_its_own_run_and_it_ends_cancelled() {
        let (door, _) = door(FakeSkills::new(vec![spec("count")]).held("count")).await;
        door.send("count", "Count", "run-1", json!({})).await;
        let mut requested = false;
        for i in 0..100 {
            door.send(CANCEL, "CancelSkill", &format!("x{i}"), json!({ "command": "run-1" }))
                .await;
            let events = door
                .events_until(&format!("x{i}"), &["SkillCancelRequested", "SkillCancelRefused"])
                .await;
            if types(&events) == ["SkillCancelRequested"] {
                requested = true;
                break;
            }
        }
        assert!(requested, "the run never took the cancel");
        assert_eq!(
            types(&door.events_until("run-1", &["CountCanceled"]).await),
            ["CountCanceled"]
        );
    }

    #[tokio::test]
    async fn a_run_the_robot_stopped_answering_during_fails_saying_it_may_still_run() {
        let (door, _) = door(FakeSkills::new(vec![spec("count")]).lost("count", vec![json!({ "at": 1 })])).await;
        door.send("count", "Count", "c1", json!({})).await;
        let events = door.events_until("c1", &["CommandFailed"]).await;
        assert_eq!(types(&events), ["CountProgressed", "CommandFailed"]);
        let reason = events[1]["data"]["reason"].as_str().unwrap();
        assert!(
            reason.contains("still be running") && reason.contains("new command id"),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn another_agent_cannot_cancel_a_run_it_did_not_start() {
        let (door, _) = door(FakeSkills::new(vec![spec("count")]).held("count")).await;
        door.send("count", "Count", "run-1", json!({})).await;
        let other = door.other();
        for i in 0..20 {
            other
                .send(CANCEL, "CancelSkill", &format!("x{i}"), json!({ "command": "run-1" }))
                .await;
            let events = other
                .events_until(&format!("x{i}"), &["SkillCancelRequested", "SkillCancelRefused"])
                .await;
            assert_eq!(types(&events), ["SkillCancelRefused"]);
        }
        // The run is still the first agent's to cancel.
        for i in 0..100 {
            door.send(CANCEL, "CancelSkill", &format!("y{i}"), json!({ "command": "run-1" }))
                .await;
            let events = door
                .events_until(&format!("y{i}"), &["SkillCancelRequested", "SkillCancelRefused"])
                .await;
            if types(&events) == ["SkillCancelRequested"] {
                return;
            }
        }
        panic!("the run's own sender could not cancel it");
    }

    #[tokio::test]
    async fn a_cancel_naming_no_run_in_flight_is_refused_with_why() {
        let (door, _) = door(FakeSkills::new(vec![spec("count")])).await;
        door.send(CANCEL, "CancelSkill", "x", json!({ "command": "never-sent" }))
            .await;
        let events = door.events_until("x", &["SkillCancelRefused"]).await;
        assert!(events[0]["data"]["reason"].as_str().unwrap().contains("lost track"));
    }
}
