//! Against the ROS 2 testbed (`testbed/rosbridge`), which the gate's rosbridge lane starts: `cargo test -p rosbridge
//! -- --ignored`, with `BEST_ROBOT_TEST_ROSBRIDGE` its address (ws://127.0.0.1:9091 when not said).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use robot::{Outcome, Skills, Topics};
use rosbridge::Rosbridge;

mod common;
use common::{config, testbed};
use serde_json::{json, Value};

async fn run(rb: &Rosbridge, run: &str, order: i64) -> (Outcome, Vec<Value>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let outcome = rb
        .run(
            run,
            "fibonacci",
            json!({ "order": order }),
            Box::new(move |f| sink.lock().unwrap().push(f)),
        )
        .await
        .unwrap();
    let feedback = seen.lock().unwrap().clone();
    (outcome, feedback)
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed"]
async fn the_skill_and_topic_schemas_come_from_rosapi() {
    let rb = testbed().await;
    let skill = &rb.skills()[0];
    assert_eq!(skill.goal["properties"]["order"]["type"], "integer");
    assert_eq!(skill.result["properties"]["sequence"]["type"], "array");
    assert_eq!(skill.feedback["properties"]["sequence"]["items"]["type"], "integer");
    let topic = &rb.topics()[0];
    assert_eq!(
        topic.message["properties"]["percentage"]["type"],
        json!(["number", "null"])
    );
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed"]
async fn a_goal_runs_to_its_result_with_its_feedback() {
    let rb = testbed().await;
    let (outcome, feedback) = run(&rb, "ok", 4).await;
    assert_eq!(outcome, Outcome::Succeeded(json!({ "sequence": [0, 1, 1, 2, 3] })));
    assert_eq!(feedback.last().unwrap(), &json!({ "sequence": [0, 1, 1, 2, 3] }));
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed"]
async fn a_rejected_goal_is_refused_and_an_aborted_one_aborted() {
    let rb = testbed().await;
    assert!(matches!(run(&rb, "rejected", 99).await.0, Outcome::Refused(_)));
    assert!(matches!(run(&rb, "aborted", 0).await.0, Outcome::Aborted(_)));
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed"]
async fn a_cancel_ends_the_run_cancelled() {
    let rb = testbed().await;
    let running = {
        let rb = rb.clone();
        tokio::spawn(async move { run(&rb, "long", 30).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(rb.cancel("long").await.unwrap());
    let (outcome, _) = running.await.unwrap();
    assert!(matches!(outcome, Outcome::Canceled(_)), "{outcome:?}");
    assert!(!rb.cancel("long").await.unwrap(), "the run is over");
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed"]
async fn a_topic_keeps_its_latest_message() {
    let rb = testbed().await;
    for _ in 0..40 {
        if let Some(s) = rb.latest("battery") {
            let pct = s.message["percentage"].as_f64().unwrap();
            assert!((pct - 0.87).abs() < 1e-6, "{pct}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no battery message within four seconds");
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed"]
async fn a_type_rosapi_does_not_know_stops_the_start_naming_it() {
    let e = Rosbridge::connect(config("no_pkg/action/Nope")).await.err().unwrap();
    assert!(e.to_string().contains("no_pkg/action/Nope"), "{e}");
}
