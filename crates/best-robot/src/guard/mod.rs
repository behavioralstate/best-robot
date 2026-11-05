//! The owner's say over the robot's skills: a decorator in front of them. It holds every goal to the hold and the
//! envelope, before its command is accepted and again before it is sent, so bounds the agent cannot see are kept
//! whatever it sent. While a skill runs, it watches what the robot reports, and stops a run that leaves the envelope
//! or that the owner takes over, telling its holder why. The robot's own controllers and its emergency stop stay the
//! last line: this guard only refuses and cancels.

pub mod envelope;
pub mod hold;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use robot::{place, schema, Outcome, Progress, Refusal, SkillSpec, Skills, Topics, Unreachable};
use serde_json::{json, Value};
use tokio::sync::{watch, Notify};

use envelope::Envelope;
use hold::Hold;

/// How often what the robot reports is read while a skill runs.
const WATCH_EVERY: Duration = Duration::from_millis(200);
/// How old the robot's last word on its speed may be while a skill runs, before the ceiling counts as not held.
const SPEED_WITHIN: Duration = Duration::from_secs(2);
/// How long a stopped run's cancel is given before it is sent again: the robot may not have taken the goal yet, or
/// the cancel may have been lost.
const CANCEL_AGAIN: Duration = Duration::from_millis(500);
/// How long `stop_all` waits for the runs it stopped to end, before it says which still run.
const STOP_WITHIN: Duration = Duration::from_secs(5);

pub struct Guard {
    inner: Arc<dyn Skills>,
    envelope: Option<Envelope>,
    topics: Option<Arc<dyn Topics>>,
    hold: Arc<Hold>,
    /// The runs in flight, listed before they are admitted, so a hold finds every run it does not refuse.
    runs: Mutex<HashMap<String, Arc<Run>>>,
}

/// A run in flight, as the guard sees it.
struct Run {
    /// Why the guard stopped it, once it did; the first reason stands.
    why: Mutex<Option<String>>,
    stop: Notify,
    ended: watch::Sender<bool>,
}

impl Run {
    fn new() -> Self {
        Self {
            why: Mutex::new(None),
            stop: Notify::new(),
            ended: watch::channel(false).0,
        }
    }

    /// Stops the run for `reason`, or for the reason it was stopped for already, and says which.
    fn stop_for(&self, reason: &str) -> String {
        let why = self.why.lock().unwrap().get_or_insert_with(|| reason.into()).clone();
        self.stop.notify_one();
        why
    }
}

impl Guard {
    /// The guard over `inner`, or why the envelope cannot be held: it reads a topic the robot does not offer, or a
    /// field that topic's message has no number at.
    pub fn new(
        inner: Arc<dyn Skills>,
        envelope: Option<Envelope>,
        topics: Option<Arc<dyn Topics>>,
        hold: Arc<Hold>,
    ) -> Result<Self, String> {
        if let Some(e) = &envelope {
            for (what, topic, pointers) in e.readings() {
                let offered = topics
                    .as_ref()
                    .and_then(|t| t.topics().iter().find(|spec| spec.name == topic).cloned());
                let Some(spec) = offered else {
                    return Err(format!(
                        "the envelope's {what} reads topic {topic:?}, which the robot does not offer: list it among its topics"
                    ));
                };
                for p in pointers {
                    if !schema::allows(schema::at(&spec.message, p), "number") {
                        return Err(format!(
                            "the envelope's {what} reads {p} on topic {topic:?}, and its message has no number there: point at the field that carries it"
                        ));
                    }
                }
            }
        }
        Ok(Self {
            inner,
            envelope,
            topics,
            hold,
            runs: Mutex::new(HashMap::new()),
        })
    }

    /// Stops every run in flight for `reason`, and says, by run, whether it ended within [`STOP_WITHIN`]. One that has
    /// not is still being cancelled.
    pub async fn stop_all(&self, reason: &str) -> Vec<(String, bool)> {
        let runs: Vec<(String, Arc<Run>)> = {
            let runs = self.runs.lock().unwrap();
            runs.iter().map(|(id, r)| (id.clone(), r.clone())).collect()
        };
        for (_, r) in &runs {
            r.stop_for(reason);
        }
        let mut stopped = Vec::new();
        for (id, r) in runs {
            let mut ended = r.ended.subscribe();
            let done = tokio::time::timeout(STOP_WITHIN, ended.wait_for(|e| *e)).await.is_ok();
            stopped.push((id, done));
        }
        stopped
    }

    fn spec(&self, skill: &str) -> Option<&SkillSpec> {
        self.inner.skills().iter().find(|s| s.name == skill)
    }

    /// Whether `goal` may be sent now: the robot is not held, and the place it names is inside the envelope.
    fn admits(&self, skill: &str, goal: &Value) -> Result<(), Refusal> {
        if let Some(h) = self.hold.current() {
            return Err(Refusal {
                code: "HELD",
                message: format!(
                    "a person has taken this robot over since {} ({}): no skill starts until they release it; send the goal again once they have",
                    h.since, h.reason
                ),
                details: json!({ "reason": h.reason, "since": h.since }),
                now: true,
            });
        }
        if let (Some(e), Some(target)) = (&self.envelope, self.spec(skill).and_then(|s| s.target.as_ref())) {
            let (frame, x, y) = place(goal, target).map_err(|why| Refusal {
                code: "NO_PLACE",
                message: format!("{why}: send the place the robot is to go, so its owner's bounds can be checked"),
                details: json!({ "frame": target.frame, "x": target.x, "y": target.y }),
                now: false,
            })?;
            e.place(&frame, x, y)?;
        }
        self.inner.check(skill, goal)
    }

    /// Why what the robot reports breaks the envelope, if it does. A reading without its number (NaN, or a message
    /// without the field) breaks it too: the bound cannot be held on it.
    fn breach(&self) -> Option<String> {
        let (e, topics) = (self.envelope.as_ref()?, self.topics.as_ref()?);
        let number = |m: &Value, at: &str, topic: &str| {
            m.pointer(at).and_then(Value::as_f64).ok_or_else(|| {
                format!("the robot reported no number at {at} on {topic}, so the bounds its owner set cannot be held")
            })
        };
        if let Some(p) = &e.position {
            if let Some(s) = topics.latest(&p.topic) {
                let at = number(&s.message, &p.x, &p.topic).and_then(|x| Ok((x, number(&s.message, &p.y, &p.topic)?)));
                let (x, y) = match at {
                    Ok(at) => at,
                    Err(why) => return Some(why),
                };
                if let Err(r) = e.within(x, y) {
                    return Some(format!(
                        "the robot reported it is at x {x:.2}, y {y:.2}, which leaves the bounds its owner set ({})",
                        r.code
                    ));
                }
            }
        }
        if let Some(sp) = &e.speed {
            let fresh = topics
                .latest(&sp.topic)
                .filter(|s| s.received_at.elapsed().is_ok_and(|age| age <= SPEED_WITHIN));
            let Some(s) = fresh else {
                return Some(format!(
                    "the robot has not reported its speed for {} s, so the {} m/s its owner allows cannot be held",
                    SPEED_WITHIN.as_secs(),
                    sp.max
                ));
            };
            let v = match (
                number(&s.message, &sp.x, &sp.topic),
                number(&s.message, &sp.y, &sp.topic),
            ) {
                (Ok(x), Ok(y)) => x.hypot(y),
                (Err(why), _) | (_, Err(why)) => return Some(why),
            };
            if v > sp.max {
                return Some(format!(
                    "the robot reported moving at {v:.2} m/s, above the {} m/s its owner allows",
                    sp.max
                ));
            }
        }
        None
    }

    /// Resolves with the first breach, read every [`WATCH_EVERY`]; never, when the envelope reads nothing.
    async fn watch(&self) -> String {
        let reads = self.envelope.as_ref().is_some_and(|e| !e.readings().is_empty());
        if !reads {
            return std::future::pending().await;
        }
        loop {
            tokio::time::sleep(WATCH_EVERY).await;
            if let Some(why) = self.breach() {
                return why;
            }
        }
    }

    async fn guarded(
        &self,
        run: &str,
        skill: &str,
        goal: Value,
        progress: Progress,
        me: &Run,
    ) -> Result<Outcome, Unreachable> {
        // Admitted again here, before the goal is sent and after the run is listed: a hold taken before this refuses
        // the run, and one taken after it finds the run to stop.
        if let Err(r) = self.admits(skill, &goal) {
            return Ok(Outcome::Refused(r.message));
        }
        let mut running = std::pin::pin!(self.inner.run(run, skill, goal, progress));
        let breach = tokio::select! {
            end = &mut running => return end,
            why = self.watch() => Some(why),
            () = me.stop.notified() => None,
        };
        let reason = me.stop_for(&breach.unwrap_or_default());
        tracing::warn!(run, reason = %reason, "stopping a run");
        // Cancelled until the robot ends it: a cancel sent before the robot took the goal, or one lost, is sent again.
        let end = loop {
            if let Err(e) = self.inner.cancel(run).await {
                tracing::warn!(run, error = %e, "the robot did not take the cancel of a stopped run; sending it again");
            }
            tokio::select! {
                end = &mut running => break end,
                () = tokio::time::sleep(CANCEL_AGAIN) => {}
            }
        };
        match end {
            Ok(Outcome::Canceled(result)) => Ok(Outcome::Stopped { reason, result }),
            end => end,
        }
    }
}

#[async_trait]
impl Skills for Guard {
    fn skills(&self) -> &[SkillSpec] {
        self.inner.skills()
    }

    fn check(&self, skill: &str, goal: &Value) -> Result<(), Refusal> {
        self.admits(skill, goal)
    }

    async fn run(&self, run: &str, skill: &str, goal: Value, progress: Progress) -> Result<Outcome, Unreachable> {
        let me = Arc::new(Run::new());
        self.runs.lock().unwrap().insert(run.into(), me.clone());
        let end = self.guarded(run, skill, goal, progress, &me).await;
        self.runs.lock().unwrap().remove(run);
        me.ended.send_replace(true);
        end
    }

    async fn cancel(&self, run: &str) -> Result<bool, Unreachable> {
        self.inner.cancel(run).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robot::{FakeSkills, FakeTopics, Target, TopicSpec};

    const ENVELOPE: &str = r#"
frame = "map"
geofence = [[-2, -2], [2, -2], [2, 2], [-2, 2]]
[[zones]]
name = "centre"
polygon = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]]
"#;

    fn go() -> SkillSpec {
        SkillSpec {
            name: "go".into(),
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

    fn goal(x: f64, y: f64) -> Value {
        json!({ "frame": "map", "x": x, "y": y })
    }

    fn guard(
        skills: FakeSkills,
        envelope: &str,
        topics: Option<FakeTopics>,
    ) -> (Guard, Arc<FakeSkills>, Arc<Hold>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let hold = Arc::new(Hold::open(dir.path()).unwrap());
        let skills = Arc::new(skills);
        let topics = topics.map(|t| Arc::new(t) as Arc<dyn Topics>);
        let g = Guard::new(
            skills.clone(),
            Some(Envelope::parse(envelope).unwrap()),
            topics,
            hold.clone(),
        )
        .unwrap();
        (g, skills, hold, dir)
    }

    #[tokio::test]
    async fn a_goal_outside_the_envelope_is_refused_and_never_reaches_the_robot() {
        let fake = FakeSkills::new(vec![go()]).ends("go", vec![], Outcome::Succeeded(json!({})));
        let (g, skills, _, _dir) = guard(fake, ENVELOPE, None);
        assert_eq!(g.check("go", &goal(5.0, 0.0)).unwrap_err().code, "OUTSIDE_GEOFENCE");
        assert_eq!(g.check("go", &goal(0.0, 0.0)).unwrap_err().code, "IN_FORBIDDEN_ZONE");
        assert_eq!(g.check("go", &json!({ "x": 1 })).unwrap_err().code, "NO_PLACE");
        // Sent anyway, as a goal accepted before the bounds moved would be: refused again, never sent.
        let end = g.run("r1", "go", goal(5.0, 0.0), Box::new(|_| {})).await.unwrap();
        assert!(
            matches!(end, Outcome::Refused(ref why) if why.contains("geofence")),
            "{end:?}"
        );
        assert!(skills.goals().is_empty());
        assert!(g.check("go", &goal(1.0, 1.0)).is_ok());
    }

    #[tokio::test]
    async fn a_held_robot_takes_no_goal_and_a_stop_tells_the_run_why() {
        let fake = FakeSkills::new(vec![go()]).held("go", vec![]);
        let (g, skills, hold, _dir) = guard(fake, ENVELOPE, None);
        let g = Arc::new(g);
        let running = {
            let g = g.clone();
            tokio::spawn(async move { g.run("r1", "go", goal(1.0, 1.0), Box::new(|_| {})).await })
        };
        while skills.goals().is_empty() {
            tokio::task::yield_now().await;
        }
        hold.hold("the owner is servicing it", "2026-11-05T00:00:00Z")
            .1
            .unwrap();
        let refused = g.check("go", &goal(1.0, 1.0)).unwrap_err();
        assert!(refused.now && refused.code == "HELD", "{refused:?}");
        assert_eq!(g.stop_all("the owner took over").await, vec![("r1".to_string(), true)]);
        let end = running.await.unwrap().unwrap();
        assert_eq!(
            end,
            Outcome::Stopped {
                reason: "the owner took over".into(),
                result: Value::Null
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hold_taken_while_a_run_starts_refuses_it_or_stops_it_never_neither() {
        for _ in 0..20 {
            let fake = FakeSkills::new(vec![go()]).held("go", vec![]);
            let (g, skills, hold, _dir) = guard(fake, ENVELOPE, None);
            let g = Arc::new(g);
            let running = {
                let g = g.clone();
                tokio::spawn(async move { g.run("r1", "go", goal(1.0, 1.0), Box::new(|_| {})).await })
            };
            hold.hold("taken over", "2026-11-05T00:00:00Z").1.unwrap();
            g.stop_all("taken over").await;
            let end = tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            match end {
                Outcome::Refused(why) => assert!(why.contains("taken over") && skills.goals().is_empty()),
                Outcome::Stopped { reason, .. } => assert_eq!(reason, "taken over"),
                other => panic!("{other:?}"),
            }
        }
    }

    fn numbers(name: &str, fields: &[&str]) -> TopicSpec {
        let properties: serde_json::Map<String, Value> = fields
            .iter()
            .map(|f| (f.to_string(), json!({ "type": ["number", "null"] })))
            .collect();
        TopicSpec {
            name: name.into(),
            description: "Numbers.".into(),
            message: json!({ "type": "object", "properties": properties }),
        }
    }

    #[tokio::test]
    async fn a_run_whose_robot_reports_it_too_fast_or_not_at_all_is_stopped_saying_so() {
        let envelope = format!("{ENVELOPE}[speed]\nmax = 0.3\ntopic = \"odom\"\nx = \"/vx\"\ny = \"/vy\"\n");
        let odom = numbers("odom", &["vx", "vy"]);
        let too_fast = FakeTopics::new(vec![odom.clone()]).live("odom", json!({ "vx": 0.3, "vy": 0.1 }));
        let (g, _, _, _dir) = guard(
            FakeSkills::new(vec![go()]).held("go", vec![]),
            &envelope,
            Some(too_fast),
        );
        let end = g.run("r1", "go", goal(1.0, 1.0), Box::new(|_| {})).await.unwrap();
        assert!(
            matches!(&end, Outcome::Stopped { reason, .. } if reason.contains("above the 0.3 m/s")),
            "{end:?}"
        );
        // A speed last heard long ago cannot hold the ceiling either.
        let silent = FakeTopics::new(vec![odom]).with("odom", json!({ "vx": 0.0, "vy": 0.0 }));
        let (g, _, _, _dir) = guard(FakeSkills::new(vec![go()]).held("go", vec![]), &envelope, Some(silent));
        let end = g.run("r1", "go", goal(1.0, 1.0), Box::new(|_| {})).await.unwrap();
        assert!(
            matches!(&end, Outcome::Stopped { reason, .. } if reason.contains("not reported its speed")),
            "{end:?}"
        );
        // A speed reported as NaN (null) cannot hold the ceiling either.
        let nan = FakeTopics::new(vec![numbers("odom", &["vx", "vy"])]).live("odom", json!({ "vx": null, "vy": 0.0 }));
        let (g, _, _, _dir) = guard(FakeSkills::new(vec![go()]).held("go", vec![]), &envelope, Some(nan));
        let end = g.run("r1", "go", goal(1.0, 1.0), Box::new(|_| {})).await.unwrap();
        assert!(
            matches!(&end, Outcome::Stopped { reason, .. } if reason.contains("no number at /vx")),
            "{end:?}"
        );
    }

    #[tokio::test]
    async fn a_run_whose_robot_reports_it_in_a_forbidden_zone_is_stopped_saying_so() {
        let envelope = format!(
            "{ENVELOPE}[position]
topic = \"pose\"
x = \"/x\"
y = \"/y\"
"
        );
        let topics = FakeTopics::new(vec![numbers("pose", &["x", "y"])]).with("pose", json!({ "x": 0.1, "y": 0.0 }));
        let (g, _, _, _dir) = guard(FakeSkills::new(vec![go()]).held("go", vec![]), &envelope, Some(topics));
        let end = g.run("r1", "go", goal(1.0, 1.0), Box::new(|_| {})).await.unwrap();
        assert!(
            matches!(&end, Outcome::Stopped { reason, .. } if reason.contains("IN_FORBIDDEN_ZONE")),
            "{end:?}"
        );
    }

    #[test]
    fn an_envelope_reading_a_topic_the_robot_lacks_stops_the_door() {
        let dir = tempfile::tempdir().unwrap();
        let envelope = format!("{ENVELOPE}[position]\ntopic = \"pose\"\nx = \"/x\"\ny = \"/y\"\n");
        let e = Guard::new(
            Arc::new(FakeSkills::new(vec![go()])),
            Some(Envelope::parse(&envelope).unwrap()),
            None,
            Arc::new(Hold::open(dir.path()).unwrap()),
        )
        .err()
        .unwrap();
        assert!(e.contains("pose"), "{e}");
    }

    #[test]
    fn an_envelope_reading_a_field_the_topic_lacks_stops_the_door() {
        let dir = tempfile::tempdir().unwrap();
        let envelope = format!("{ENVELOPE}[speed]\nmax = 1\ntopic = \"odom\"\nx = \"/twist/x\"\ny = \"/vy\"\n");
        let topics = Arc::new(FakeTopics::new(vec![numbers("odom", &["vx", "vy"])])) as Arc<dyn Topics>;
        let e = Guard::new(
            Arc::new(FakeSkills::new(vec![go()])),
            Some(Envelope::parse(&envelope).unwrap()),
            Some(topics),
            Arc::new(Hold::open(dir.path()).unwrap()),
        )
        .err()
        .unwrap();
        assert!(e.contains("/twist/x") && e.contains("no number"), "{e}");
    }
}
