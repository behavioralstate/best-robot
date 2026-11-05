//! `best-robot approve <code>`: the person at the machine admits the agent that showed them the code, by handing the
//! running door the admin token only this machine's owner can read.

use serde_json::Value;

use crate::settings::Settings;

/// Admits `code` on the door these settings describe, and says who was admitted.
pub fn approve(settings: &Settings, code: &str) -> Result<String, String> {
    let token_file = settings.data.join("admin-token");
    let token = std::fs::read_to_string(&token_file).map_err(|e| {
        format!(
            "no admin token at {}: is the door running on this machine with the same BEST_ROBOT_DATA? ({e})",
            token_file.display()
        )
    })?;
    let url = format!("http://{}/auth/approve", settings.listen);
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut answer = agent
        .post(&url)
        .send_form([("user_code", code.trim()), ("proof", token.trim())])
        .map_err(|e| format!("no door answered at {url}: is it running with the same BEST_ROBOT_LISTEN? ({e})"))?;
    let status = answer.status().as_u16();
    let body: Value = answer.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => Ok(admitted_line(&body["admitted"])),
        _ => Err(format!(
            "the door refused ({status}): {}",
            body["error_description"]
                .as_str()
                .unwrap_or("is the code one an agent showed in the last ten minutes?")
        )),
    }
}

fn admitted_line(a: &Value) -> String {
    let who = a["agent_label"]
        .as_str()
        .or(a["client_id"].as_str())
        .unwrap_or("the agent");
    match a["connection"].as_str() {
        Some(c) => format!("approved {who} as {c}"),
        None => format!("approved {who}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{door, settings::RobotKind};
    use std::sync::Arc;

    /// The whole sign-in against a door served in this test: an agent starts the device flow, the person approves
    /// its code, and the agent's next poll redeems a key that opens the robot's state.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_person_admits_the_agent_and_its_key_reads_the_robot() {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen = listener.local_addr().unwrap();
        let settings = Settings {
            listen,
            endpoint: format!("http://{listen}"),
            data: dir.path().to_path_buf(),
            robot: RobotKind::Fake,
        };
        let parts = crate::surface::Parts {
            state: Some(Arc::new(robot::Fake::default())),
            skills: None,
            topics: None,
        };
        let opened = door::open(&settings, "test", crate::surface::catalogue(parts).unwrap()).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let served = tokio::spawn(door::serve(listener, opened, async {
            stopped.await.ok();
        }));

        let base = settings.endpoint.clone();
        let line = tokio::task::spawn_blocking(move || {
            let agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build()
                .new_agent();
            let device: Value = agent
                .post(format!("{base}/auth/device"))
                .send_form([("client_id", "test"), ("connection", "test-robot")])
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            let line = approve(&settings, device["user_code"].as_str().unwrap()).unwrap();
            let token: Value = agent
                .post(format!("{base}/auth/token"))
                .send_form([
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device["device_code"].as_str().unwrap()),
                    ("client_id", "test"),
                ])
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            let key = token["access_token"].as_str().unwrap();
            let state: Value = agent
                .get(format!("{base}/queries/get-robot-state"))
                .header("X-Api-Key", key)
                .call()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert_eq!(state["name"], "fake", "the key did not read the robot: {state}");
            assert!(approve(&settings, "NOPE-NOPE").unwrap_err().contains("refused"));
            line
        })
        .await
        .unwrap();

        assert_eq!(line, "approved test as test-robot");
        stop.send(()).unwrap();
        served.await.unwrap().unwrap();
    }
}
