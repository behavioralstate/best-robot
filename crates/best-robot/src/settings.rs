//! What the door is told at start: where it listens, the address agents reach, where it keeps its data and which
//! robot stands behind it. Read through a lookup the caller hands in, so the rules are tested without the process's
//! environment.

use std::net::SocketAddr;
use std::path::PathBuf;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:7780";

#[derive(Debug, PartialEq)]
pub struct Settings {
    pub listen: SocketAddr,
    pub endpoint: String,
    pub data: PathBuf,
    pub robot: RobotKind,
}

/// The robots this build can stand in front of.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RobotKind {
    /// Answers with a fixed state and moves nothing.
    Fake,
}

impl Settings {
    pub fn read(var: impl Fn(&str) -> Option<String>, home: Option<PathBuf>) -> Result<Self, String> {
        let listen_text = var("BEST_ROBOT_LISTEN").unwrap_or_else(|| DEFAULT_LISTEN.into());
        let listen: SocketAddr = listen_text.parse().map_err(|_| {
            format!("BEST_ROBOT_LISTEN={listen_text} is not an address and port, such as {DEFAULT_LISTEN}")
        })?;
        if !listen.ip().is_loopback() {
            return Err(format!(
                "BEST_ROBOT_LISTEN={listen} is off loopback, and this door serves http alone: off loopback the credential would travel in the clear. Listen on 127.0.0.1, behind a proxy on this machine that serves https if others must reach it"
            ));
        }
        if listen.port() == 0 {
            return Err(format!(
                "BEST_ROBOT_LISTEN={listen} names no port: agents and best-robot approve reach the door at the port it listens on, so give one, such as {DEFAULT_LISTEN}"
            ));
        }
        let endpoint = var("BEST_ROBOT_ENDPOINT").unwrap_or_else(|| format!("http://{listen}"));
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(format!(
                "BEST_ROBOT_ENDPOINT={endpoint} is not an http:// or https:// address: it is the base URL agents reach this door at"
            ));
        }
        let data = match (var("BEST_ROBOT_DATA"), home) {
            (Some(d), _) => PathBuf::from(d),
            (None, Some(h)) => h.join(".best-robot"),
            (None, None) => {
                return Err(
                    "no home directory to keep the door's data in: set BEST_ROBOT_DATA to an absolute path".into(),
                )
            }
        };
        let robot = match var("BEST_ROBOT_ROBOT").as_deref() {
            None | Some("fake") => RobotKind::Fake,
            Some(other) => {
                return Err(format!(
                    "BEST_ROBOT_ROBOT={other} is not a robot this build knows: fake"
                ))
            }
        };
        Ok(Self {
            listen,
            endpoint: endpoint.trim_end_matches('/').into(),
            data,
            robot,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn read(vars: &[(&str, &str)]) -> Result<Settings, String> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Settings::read(|k| vars.get(k).cloned(), Some(PathBuf::from("/home/me")))
    }

    #[test]
    fn nothing_said_is_loopback_the_fake_and_a_folder_in_home() {
        let s = read(&[]).unwrap();
        assert_eq!(s.listen.to_string(), DEFAULT_LISTEN);
        assert_eq!(s.endpoint, format!("http://{DEFAULT_LISTEN}"));
        assert_eq!(s.data, PathBuf::from("/home/me").join(".best-robot"));
        assert_eq!(s.robot, RobotKind::Fake);
    }

    #[test]
    fn an_address_off_loopback_is_refused_with_the_reason() {
        let e = read(&[("BEST_ROBOT_LISTEN", "0.0.0.0:7780")]).unwrap_err();
        assert!(e.contains("off loopback"), "{e}");
    }

    #[test]
    fn port_0_is_refused_since_no_one_could_reach_the_door_at_it() {
        let e = read(&[("BEST_ROBOT_LISTEN", "127.0.0.1:0")]).unwrap_err();
        assert!(e.contains("BEST_ROBOT_LISTEN") && e.contains("no port"), "{e}");
    }

    #[test]
    fn the_endpoint_is_kept_without_a_trailing_slash() {
        let s = read(&[("BEST_ROBOT_ENDPOINT", "https://robot.example/")]).unwrap();
        assert_eq!(s.endpoint, "https://robot.example");
    }

    #[test]
    fn what_is_not_understood_is_refused_by_name() {
        assert!(read(&[("BEST_ROBOT_LISTEN", "nowhere")])
            .unwrap_err()
            .contains("BEST_ROBOT_LISTEN"));
        assert!(read(&[("BEST_ROBOT_ENDPOINT", "robot.example")])
            .unwrap_err()
            .contains("BEST_ROBOT_ENDPOINT"));
        assert!(read(&[("BEST_ROBOT_ROBOT", "spot")]).unwrap_err().contains("fake"));
    }

    #[test]
    fn no_home_and_no_data_folder_says_what_to_set() {
        let e = Settings::read(|_| None, None).unwrap_err();
        assert!(e.contains("BEST_ROBOT_DATA"), "{e}");
    }
}
