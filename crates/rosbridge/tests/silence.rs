//! A rosbridge that goes silent without closing its socket, as a robot does when its power or cable is pulled: the
//! testbed container is paused, so its socket stays open and answers nothing. Its own test binary, so pausing it
//! never cuts the other testbed tests short. `BEST_ROBOT_TEST_CONTAINER` names the container; the gate's rosbridge
//! lane sets it.

use std::process::Command;
use std::time::{Duration, Instant};

use robot::{Outcome, Skills};
use serde_json::json;

mod common;
use common::testbed;

fn docker(verb: &str, container: &str) {
    let ok = Command::new("docker")
        .args([verb, container])
        .status()
        .unwrap()
        .success();
    assert!(ok, "docker {verb} {container} failed");
}

#[tokio::test]
#[ignore = "needs the ROS 2 testbed and the name of its container"]
async fn a_run_on_a_silent_rosbridge_ends_unreachable_and_the_bridge_reconnects_once_it_answers() {
    let container = std::env::var("BEST_ROBOT_TEST_CONTAINER").expect("BEST_ROBOT_TEST_CONTAINER names the testbed");
    let rb = testbed().await;
    let running = {
        let rb = rb.clone();
        tokio::spawn(async move {
            rb.run("cut", "fibonacci", json!({ "order": 30 }), Box::new(|_| {}))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    docker("pause", &container);
    let started = Instant::now();
    let ended = tokio::time::timeout(Duration::from_secs(15), running).await;
    // Still silent: a goal sent now never leaves the door, and says so.
    let unsent = rb
        .run("unsent", "fibonacci", json!({ "order": 2 }), Box::new(|_| {}))
        .await;
    docker("unpause", &container);
    let ended = ended
        .expect("the run was still waiting on a silent rosbridge after fifteen seconds")
        .unwrap();
    assert!(ended.is_err(), "{ended:?}");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(
        matches!(&unsent, Ok(Outcome::Refused(why)) if why.contains("not sent")),
        "{unsent:?}"
    );

    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        // Until it reconnects, a goal is refused unsent.
        match rb
            .run("after", "fibonacci", json!({ "order": 2 }), Box::new(|_| {}))
            .await
            .unwrap()
        {
            Outcome::Refused(why) => assert!(why.contains("not sent"), "{why}"),
            outcome => {
                assert_eq!(outcome, Outcome::Succeeded(json!({ "sequence": [0, 1, 1] })));
                return;
            }
        }
    }
    panic!("the bridge did not reconnect within ten seconds of rosbridge answering again");
}
