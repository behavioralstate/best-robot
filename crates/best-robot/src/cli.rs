//! The owner's commands, run on the machine the door runs on: `best-robot approve <code>` admits the agent that
//! showed the code, for every skill or the ones `--may` names, for as long as `--expires` says; `best-robot hold`
//! takes the robot over and `best-robot release` gives it back. Each hands the running door the admin token only this
//! machine's owner can read.

use serde_json::Value;

use crate::settings::Settings;

/// What `approve` grants: the scopes, comma-separated, and the seconds the key lasts; neither means every skill, for
/// as long as the agent asked.
#[derive(Default)]
pub struct Grant<'a> {
    pub may: Option<&'a str>,
    pub expires_in: Option<u64>,
}

/// Admits `code` on the door these settings describe, and says who was admitted.
pub fn approve(settings: &Settings, code: &str, grant: Grant) -> Result<String, String> {
    let expires = grant.expires_in.map(|s| s.to_string());
    let mut form = vec![("user_code", code.trim())];
    form.extend(grant.may.map(|m| ("may", m)));
    form.extend(expires.as_deref().map(|e| ("expires_in", e)));
    let body = post(settings, "/auth/approve", form, |b| {
        b["error_description"]
            .as_str()
            .unwrap_or("is the code one an agent showed in the last ten minutes?")
            .to_string()
    })?;
    Ok(admitted_line(&body["admitted"], grant.may))
}

/// Holds the robot: no skill starts until `release`; with `now`, the runs in flight are stopped too, each told to its
/// holder. Says what was held and what was stopped.
pub fn hold(settings: &Settings, now: bool, reason: Option<&str>) -> Result<String, String> {
    let mut form = vec![];
    if now {
        form.push(("now", "true"));
    }
    form.extend(reason.map(|r| ("reason", r)));
    let body = post(settings, "/owner/hold", form, owner_error)?;
    let mut lines = vec![format!(
        "held since {} ({}): no skill starts until best-robot release",
        body["held"]["since"].as_str().unwrap_or("now"),
        body["held"]["reason"].as_str().unwrap_or("")
    )];
    lines.extend(body["warning"].as_str().map(|w| format!("warning: {w}")));
    for s in body["stopped"].as_array().into_iter().flatten() {
        let run = s["run"].as_str().unwrap_or("?");
        lines.push(match s["ended"].as_bool() {
            Some(true) => format!("run {run} has ended, and its holder is told how"),
            _ => {
                format!("run {run} has not ended yet: the door keeps cancelling it; stop it at the robot if it runs on")
            }
        });
    }
    Ok(lines.join("\n"))
}

/// Releases the robot.
pub fn release(settings: &Settings) -> Result<String, String> {
    let body = post(settings, "/owner/release", vec![], owner_error)?;
    Ok(match body["released"].as_bool() {
        Some(true) => "released: skills start again".into(),
        _ => "the robot was not held".into(),
    })
}

/// How long a key lasts, from `90s`, `30m`, `2h`, `7d` or plain seconds.
pub fn seconds(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let (number, unit) = match t.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => t.split_at(i),
        None => (t, "s"),
    };
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => 0,
    };
    match number.parse::<u64>() {
        Ok(n) if n > 0 && scale > 0 && n.checked_mul(scale).is_some() => Ok(n * scale),
        _ => Err(format!(
            "--expires {text:?} is not a span: write it as 90s, 30m, 2h or 7d"
        )),
    }
}

/// POSTs `form` with the admin token as its proof, and returns the answer's body, or the refusal `said` reads in it.
fn post(
    settings: &Settings,
    path: &str,
    form: Vec<(&str, &str)>,
    said: impl Fn(&Value) -> String,
) -> Result<Value, String> {
    let token_file = settings.data.join("admin-token");
    let token = std::fs::read_to_string(&token_file).map_err(|e| {
        format!(
            "no admin token at {}: is the door running on this machine with the same BEST_ROBOT_DATA? ({e})",
            token_file.display()
        )
    })?;
    let form: Vec<(&str, &str)> = form.into_iter().chain([("proof", token.trim())]).collect();
    let url = format!("http://{}{path}", settings.listen);
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut answer = agent
        .post(&url)
        .send_form(form)
        .map_err(|e| format!("no door answered at {url}: is it running with the same BEST_ROBOT_LISTEN? ({e})"))?;
    let status = answer.status().as_u16();
    let body: Value = answer.body_mut().read_json().unwrap_or(Value::Null);
    match status {
        200 => Ok(body),
        _ => Err(format!("the door refused ({status}): {}", said(&body))),
    }
}

fn owner_error(body: &Value) -> String {
    body["error"]["message"]
        .as_str()
        .unwrap_or("is the door running with the same BEST_ROBOT_DATA?")
        .to_string()
}

fn admitted_line(a: &Value, may: Option<&str>) -> String {
    let who = a["agent_label"]
        .as_str()
        .or(a["client_id"].as_str())
        .unwrap_or("the agent");
    let as_what = a["connection"].as_str().map(|c| format!(" as {c}")).unwrap_or_default();
    let what = may.map(|m| format!(", for {m} alone")).unwrap_or_default();
    format!("approved {who}{as_what}{what}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::envelope::Envelope;
    use crate::{door, settings::RobotKind, surface::Parts};
    use robot::{FakeSkills, Outcome, SkillSpec, Target};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn a_span_reads_in_seconds() {
        assert_eq!(seconds("90s"), Ok(90));
        assert_eq!(seconds("2h"), Ok(7200));
        assert_eq!(seconds("7d"), Ok(604800));
        assert_eq!(seconds("45"), Ok(45));
        assert!(seconds("2 weeks").is_err());
        assert!(seconds("0m").is_err());
        assert!(seconds("213503982334602d").is_err(), "a span that overflows is not one");
    }

    struct Served {
        settings: Settings,
        base: String,
        agent: ureq::Agent,
    }

    impl Served {
        fn device(&self, client: &str) -> Value {
            self.agent
                .post(format!("{}/auth/device", self.base))
                .send_form([("client_id", client), ("connection", &format!("{client}-robot"))])
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap()
        }

        fn sign_in(&self, client: &str, grant: Grant) -> String {
            let device = self.device(client);
            approve(&self.settings, device["user_code"].as_str().unwrap(), grant).unwrap();
            let token: Value = self
                .agent
                .post(format!("{}/auth/token", self.base))
                .send_form([
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device["device_code"].as_str().unwrap()),
                    ("client_id", client),
                ])
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            token["access_token"].as_str().unwrap().into()
        }

        fn send(&self, key: &str, schema: &str, ty: &str, id: &str, data: Value) -> (u16, Value) {
            let envelope = json!({ "specversion": "1.0", "id": id, "source": "test", "type": ty,
                "datacontenttype": "application/json", "dataschema": format!("{}/commands/{schema}/1.0", self.base),
                "time": "2026-11-05T00:00:00Z", "data": data });
            let mut res = self
                .agent
                .post(format!("{}/commands", self.base))
                .header("X-Api-Key", key)
                .header("Content-Type", "application/json")
                .send(envelope.to_string())
                .unwrap();
            (res.status().as_u16(), res.body_mut().read_json().unwrap_or(Value::Null))
        }

        /// The events under `id`, once one of `last` is among them.
        fn events_until(&self, key: &str, id: &str, last: &[&str]) -> Vec<Value> {
            for _ in 0..100 {
                let body: Value = self
                    .agent
                    .get(format!("{}/events?correlationId={id}", self.base))
                    .header("X-Api-Key", key)
                    .call()
                    .unwrap()
                    .body_mut()
                    .read_json()
                    .unwrap();
                let events = body["events"].as_array().cloned().unwrap_or_default();
                if events.iter().any(|e| last.iter().any(|l| e["type"] == *l)) {
                    return events;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("none of {last:?} under {id}");
        }
    }

    /// A door over `parts`, served in this test, and the test's `body` run against it off the runtime.
    async fn served(parts: Parts, envelope: Option<Envelope>, body: impl FnOnce(Served) + Send + 'static) {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen = listener.local_addr().unwrap();
        let settings = Settings {
            listen,
            endpoint: format!("http://{listen}"),
            data: dir.path().to_path_buf(),
            robot: RobotKind::Fake,
            envelope: None,
        };
        let (opened, owner) = door::build(&settings, parts, envelope).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let served = tokio::spawn(door::serve(listener, opened, owner, async {
            stopped.await.ok();
        }));
        let base = settings.endpoint.clone();
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .new_agent();
        tokio::task::spawn_blocking(move || body(Served { settings, base, agent }))
            .await
            .unwrap();
        stop.send(()).unwrap();
        served.await.unwrap().unwrap();
    }

    fn skill(name: &str) -> SkillSpec {
        SkillSpec {
            name: name.into(),
            description: "Goes.".into(),
            goal: json!({ "type": "object" }),
            feedback: json!({ "type": "object" }),
            result: json!({ "type": "object" }),
            target: Some(Target {
                frame: "/frame".into(),
                x: "/x".into(),
                y: "/y".into(),
            }),
        }
    }

    fn skills(fake: FakeSkills) -> Parts {
        Parts {
            state: None,
            skills: Some(Arc::new(fake)),
            topics: None,
        }
    }

    /// The whole sign-in: an agent starts the device flow, the person approves its code, and the agent's next poll
    /// redeems a key that opens the robot's state.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_person_admits_the_agent_and_its_key_reads_the_robot() {
        let parts = Parts {
            state: Some(Arc::new(robot::Fake::default())),
            skills: None,
            topics: None,
        };
        served(parts, None, |s| {
            let key = s.sign_in("test", Grant::default());
            let state: Value = s
                .agent
                .get(format!("{}/queries/get-robot-state", s.base))
                .header("X-Api-Key", &key)
                .call()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert_eq!(state["name"], "fake", "the key did not read the robot: {state}");
            assert!(approve(&s.settings, "NOPE-NOPE", Grant::default())
                .unwrap_err()
                .contains("refused"));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_key_approved_for_one_skill_is_refused_every_other() {
        let fake = FakeSkills::new(vec![skill("go"), skill("dock")]).ends("go", vec![], Outcome::Succeeded(json!({})));
        served(skills(fake), None, |s| {
            let key = s.sign_in(
                "scoped",
                Grant {
                    may: Some("go"),
                    expires_in: Some(3600),
                },
            );
            let place = json!({ "frame": "map", "x": 1, "y": 1 });
            assert_eq!(s.send(&key, "go", "Go", "g1", place.clone()).0, 201);
            let (status, body) = s.send(&key, "dock", "Dock", "d1", place);
            assert_eq!(status, 403, "{body}");
            assert!(body.to_string().contains("--may dock"), "{body}");
            let other = s.device("other");
            let refused = approve(
                &s.settings,
                other["user_code"].as_str().unwrap(),
                Grant {
                    may: Some("fly"),
                    expires_in: None,
                },
            )
            .unwrap_err();
            assert!(refused.contains("no such scope fly"), "{refused}");
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hold_now_stops_the_run_telling_its_holder_and_release_takes_commands_again() {
        let envelope = Envelope::parse("frame = \"map\"\ngeofence = [[-2, -2], [2, -2], [2, 2], [-2, 2]]\n").unwrap();
        served(
            skills(FakeSkills::new(vec![skill("go")]).held("go", vec![json!({ "left": 1 })])),
            Some(envelope),
            |s| {
                let key = s.sign_in("test", Grant::default());
                let place = json!({ "frame": "map", "x": 1, "y": 1 });
                let (status, body) = s.send(&key, "go", "Go", "far", json!({ "frame": "map", "x": 5, "y": 0 }));
                assert_eq!(
                    (status, body["error"]["code"].as_str()),
                    (400, Some("OUTSIDE_GEOFENCE")),
                    "{body}"
                );
                assert_eq!(s.send(&key, "go", "Go", "run-1", place.clone()).0, 201);
                s.events_until(&key, "run-1", &["GoProgressed"]);
                let said = hold(&s.settings, true, Some("servicing the arm")).unwrap();
                assert!(said.contains("has ended"), "{said}");
                let events = s.events_until(&key, "run-1", &["GoStopped"]);
                let reason = events.last().unwrap()["data"]["reason"].as_str().unwrap().to_string();
                assert!(reason.contains("servicing the arm"), "{reason}");
                let (status, body) = s.send(&key, "go", "Go", "while-held", place.clone());
                assert_eq!((status, body["error"]["code"].as_str()), (409, Some("HELD")), "{body}");
                assert_eq!(release(&s.settings).unwrap(), "released: skills start again");
                assert_eq!(s.send(&key, "go", "Go", "after", place).0, 201);
            },
        )
        .await;
    }
}
