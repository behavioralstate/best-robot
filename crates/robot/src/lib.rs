//! The robot as the door sees it, in three narrow ports an adapter implements as far as it can: [`Robot`] (its state),
//! [`Skills`] (what it can be asked to do) and [`Topics`] (what it last said). The door depends on these and never on
//! what stands behind them: the fakes here, or an adapter over ROS 2. Goals, feedback and messages are JSON values, as
//! a robot's middleware carries them; how the door names and serves them is the door's matter.

use async_trait::async_trait;

mod fake;
mod skills;
mod topics;
pub use fake::{Fake, FakeSkills, FakeTopics};
pub use skills::{Outcome, Progress, SkillSpec, Skills};
pub use topics::{Snapshot, TopicSpec, Topics};

/// A robot the door speaks for.
#[async_trait]
pub trait Robot: Send + Sync {
    /// What the robot is doing now, where it is and how much charge it has.
    async fn state(&self) -> Result<State, Unreachable>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    pub status: Status,
    /// Where the robot is, when it knows.
    pub pose: Option<Pose>,
    /// Charge left, 0 to 100, when the robot reports one.
    pub battery_percent: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Moving,
    Fault,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Moving => "moving",
            Status::Fault => "fault",
        }
    }
}

/// A pose in a named frame: metres and radians, as ROS gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub frame: String,
    pub x: f64,
    pub y: f64,
    pub yaw: f64,
}

/// The robot did not answer; the reason says what was tried.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("the robot did not answer: {0}")]
pub struct Unreachable(pub String);
