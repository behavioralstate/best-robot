//! What the door offers, built from the parts of the robot it is handed: its state as a query, its skills as commands
//! with their events, its topics as queries. Each part is optional, as far as the adapter behind it goes; the wire
//! shapes live in these modules, next to their schemas.

mod skills;
mod state;
mod topics;

use std::collections::HashSet;
use std::sync::Arc;

use best_peer_door::Catalogue;
use robot::{Robot, Skills, Topics};

pub struct Parts {
    pub state: Option<Arc<dyn Robot>>,
    pub skills: Option<Arc<dyn Skills>>,
    pub topics: Option<Arc<dyn Topics>>,
}

/// The catalogue over `parts`, or why two of them would answer to the same name.
pub fn catalogue(parts: Parts) -> Result<Catalogue, String> {
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
    if let Some(sk) = parts.skills {
        for s in sk.skills() {
            claim(s.name.clone(), &format!("skill {}", s.name))?;
        }
        claim(skills::CANCEL.into(), "cancelling a skill")?;
        c = skills::add(c, sk);
    }
    if let Some(t) = parts.topics {
        for spec in t.topics() {
            claim(topics::query_name(&spec.name), &format!("topic {}", spec.name))?;
        }
        c = topics::add(c, t);
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

    fn skill(name: &str) -> SkillSpec {
        SkillSpec {
            name: name.into(),
            description: "d".into(),
            goal: json!({ "type": "object" }),
            feedback: json!({ "type": "object" }),
            result: json!({ "type": "object" }),
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
        assert!(catalogue(shadowing_cancel).err().unwrap().contains("cancel-skill"));
        let topic = TopicSpec {
            name: "robot-state".into(),
            description: "d".into(),
            message: json!({ "type": "object" }),
        };
        let shadowing_state = Parts {
            state: Some(Arc::new(robot::Fake::default())),
            skills: None,
            topics: Some(Arc::new(FakeTopics::new(vec![topic]))),
        };
        assert!(catalogue(shadowing_state).err().unwrap().contains("get-robot-state"));
    }
}
