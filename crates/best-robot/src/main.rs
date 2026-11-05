//! best-robot: a BEST door in front of a robot. This file is the composition root: it reads the settings, chooses
//! the adapters for the robot's parts and hands them to the door; nothing below it knows which robot it is.

mod approve;
mod approver;
mod door;
mod settings;
mod surface;
#[cfg(test)]
mod testing;

use std::sync::Arc;

use settings::{RobotKind, Settings};
use surface::Parts;

const USAGE: &str = "usage: best-robot [serve] | best-robot approve <code> | best-robot --version

  serve            serve the door (the default)
  approve <code>   admit the agent that showed you this code

settings, from the environment:
  BEST_ROBOT_LISTEN    where the door listens, on loopback (default 127.0.0.1:7780)
  BEST_ROBOT_ENDPOINT  the base URL agents reach it at (default http://<listen>)
  BEST_ROBOT_DATA      where it keeps its credentials and event log (default ~/.best-robot)
  BEST_ROBOT_ROBOT     the robot behind it: fake (default) or rosbridge
  BEST_ROBOT_CONFIG    with rosbridge: the file naming the rosbridge and the skills and topics to expose";

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

/// The robot's parts, each from the adapter that has it.
async fn parts(kind: &RobotKind) -> Result<Parts, String> {
    Ok(match kind {
        RobotKind::Fake => Parts {
            state: Some(Arc::new(robot::Fake::default())),
            skills: None,
            topics: None,
        },
        RobotKind::Rosbridge { config } => {
            let text = std::fs::read_to_string(config)
                .map_err(|e| format!("cannot read BEST_ROBOT_CONFIG={}: {e}", config.display()))?;
            let config = rosbridge::Config::parse(&text)
                .map_err(|why| format!("BEST_ROBOT_CONFIG={}: {why}", config.display()))?;
            let bridge = Arc::new(rosbridge::Rosbridge::connect(config).await.map_err(|e| e.to_string())?);
            Parts {
                state: None,
                skills: Some(bridge.clone()),
                topics: Some(bridge),
            }
        }
    })
}

fn serve(settings: Settings) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let runtime = tokio::runtime::Runtime::new().unwrap_or_else(|e| fail(&format!("cannot start: {e}")));
    runtime.block_on(async {
        let parts = parts(&settings.robot).await.unwrap_or_else(|why| fail(&why));
        let catalogue = surface::catalogue(parts).unwrap_or_else(|why| fail(&why));
        let door = door::open(&settings, catalogue).unwrap_or_else(|e| {
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
