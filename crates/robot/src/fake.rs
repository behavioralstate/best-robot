use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Notify;

use crate::{
    Outcome, Pose, Progress, Robot, SkillSpec, Skills, Snapshot, State, Status, TopicSpec, Topics, Unreachable,
};

/// A robot that answers with the state it was given: idle at the origin of `map`, fully charged, unless told
/// otherwise.
pub struct Fake {
    state: State,
}

impl Fake {
    pub fn new(state: State) -> Self {
        Self { state }
    }
}

impl Default for Fake {
    fn default() -> Self {
        Self::new(State {
            name: "fake".into(),
            status: Status::Idle,
            pose: Some(Pose {
                frame: "map".into(),
                x: 0.0,
                y: 0.0,
                yaw: 0.0,
            }),
            battery_percent: Some(100.0),
        })
    }
}

#[async_trait]
impl Robot for Fake {
    async fn state(&self) -> Result<State, Unreachable> {
        Ok(self.state.clone())
    }
}

/// How a fake skill behaves: the feedback it sends, then how it ends, or that the robot stopped answering. `held` runs
/// wait for a cancel instead, and end [`Outcome::Canceled`] with a null result.
#[derive(Clone)]
pub struct Script {
    pub feedback: Vec<Value>,
    pub end: Result<Outcome, Unreachable>,
    pub held: bool,
}

/// Skills that do what their script says, and record each goal they were given.
pub struct FakeSkills {
    specs: Vec<SkillSpec>,
    scripts: HashMap<String, Script>,
    running: Mutex<HashMap<String, std::sync::Arc<Notify>>>,
    goals: Mutex<Vec<(String, Value)>>,
}

impl FakeSkills {
    pub fn new(specs: Vec<SkillSpec>) -> Self {
        Self {
            specs,
            scripts: HashMap::new(),
            running: Mutex::new(HashMap::new()),
            goals: Mutex::new(Vec::new()),
        }
    }

    /// `skill` sends `feedback`, then ends with `outcome`.
    pub fn ends(mut self, skill: &str, feedback: Vec<Value>, outcome: Outcome) -> Self {
        let script = Script {
            feedback,
            end: Ok(outcome),
            held: false,
        };
        self.scripts.insert(skill.into(), script);
        self
    }

    /// `skill` sends `feedback`, then the robot stops answering, as when its rosbridge is lost mid-run.
    pub fn lost(mut self, skill: &str, feedback: Vec<Value>) -> Self {
        let script = Script {
            feedback,
            end: Err(Unreachable("the connection was lost while it ran".into())),
            held: false,
        };
        self.scripts.insert(skill.into(), script);
        self
    }

    /// `skill` sends `feedback`, then runs until it is cancelled.
    pub fn held(mut self, skill: &str, feedback: Vec<Value>) -> Self {
        let script = Script {
            feedback,
            end: Ok(Outcome::Canceled(Value::Null)),
            held: true,
        };
        self.scripts.insert(skill.into(), script);
        self
    }

    /// The goals given so far, by skill, in order.
    pub fn goals(&self) -> Vec<(String, Value)> {
        self.goals.lock().unwrap().clone()
    }
}

#[async_trait]
impl Skills for FakeSkills {
    fn skills(&self) -> &[SkillSpec] {
        &self.specs
    }

    async fn run(&self, run: &str, skill: &str, goal: Value, progress: Progress) -> Result<Outcome, Unreachable> {
        self.goals.lock().unwrap().push((skill.into(), goal));
        let Some(script) = self.scripts.get(skill) else {
            return Ok(Outcome::Refused(format!("no script for {skill}")));
        };
        let cancelled = std::sync::Arc::new(Notify::new());
        self.running.lock().unwrap().insert(run.into(), cancelled.clone());
        for f in &script.feedback {
            progress(f.clone());
        }
        if script.held {
            cancelled.notified().await;
        }
        self.running.lock().unwrap().remove(run);
        script.end.clone()
    }

    async fn cancel(&self, run: &str) -> Result<bool, Unreachable> {
        match self.running.lock().unwrap().get(run) {
            Some(n) => {
                n.notify_one();
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Topics whose latest messages are the ones it was given: dated long ago, or, for a live one, just now on every read,
/// as a topic the robot keeps publishing.
pub struct FakeTopics {
    specs: Vec<TopicSpec>,
    latest: HashMap<String, (Value, bool)>,
}

impl FakeTopics {
    pub fn new(specs: Vec<TopicSpec>) -> Self {
        Self {
            specs,
            latest: HashMap::new(),
        }
    }

    pub fn with(mut self, topic: &str, message: Value) -> Self {
        self.latest.insert(topic.into(), (message, false));
        self
    }

    pub fn live(mut self, topic: &str, message: Value) -> Self {
        self.latest.insert(topic.into(), (message, true));
        self
    }
}

impl Topics for FakeTopics {
    fn topics(&self) -> &[TopicSpec] {
        &self.specs
    }

    fn latest(&self, topic: &str) -> Option<Snapshot> {
        self.latest.get(topic).map(|(m, live)| Snapshot {
            message: m.clone(),
            received_at: if *live {
                SystemTime::now()
            } else {
                SystemTime::UNIX_EPOCH
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn answers_with_the_state_it_was_given() {
        let given = State {
            name: "r1".into(),
            status: Status::Fault,
            pose: None,
            battery_percent: None,
        };
        assert_eq!(Fake::new(given.clone()).state().await.unwrap(), given);
    }

    #[tokio::test]
    async fn a_held_skill_runs_until_cancelled() {
        let skills = Arc::new(FakeSkills::new(vec![]).held("wait", vec![]));
        let running = {
            let skills = skills.clone();
            tokio::spawn(async move { skills.run("r1", "wait", json!({}), Box::new(|_| {})).await })
        };
        while !skills.cancel("r1").await.unwrap() {
            tokio::task::yield_now().await;
        }
        assert_eq!(running.await.unwrap().unwrap(), Outcome::Canceled(Value::Null));
        assert!(!skills.cancel("r1").await.unwrap(), "the run is over");
    }
}
