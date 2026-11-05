//! The robot as the door sees it. The door depends on [`Robot`] and never on what stands behind it: the fake here,
//! and later an adapter over ROS 2. What the robot is on the wire is the door's matter, not this crate's.

use async_trait::async_trait;

mod fake;
pub use fake::Fake;

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
#[derive(Debug, thiserror::Error)]
#[error("the robot did not answer: {0}")]
pub struct Unreachable(pub String);
