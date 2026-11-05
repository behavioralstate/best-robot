# best-robot

A [Behavioral State Protocol](https://behavioralstate.io) door in front of a robot's existing ROS 2 stack. An agent
that knows only the door's manifest reads what the robot is doing and, as skills are exposed, asks it to do them: a
skill is a command, its outcome an event. Nothing below the skill level is on the door. The robot's controllers keep
the real-time loops, and the emergency stop stays a physical button.

It sits next to [best-peer](https://github.com/behavioralstate/best-peer), which hosts the models a robot reasons
with, and is built on best-peer's door crate. The plan is the epic
[behavioralstate/spec#260](https://github.com/behavioralstate/spec/issues/260) and its slices, on the
[Behavioralstate board](https://github.com/orgs/Amarcode-Studios/projects/4). How the work is made and shipped is
[SDLC.md](SDLC.md).

**Where it stands:** the door serves a fake robot's state. There's no ROS bridge and no commands yet, and it serves
http on loopback only.

## Try it

```bash
cargo run -p best-robot
```

It prints `best-robot is serving its door at http://127.0.0.1:7780/.well-known/best` and waits. That URL is the
manifest, the one document an agent needs. An agent signs in by the device flow and shows you a code; at the same
machine, with the same settings:

```bash
best-robot approve 8BB9-CB44
```

Its next poll receives a key, and `get-robot-state` answers with the robot's name, status (`idle`, `moving`,
`fault`), pose in a named frame and charge.

| Variable | Default | What it is |
|---|---|---|
| `BEST_ROBOT_LISTEN` | `127.0.0.1:7780` | Where the door listens. Loopback only: off it the key would travel in the clear, and the door refuses to start. |
| `BEST_ROBOT_ENDPOINT` | `http://<listen>` | The base URL agents reach it at. An https proxy on this machine sets it to its own address. |
| `BEST_ROBOT_DATA` | `~/.best-robot` | The admin token, the credentials (hashes only) and the event log. |
| `BEST_ROBOT_ROBOT` | `fake` | The robot behind the door. `fake` answers with a fixed state and moves nothing. |

## Layout

| Crate | What it owns |
|---|---|
| `crates/robot` | The `Robot` abstraction the door depends on, its state types, and the fake. It knows nothing of JSON, HTTP or ROS. |
| `crates/best-robot` | The binary. `main.rs` is the composition root that chooses the robot adapter. `settings` reads the environment, `approver` admits on the machine's admin token, `surface` holds the catalogue and the wire shape next to its schema, `door` puts best-peer's door around them, and `approve` is the owner's command. |

## Gate

`scripts/gate.sh all` (Git Bash on Windows) ends in `GATE GREEN` or `GATE RED`. CI runs the same script.
