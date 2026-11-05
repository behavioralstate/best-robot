//! The robot's [`Skills`] and [`Topics`] over a ROS 2 rosbridge (2.x, which carries actions). A skill is a ROS 2
//! action the operator listed, its schemas generated from the type rosapi describes; a topic is one whose latest
//! message the door keeps. Nothing of ROS reaches past this crate.

mod client;
pub mod config;
mod schema;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use robot::{Outcome, Progress, SkillSpec, Skills, Snapshot, TopicSpec, Topics, Unreachable};
use serde_json::{json, Value};

use client::Client;
pub use config::Config;

/// How long the door waits for rosbridge at start.
const CONNECT_WITHIN: Duration = Duration::from_secs(15);

/// `action_msgs/msg/GoalStatus`, as rosbridge reports a goal's end.
const SUCCEEDED: u64 = 4;
const CANCELED: u64 = 5;

pub struct Rosbridge {
    client: Arc<Client>,
    skills: Vec<SkillSpec>,
    /// Skill name to its action and type.
    actions: HashMap<String, (String, String)>,
    topics: Vec<TopicSpec>,
    /// Topic name on the door to its ROS topic.
    ros_topics: HashMap<String, String>,
    /// Runs in flight, to the action each runs on.
    runs: Mutex<HashMap<String, String>>,
}

impl Rosbridge {
    /// Connects to the rosbridge `config` names and describes every skill and topic it lists, from rosapi. A type
    /// rosapi does not know stops it here, named.
    pub async fn connect(config: Config) -> Result<Self, Unreachable> {
        let ros_topics: Vec<(String, String)> = config
            .topics
            .iter()
            .map(|t| (t.topic.clone(), t.message_type.clone()))
            .collect();
        let client = Client::start(&config.rosbridge, ros_topics);
        client.connected(CONNECT_WITHIN).await?;
        let mut skills = Vec::new();
        for s in &config.skills {
            skills.push(SkillSpec {
                name: s.name.clone(),
                description: s.description.clone(),
                goal: describe(&client, "/rosapi/action_goal_details", &s.action_type).await?,
                feedback: describe(&client, "/rosapi/action_feedback_details", &s.action_type).await?,
                result: describe(&client, "/rosapi/action_result_details", &s.action_type).await?,
            });
        }
        let mut topics = Vec::new();
        for t in &config.topics {
            topics.push(TopicSpec {
                name: t.name.clone(),
                description: t.description.clone(),
                message: describe(&client, "/rosapi/message_details", &t.message_type).await?,
            });
        }
        Ok(Self {
            client,
            skills,
            actions: config
                .skills
                .into_iter()
                .map(|s| (s.name, (s.action, s.action_type)))
                .collect(),
            topics,
            ros_topics: config.topics.into_iter().map(|t| (t.name, t.topic)).collect(),
            runs: Mutex::new(HashMap::new()),
        })
    }
}

async fn describe(client: &Client, service: &str, ros_type: &str) -> Result<Value, Unreachable> {
    let values = client
        .call(service, json!({ "type": ros_type }))
        .await?
        .map_err(|why| Unreachable(format!("rosapi could not describe {ros_type}: {why}")))?;
    let typedefs: Vec<schema::Typedef> = serde_json::from_value(values["typedefs"].clone()).map_err(|e| {
        Unreachable(format!(
            "rosapi described {ros_type} in a shape this door does not read: {e}"
        ))
    })?;
    schema::of(&typedefs).map_err(|why| Unreachable(format!("{ros_type}: {why}")))
}

#[async_trait]
impl Skills for Rosbridge {
    fn skills(&self) -> &[SkillSpec] {
        &self.skills
    }

    async fn run(&self, run: &str, skill: &str, goal: Value, progress: Progress) -> Result<Outcome, Unreachable> {
        let Some((action, action_type)) = self.actions.get(skill) else {
            return Ok(Outcome::Refused(format!("no skill {skill} is configured")));
        };
        let send = json!({ "op": "send_action_goal", "id": run, "action": action, "action_type": action_type,
            "args": goal, "feedback": true });
        let mut answers = self.client.open(run, send)?;
        self.runs.lock().unwrap().insert(run.into(), action.clone());
        let end = loop {
            let Some(answer) = answers.recv().await else {
                break Err(Unreachable(format!(
                    "lost rosbridge while {skill} ran; the robot may still be running it"
                )));
            };
            match answer["op"].as_str() {
                Some("action_feedback") => progress(answer["values"].clone()),
                Some("action_result") => break Ok(outcome(&answer)),
                _ => {}
            }
        };
        self.runs.lock().unwrap().remove(run);
        self.client.close(run);
        end
    }

    async fn cancel(&self, run: &str) -> Result<bool, Unreachable> {
        let Some(action) = self.runs.lock().unwrap().get(run).cloned() else {
            return Ok(false);
        };
        self.client
            .send(json!({ "op": "cancel_action_goal", "id": run, "action": action }))?;
        Ok(true)
    }
}

fn outcome(answer: &Value) -> Outcome {
    let values = answer["values"].clone();
    if answer["result"] != true {
        return Outcome::Refused(
            values
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| values.to_string()),
        );
    }
    match answer["status"].as_u64() {
        Some(SUCCEEDED) => Outcome::Succeeded(values),
        Some(CANCELED) => Outcome::Canceled(values),
        _ => Outcome::Aborted(values),
    }
}

impl Topics for Rosbridge {
    fn topics(&self) -> &[TopicSpec] {
        &self.topics
    }

    fn latest(&self, topic: &str) -> Option<Snapshot> {
        self.ros_topics.get(topic).and_then(|t| self.client.latest(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_reads_as_its_status_says() {
        let r = |v: Value| outcome(&v);
        assert_eq!(
            r(json!({ "result": true, "status": 4, "values": { "s": 1 } })),
            Outcome::Succeeded(json!({ "s": 1 }))
        );
        assert_eq!(
            r(json!({ "result": true, "status": 5, "values": {} })),
            Outcome::Canceled(json!({}))
        );
        assert_eq!(
            r(json!({ "result": true, "status": 6, "values": {} })),
            Outcome::Aborted(json!({}))
        );
        assert_eq!(
            r(json!({ "result": false, "status": 0, "values": "Action goal was rejected" })),
            Outcome::Refused("Action goal was rejected".into())
        );
    }
}
