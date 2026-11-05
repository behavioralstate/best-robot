//! best-robot: a BEST door in front of a robot. This file is the composition root: it reads the settings, chooses
//! the robot adapter and hands it to the door; nothing below it knows which robot it is.

mod approve;
mod approver;
mod door;
mod settings;
mod surface;

use std::sync::Arc;

use robot::Robot;
use settings::{RobotKind, Settings};

const USAGE: &str = "usage: best-robot [serve] | best-robot approve <code> | best-robot --version

  serve            serve the door (the default)
  approve <code>   admit the agent that showed you this code

settings, from the environment:
  BEST_ROBOT_LISTEN    where the door listens, on loopback (default 127.0.0.1:7780)
  BEST_ROBOT_ENDPOINT  the base URL agents reach it at (default http://<listen>)
  BEST_ROBOT_DATA      where it keeps its credentials and event log (default ~/.best-robot)
  BEST_ROBOT_ROBOT     the robot behind it: fake (default)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(settings()),
        Some("approve") => match args.get(1) {
            Some(code) => match approve::approve(&settings(), code) {
                Ok(line) => println!("{line}"),
                Err(why) => fail(&why),
            },
            None => fail(USAGE),
        },
        Some("--version" | "-V") => println!("best-robot {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h" | "help") => println!("{USAGE}"),
        Some(other) => fail(&format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

fn settings() -> Settings {
    #[allow(deprecated)]
    let home = std::env::home_dir();
    Settings::read(|k| std::env::var(k).ok(), home).unwrap_or_else(|why| fail(&why))
}

fn robot(kind: RobotKind) -> Arc<dyn Robot> {
    match kind {
        RobotKind::Fake => Arc::new(robot::Fake::default()),
    }
}

fn serve(settings: Settings) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|e| fail(&format!("cannot start: {e}")));
    runtime.block_on(async {
        let door = door::open(&settings, robot(settings.robot)).unwrap_or_else(|e| {
            fail(&format!(
                "cannot keep the door's data in {}: {e}",
                settings.data.display()
            ))
        });
        let listener = tokio::net::TcpListener::bind(settings.listen)
            .await
            .unwrap_or_else(|e| fail(&format!("cannot listen on {}: {e}", settings.listen)));
        println!(
            "best-robot is serving its door at {}/.well-known/best, its data in {}",
            settings.endpoint,
            settings.data.display()
        );
        let stop = async {
            tokio::signal::ctrl_c().await.ok();
        };
        door::serve(listener, door, stop)
            .await
            .unwrap_or_else(|e| fail(&format!("the door stopped: {e}")));
    });
}

fn fail(why: &str) -> ! {
    eprintln!("best-robot: {why}");
    std::process::exit(1)
}
