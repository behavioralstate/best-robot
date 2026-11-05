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
use robot::{place, Outcome, Progress, Refusal, SkillSpec, Skills, Topics, Unreachable};
use serde_json::{json, Value};

use envelope::Envelope;
use hold::Hold;

/// How often what the robot reports is read while a skill runs.
const WATCH_EVERY: Duration = Duration::from_millis(200);
/// How old the robot's last word on its speed may be while a skill runs, before the ceiling counts as not held.
const SPEED_WITHIN: Duration = Duration::from_secs(2);

pub struct Guard {
    inner: Arc<dyn Skills>,
    envelope: Option<Envelope>,
    topics: Option<Arc<dyn Topics>>,
    hold: Arc<Hold>,
    /// The runs in flight, each with why the guard stopped it, once it did.
    runs: Mutex<HashMap<String, Option<String>>>,
}

impl Guard {
    /// The guard over `inner`, or why the envelope cannot be held: it reads a topic the robot does not offer.
    pub fn new(
        inner: Arc<dyn Skills>,
        envelope: Option<Envelope>,
        topics: Option<Arc<dyn Topics>>,
        hold: Arc<Hold>,
    ) -> Result<Self, String> {
        if let Some(e) = &envelope {
            for wanted in e.topics() {
                let offered = topics
                    .as_ref()
                    .is_some_and(|t| t.topics().iter().any(|spec| spec.name == wanted));
                if !offered {
                    return Err(format!(
                        "the envelope reads topic {wanted:?}, which the robot does not offer: list it among its topics"
                    ));
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

    /// Stops every run in flight for `reason`, and says, by run, whether the robot took the cancel.
    pub async fn stop_all(&self, reason: &str) -> Vec<(String, Result<bool, Unreachable>)> {
        let runs: Vec<String> = {
            let mut runs = self.runs.lock().unwrap();
            for why in runs.values_mut() {
                why.get_or_insert_with(|| reason.to_string());
            }
            runs.keys().cloned().collect()
        };
        let mut stopped = Vec::new();
        for run in runs {
            let took = self.inner.cancel(&run).await;
            stopped.push((run, took));
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

    /// Why what the robot reports breaks the envelope, if it does.
    fn breach(&self) -> Option<String> {
        let (e, topics) = (self.envelope.as_ref()?, self.topics.as_ref()?);
        let number = |m: &Value, at: &str| m.pointer(at).and_then(Value::as_f64);
        if let Some(p) = &e.position {
            if let Some(s) = topics.latest(&p.topic) {
                if let (Some(x), Some(y)) = (number(&s.message, &p.x), number(&s.message, &p.y)) {
                    if let Err(r) = e.within(x, y) {
                        return Some(format!(
                            "the robot reported it is at x {x:.2}, y {y:.2}, which leaves the bounds its owner set ({})",
                            r.code
                        ));
                    }
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
            let v = number(&s.message, &sp.x)
                .unwrap_or(0.0)
                .hypot(number(&s.message, &sp.y).unwrap_or(0.0));
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
        let reads = self.envelope.as_ref().is_some_and(|e| !e.topics().is_empty());
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
        // Again here, before the goal is sent: the hold or the envelope may have changed since it was accepted.
        if let Err(r) = self.admits(skill, &goal) {
            return Ok(Outcome::Refused(r.message));
        }
        self.runs.lock().unwrap().insert(run.into(), None);
        let mut running = std::pin::pin!(self.inner.run(run, skill, goal, progress));
        let end = tokio::select! {
            end = &mut running => end,
            why = self.watch() => {
                self.runs.lock().unwrap().get_mut(run).map(|w| w.get_or_insert(why));
                if let Err(e) = self.inner.cancel(run).await {
                    tracing::warn!(run, error = %e, "the robot did not take the cancel of a run that left its envelope");
                }
                running.await
            }
        };
        let stopped = self.runs.lock().unwrap().remove(run).flatten();
        match (end, stopped) {
            (Ok(Outcome::Canceled(result)), Some(reason)) => Ok(Outcome::Stopped { reason, result }),
            (end, _) => end,
        }
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
        hold.hold("the owner is servicing it", "2026-11-05T00:00:00Z").unwrap();
        let refused = g.check("go", &goal(1.0, 1.0)).unwrap_err();
        assert!(refused.now && refused.code == "HELD", "{refused:?}");
        let mut stopped = vec![];
        while stopped.is_empty() || !matches!(stopped[0], (_, Ok(true))) {
            stopped = g.stop_all("the owner took over").await;
            tokio::task::yield_now().await;
        }
        let end = running.await.unwrap().unwrap();
        assert_eq!(
            end,
            Outcome::Stopped {
                reason: "the owner took over".into(),
                result: Value::Null
            }
        );
    }

    #[tokio::test]
    async fn a_run_whose_robot_reports_it_too_fast_or_not_at_all_is_stopped_saying_so() {
        let envelope = format!("{ENVELOPE}[speed]\nmax = 0.3\ntopic = \"odom\"\nx = \"/vx\"\ny = \"/vy\"\n");
        let odom = TopicSpec {
            name: "odom".into(),
            description: "Odometry.".into(),
            message: json!({ "type": "object" }),
        };
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
        let pose = TopicSpec {
            name: "pose".into(),
            description: "Where it is.".into(),
            message: json!({ "type": "object" }),
        };
        let topics = FakeTopics::new(vec![pose]).with("pose", json!({ "x": 0.1, "y": 0.0 }));
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
}
