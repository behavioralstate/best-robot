use async_trait::async_trait;
use serde_json::Value;

use crate::Unreachable;

/// A skill the robot offers: what it is called, what it does, and the JSON Schemas of its goal, its feedback and its
/// result.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub goal: Value,
    pub feedback: Value,
    pub result: Value,
}

/// How a run of a skill ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Succeeded(Value),
    /// The robot gave up on the goal; the result is what it had.
    Aborted(Value),
    /// Ended by a cancel; the result is what it had.
    Canceled(Value),
    /// The robot never started it: the reason says why (rejected, no such skill server, an unknown type).
    Refused(String),
}

/// Called with each feedback the robot sends while a skill runs.
pub type Progress = Box<dyn Fn(Value) + Send + Sync>;

/// What the robot can be asked to do.
#[async_trait]
pub trait Skills: Send + Sync {
    fn skills(&self) -> &[SkillSpec];
    /// Runs `skill` with `goal` until it ends, calling `progress` with each feedback. `run` names the run, so
    /// [`Skills::cancel`] can end it; it is the caller's and unique among the runs in flight.
    async fn run(&self, run: &str, skill: &str, goal: Value, progress: Progress) -> Result<Outcome, Unreachable>;
    /// Asks the robot to end the run `run`; false when no such run is in flight. The run itself then ends, as
    /// [`Outcome::Canceled`] or as whatever came first.
    async fn cancel(&self, run: &str) -> Result<bool, Unreachable>;
}
