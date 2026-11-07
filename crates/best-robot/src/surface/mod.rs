//! What the door offers, built from the parts of the robot it is handed: its state as a query, its skills as commands
//! with their events, its topics as queries. Each part is optional, as far as the adapter behind it goes; the wire
//! shapes live in these modules, next to their schemas.

pub mod frames;
mod skills;
mod state;
mod topics;

use std::collections::HashSet;
use std::sync::Arc;

use best_peer_door::Catalogue;
use robot::{Robot, Skills, Topics};

use frames::Frames;

pub struct Parts {
    pub state: Option<Arc<dyn Robot>>,
    pub skills: Option<Arc<dyn Skills>>,
    pub topics: Option<Arc<dyn Topics>>,
}

/// What the door says it is, in its manifest, from the parts it was handed.
pub fn describe(parts: &Parts) -> String {
    let mut offers = Vec::new();
    if parts.state.is_some() {
        offers.push("its state, read by query");
    }
    if parts.skills.as_ref().is_some_and(|s| !s.skills().is_empty()) {
        offers.push("its skills, sent as commands, their runs told as events");
    }
    if parts.topics.as_ref().is_some_and(|t| !t.topics().is_empty()) {
        offers.push("the latest message on each of its topics, read by query");
    }
    match offers.as_slice() {
        [] => "A robot that offers nothing yet.".into(),
        _ => format!("A robot: {}.", offers.join("; ")),
    }
}

/// The catalogue over `parts`, a camera's images kept in `frames`, or why two parts would answer to the same name or
/// type.
pub fn catalogue(parts: Parts, frames: &Arc<Frames>) -> Result<Catalogue, String> {
    let mut names = HashSet::new();
    let mut claim = |name: String, what: &str| {
        if names.insert(name.clone()) {
            Ok(())
        } else {
            Err(format!(
                "{what} would answer to {name}, which another part of the door already does: rename it"
            ))
        }
    };
    let mut c = Catalogue::new();
    if let Some(robot) = parts.state {
        claim("get-robot-state".into(), "the robot's state")?;
        c = c.query(state::query(robot));
    }
    if let Some(sk) = parts.skills.filter(|s| !s.skills().is_empty()) {
        for s in sk.skills() {
            claim(s.name.clone(), &format!("skill {}", s.name))?;
            for ty in skills::types(&s.name) {
                claim(ty, &format!("skill {}", s.name))?;
            }
        }
        claim(skills::CANCEL.into(), "cancelling a skill")?;
        for ty in skills::CANCEL_TYPES {
            claim(ty.into(), "cancelling a skill")?;
        }
        c = skills::add(c, sk);
    }
    if let Some(t) = parts.topics {
        for spec in t.topics() {
            claim(topics::query_name(&spec.name), &format!("topic {}", spec.name))?;
        }
        c = topics::add(c, t, frames);
    }
    Ok(c)
}

/// `kebab-case` as `PascalCase`.
fn pascal(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use robot::{FakeSkills, FakeTopics, SkillSpec, TopicSpec};
    use serde_json::json;

    fn frames() -> Arc<Frames> {
        Frames::new("http://door.test")
    }

    fn skill(name: &str) -> SkillSpec {
        SkillSpec {
            name: name.into(),
            description: "d".into(),
            goal: json!({ "type": "object" }),
            feedback: json!({ "type": "object" }),
            result: json!({ "type": "object" }),
            target: None,
        }
    }

    #[test]
    fn kebab_names_become_pascal_types() {
        assert_eq!(pascal("navigate-to-pose"), "NavigateToPose");
        assert_eq!(pascal("fibonacci"), "Fibonacci");
    }

    #[test]
    fn a_skill_or_topic_that_would_shadow_another_name_stops_the_door() {
        let shadowing_cancel = Parts {
            state: None,
            skills: Some(Arc::new(FakeSkills::new(vec![skill("cancel-skill")]))),
            topics: None,
        };
        assert!(catalogue(shadowing_cancel, &frames())
            .err()
            .unwrap()
            .contains("cancel-skill"));
        let topic = TopicSpec {
            name: "robot-state".into(),
            description: "d".into(),
            frames: false,
            message: json!({ "type": "object" }),
        };
        let shadowing_state = Parts {
            state: Some(Arc::new(robot::Fake::default())),
            skills: None,
            topics: Some(Arc::new(FakeTopics::new(vec![topic]))),
        };
        assert!(catalogue(shadowing_state, &frames())
            .err()
            .unwrap()
            .contains("get-robot-state"));
    }

    #[test]
    fn two_skills_whose_types_would_be_the_same_stop_the_door() {
        // x-1 and x1 are both X1; count-progressed's command would be count's feedback event.
        for pair in [["x-1", "x1"], ["count", "count-progressed"]] {
            let parts = Parts {
                state: None,
                skills: Some(Arc::new(FakeSkills::new(pair.map(skill).to_vec()))),
                topics: None,
            };
            assert!(catalogue(parts, &frames()).is_err(), "{pair:?}");
        }
    }

    #[test]
    fn the_description_and_cancel_skill_follow_what_the_parts_offer() {
        let none = Parts {
            state: Some(Arc::new(robot::Fake::default())),
            skills: Some(Arc::new(FakeSkills::new(vec![]))),
            topics: None,
        };
        assert_eq!(describe(&none), "A robot: its state, read by query.");
        let c = catalogue(none, &frames()).unwrap();
        assert!(
            c.command_by_schema(skills::CANCEL, "1.0").is_none(),
            "no skill, nothing to cancel"
        );
        let some = Parts {
            state: None,
            skills: Some(Arc::new(FakeSkills::new(vec![skill("count")]))),
            topics: None,
        };
        assert!(describe(&some).contains("skills"));
        assert!(catalogue(some, &frames())
            .unwrap()
            .command_by_schema(skills::CANCEL, "1.0")
            .is_some());
    }
}
