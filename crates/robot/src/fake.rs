use async_trait::async_trait;

use crate::{Pose, Robot, State, Status, Unreachable};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
