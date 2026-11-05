use async_trait::async_trait;
use serde_json::Value;

use crate::Unreachable;

/// A skill the robot offers: what it is called, what it does, the JSON Schemas of its goal, its feedback and its
/// result, and, for a skill that takes the robot somewhere, where in the goal that place is.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillSpec {
    pub name: String,
    pub description: String,
    pub goal: Value,
    pub feedback: Value,
    pub result: Value,
    pub target: Option<Target>,
}

/// Where a skill's goal names the place it takes the robot: JSON Pointers (RFC 6901) into the goal, to the frame the
/// place is in and to its x and y.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub frame: String,
    pub x: String,
    pub y: String,
}

/// The place a goal names, by `target`: its frame, x and y, or what is missing.
pub fn place(goal: &Value, target: &Target) -> Result<(String, f64, f64), String> {
    let number = |at: &str| {
        goal.pointer(at)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("the goal has no number at {at}"))
    };
    let frame = goal
        .pointer(&target.frame)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the goal names no frame at {}", target.frame))?;
    Ok((frame.to_string(), number(&target.x)?, number(&target.y)?))
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
    /// Ended by whoever guards the robot, not by the agent: its owner took over, or the robot left the bounds its
    /// owner set. The reason says which; the result is what the robot had.
    Stopped {
        reason: String,
        result: Value,
    },
}

/// Why a goal is not taken, before it is sent: a code, what is wrong and what to send instead, and the facts behind
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    /// Whether the goal itself is wrong, or only now is: a goal sent again later may be taken.
    pub now: bool,
}

/// Called with each feedback the robot sends while a skill runs.
pub type Progress = Box<dyn Fn(Value) + Send + Sync>;

/// What the robot can be asked to do.
#[async_trait]
pub trait Skills: Send + Sync {
    fn skills(&self) -> &[SkillSpec];
    /// Whether `goal` for `skill` would be taken now, asked before a command is accepted, so a goal that would be
    /// refused is refused at once with what to send instead. A robot takes every goal its schema allows: only a guard
    /// in front of it says otherwise.
    fn check(&self, _skill: &str, _goal: &Value) -> Result<(), Refusal> {
        Ok(())
    }
    /// Runs `skill` with `goal` until it ends, calling `progress` with each feedback. `run` names the run, so
    /// [`Skills::cancel`] can end it; it is the caller's and unique among the runs in flight.
    async fn run(&self, run: &str, skill: &str, goal: Value, progress: Progress) -> Result<Outcome, Unreachable>;
    /// Asks the robot to end the run `run`; false when no such run is in flight. The run itself then ends, as
    /// [`Outcome::Canceled`] or as whatever came first.
    async fn cancel(&self, run: &str) -> Result<bool, Unreachable>;
}
