//! What the testbed tests share: the door's config for the testbed, and a bridge connected to it.

use std::sync::Arc;

use rosbridge::{Config, Rosbridge};

pub fn config(skill_type: &str) -> Config {
    let url = std::env::var("BEST_ROBOT_TEST_ROSBRIDGE").unwrap_or_else(|_| "ws://127.0.0.1:9091".into());
    Config::parse(&format!(
        r#"
rosbridge = "{url}"
[[skills]]
name = "fibonacci"
action = "/fibonacci"
type = "{skill_type}"
description = "Computes a Fibonacci sequence of the given order."
[[topics]]
name = "battery"
topic = "/battery_state"
type = "sensor_msgs/msg/BatteryState"
description = "The battery's state."
"#
    ))
    .unwrap()
}

pub async fn testbed() -> Arc<Rosbridge> {
    Arc::new(
        Rosbridge::connect(config("example_interfaces/action/Fibonacci"))
            .await
            .unwrap(),
    )
}
