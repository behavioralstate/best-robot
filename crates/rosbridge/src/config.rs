//! What the operator exposes: the rosbridge to reach, the actions that become skills and the topics whose latest
//! message is read. Every name is checked here, so a typo stops the door at start with the field named.

use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The rosbridge websocket, `ws://host:port`.
    pub rosbridge: String,
    #[serde(default)]
    pub skills: Vec<SkillConfig>,
    #[serde(default)]
    pub topics: Vec<TopicConfig>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillConfig {
    /// The skill's name on the door, kebab-case.
    pub name: String,
    /// The ROS 2 action, such as `/navigate_to_pose`.
    pub action: String,
    /// Its type, such as `nav2_msgs/action/NavigateToPose`.
    #[serde(rename = "type")]
    pub action_type: String,
    pub description: String,
    /// For an action that takes the robot somewhere: where its goal names that place, so the owner's bounds can be
    /// held to it.
    pub target: Option<TargetConfig>,
}

/// JSON Pointers into a goal: the frame of the place it names, and its x and y.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub frame: String,
    pub x: String,
    pub y: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TopicConfig {
    /// The topic's name on the door, kebab-case.
    pub name: String,
    /// The ROS 2 topic, such as `/battery_state`.
    pub topic: String,
    /// Its message type, such as `sensor_msgs/msg/BatteryState`.
    #[serde(rename = "type")]
    pub message_type: String,
    pub description: String,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if !config.rosbridge.starts_with("ws://") {
            return Err(format!(
                "rosbridge = {:?} is not a ws:// address, such as ws://127.0.0.1:9090",
                config.rosbridge
            ));
        }
        let mut names = std::collections::HashSet::new();
        for (kind, name, description, ros_type, middle) in config
            .skills
            .iter()
            .map(|s| ("skills", &s.name, &s.description, &s.action_type, "action"))
            .chain(
                config
                    .topics
                    .iter()
                    .map(|t| ("topics", &t.name, &t.description, &t.message_type, "msg")),
            )
        {
            let at = format!("{kind} {name:?}");
            if !is_kebab(name) {
                return Err(format!(
                    "{at}: name is not kebab-case (lower-case letters, digits and single dashes)"
                ));
            }
            if !names.insert(name.as_str()) {
                return Err(format!("{at}: name is used twice; each skill and topic needs its own"));
            }
            if description.trim().is_empty() || description.chars().count() > 500 || description.contains("://") {
                return Err(format!(
                    "{at}: description must say what it is in 1 to 500 characters, with no URL"
                ));
            }
            let parts: Vec<&str> = ros_type.split('/').collect();
            if parts.len() != 3 || parts[1] != middle || parts.iter().any(|p| p.is_empty()) {
                return Err(format!("{at}: type {ros_type:?} is not package/{middle}/Name"));
            }
        }
        for s in &config.skills {
            if let Some(t) = &s.target {
                for (field, pointer) in [("frame", &t.frame), ("x", &t.x), ("y", &t.y)] {
                    if !pointer.starts_with('/') {
                        return Err(format!(
                            "skills {:?}: target.{field} {pointer:?} is not a JSON Pointer into the goal, such as /pose/pose/position/x",
                            s.name
                        ));
                    }
                }
            }
        }
        Ok(config)
    }
}

fn is_kebab(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase())
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
rosbridge = "ws://127.0.0.1:9090"

[[skills]]
name = "fibonacci"
action = "/fibonacci"
type = "example_interfaces/action/Fibonacci"
description = "Computes a Fibonacci sequence of the given order."

[[topics]]
name = "battery"
topic = "/battery_state"
type = "sensor_msgs/msg/BatteryState"
description = "The battery's state."
"#;

    #[test]
    fn a_good_config_reads_whole() {
        let c = Config::parse(GOOD).unwrap();
        assert_eq!(c.skills[0].action_type, "example_interfaces/action/Fibonacci");
        assert_eq!(c.topics[0].topic, "/battery_state");
    }

    #[test]
    fn each_mistake_is_refused_naming_the_field() {
        let cases = [
            (GOOD.replace("ws://", "http://"), "ws://"),
            (GOOD.replace("name = \"battery\"", "name = \"Battery\""), "kebab-case"),
            (GOOD.replace("name = \"battery\"", "name = \"fibonacci\""), "used twice"),
            (GOOD.replace("The battery's state.", ""), "description"),
            (
                GOOD.replace("sensor_msgs/msg/BatteryState", "sensor_msgs/BatteryState"),
                "package/msg/Name",
            ),
            (
                GOOD.replace("example_interfaces/action/", "example_interfaces/msg/"),
                "package/action/Name",
            ),
            (GOOD.replace("action = ", "acton = "), "acton"),
            (
                GOOD.replace(
                    "description = \"Computes",
                    "target = { frame = \"/f\", x = \"x\", y = \"/y\" }
description = \"Computes",
                ),
                "target.x",
            ),
        ];
        for (text, said) in cases {
            let e = Config::parse(&text).unwrap_err();
            assert!(e.contains(said), "{said:?} not in {e:?}");
        }
    }
}
