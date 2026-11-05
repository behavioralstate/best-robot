use std::time::SystemTime;

use serde_json::Value;

/// A topic the robot publishes and the door keeps the latest message of: its name, what it says, and the JSON Schema of
/// a message.
#[derive(Clone, Debug, PartialEq)]
pub struct TopicSpec {
    pub name: String,
    pub description: String,
    pub message: Value,
}

/// The latest message on a topic, and when it arrived.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub message: Value,
    pub received_at: SystemTime,
}

/// What the robot last said on each topic.
pub trait Topics: Send + Sync {
    fn topics(&self) -> &[TopicSpec];
    /// The latest message on `topic`, none when nothing has arrived since the door started.
    fn latest(&self, topic: &str) -> Option<Snapshot>;
}
