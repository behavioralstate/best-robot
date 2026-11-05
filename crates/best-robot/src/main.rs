//! best-robot: a BEST door in front of a robot. This file is the composition root: it reads the settings, chooses
//! the adapters for the robot's parts and hands them to the door; nothing below it knows which robot it is.

mod approver;
mod cli;
mod door;
mod guard;
mod owner;
mod settings;
mod surface;
#[cfg(test)]
mod testing;

use std::sync::Arc;

use settings::{RobotKind, Settings};
use surface::Parts;

const USAGE: &str = "usage: best-robot [serve] | approve <code> [--may <skills>] [--expires <span>] | hold [--now] [--reason <text>] | release | --version

  serve            serve the door (the default)
  approve <code>   admit the agent that showed you this code: for every skill, or those --may names (comma-separated),
                   until --expires ends its key (90s, 30m, 2h, 7d)
  hold             take the robot over: no skill starts until release; --now stops the runs in flight too, each told
                   to its holder; --reason says why, to them
  release          give the robot back

settings, from the environment:
  BEST_ROBOT_LISTEN    where the door listens, on loopback (default 127.0.0.1:7780)
  BEST_ROBOT_ENDPOINT  the base URL agents reach it at (default http://<listen>)
  BEST_ROBOT_DATA      where it keeps its credentials and event log (default ~/.best-robot)
  BEST_ROBOT_ROBOT     the robot behind it: fake (default) or rosbridge
  BEST_ROBOT_CONFIG    with rosbridge: the file naming the rosbridge and the skills and topics to expose
  BEST_ROBOT_ENVELOPE  the owner's bounds on the skills: a geofence, forbidden zones and a speed ceiling";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(settings()),
        Some("approve") => {
            let code = args
                .get(1)
                .filter(|c| !c.starts_with("--"))
                .unwrap_or_else(|| fail(USAGE));
            let o = options(&args[2..], &["--may", "--expires"], &[]);
            let expires_in = o
                .value("--expires")
                .map(|e| cli::seconds(e).unwrap_or_else(|why| fail(&why)));
            done(cli::approve(
                &settings(),
                code,
                cli::Grant {
                    may: o.value("--may"),
                    expires_in,
                },
            ));
        }
        Some("hold") => {
            let o = options(&args[1..], &["--reason"], &["--now"]);
            done(cli::hold(&settings(), o.has("--now"), o.value("--reason")));
        }
        Some("release") => {
            options(&args[1..], &[], &[]);
            done(cli::release(&settings()))
        }
        Some("--version" | "-V") => println!("best-robot {}", env!("CARGO_PKG_VERSION")),
        Some("--help" | "-h" | "help") => println!("{USAGE}"),
        Some(other) => fail(&format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

/// The flags an owner's command was given, or the end of it, naming what it does not take: a command that weakens
/// what the owner typed (a key for every skill, a hold that stops nothing) must never run on a mistyped flag.
fn options<'a>(args: &'a [String], valued: &[&str], bare: &[&str]) -> Options<'a> {
    parse(args, valued, bare).unwrap_or_else(|why| fail(&format!("{why}\n\n{USAGE}")))
}

#[derive(Debug, Default, PartialEq)]
struct Options<'a>(Vec<(&'a str, Option<&'a str>)>);

impl<'a> Options<'a> {
    fn value(&self, name: &str) -> Option<&'a str> {
        self.0.iter().find(|(n, _)| *n == name).and_then(|(_, v)| *v)
    }

    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| *n == name)
    }
}

fn parse<'a>(args: &'a [String], valued: &[&str], bare: &[&str]) -> Result<Options<'a>, String> {
    let mut o = Options::default();
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        let a = a.as_str();
        if o.has(a) {
            return Err(format!("{a} is given twice"));
        }
        if valued.contains(&a) {
            match rest.next() {
                Some(v) if !v.starts_with("--") => o.0.push((a, Some(v))),
                _ => return Err(format!("{a} needs a value")),
            }
        } else if bare.contains(&a) {
            o.0.push((a, None));
        } else {
            return Err(format!("{a:?} is not something this command takes"));
        }
    }
    Ok(o)
}

fn done(said: Result<String, String>) {
    match said {
        Ok(line) => println!("{line}"),
        Err(why) => fail(&why),
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
        let envelope = settings.envelope.as_ref().map(|path| {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| fail(&format!("cannot read BEST_ROBOT_ENVELOPE={}: {e}", path.display())));
            guard::envelope::Envelope::parse(&text)
                .unwrap_or_else(|why| fail(&format!("BEST_ROBOT_ENVELOPE={}: {why}", path.display())))
        });
        let parts = parts(&settings.robot).await.unwrap_or_else(|why| fail(&why));
        let (door, owner) = door::build(&settings, parts, envelope).unwrap_or_else(|why| fail(&why));
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
        door::serve(listener, door, owner, stop)
            .await
            .unwrap_or_else(|e| fail(&format!("the door stopped: {e}")));
    });
}

fn fail(why: &str) -> ! {
    eprintln!("best-robot: {why}");
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_owners_command_refuses_what_it_does_not_take() {
        let given = args(&["--may", "go", "--now"]);
        let o = parse(&given, &["--may"], &["--now"]).unwrap();
        assert_eq!((o.value("--may"), o.has("--now")), (Some("go"), true));
        for wrong in [
            &["--may=go"][..],
            &["--noww"],
            &["now"],
            &["--may"],
            &["--may", "--now"],
            &["--now", "--now"],
        ] {
            assert!(parse(&args(wrong), &["--may"], &["--now"]).is_err(), "{wrong:?}");
        }
    }
}
