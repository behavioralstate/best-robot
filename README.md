# best-robot

A [Behavioral State Protocol](https://behavioralstate.io) door in front of a robot's existing ROS 2 stack. An agent
that knows only the door's manifest reads what the robot is doing and asks it to do what it can: a skill is a
command, its run a series of events. Nothing below the skill level is on the door. The robot's controllers keep the
real-time loops, and the emergency stop stays a physical button.

It sits next to [best-peer](https://github.com/behavioralstate/best-peer), which hosts the models a robot reasons
with, and is built on best-peer's door crate. The plan is the epic
[behavioralstate/spec#260](https://github.com/behavioralstate/spec/issues/260) and its slices, on the
[Behavioralstate board](https://github.com/orgs/Amarcode-Studios/projects/4). How the work is made and shipped is
[SDLC.md](SDLC.md).

**Where it stands:** the door serves a fake robot's state, or a ROS 2 robot's actions and topics through rosbridge,
proven against a ROS 2 Jazzy testbed, and drives a simulated TurtleBot3 under Nav2 behind its owner's envelope and
hold. It serves http on loopback only. The envelope is proven in simulation alone: try it on a real robot with the
robot's own limits and its emergency stop in place, since it bounds the goals and what the robot reports, not the
path the robot's planner takes.

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

Its next poll receives a key, good for every skill. `--may navigate-to,spin` limits it to those skills (a command for
any other is refused with 403 `FORBIDDEN`, naming the approve that would allow it), and `--expires 2h` ends it then
(`90s`, `30m`, `2h`, `7d`; a year at most). With the key, `get-robot-state` answers with the fake robot's name, status (`idle`, `moving`,
`fault`), pose in a named frame and charge. When the robot does not answer, the query is refused with 503
`ROBOT_UNREACHABLE`. When it reports a pose number or a charge that is not finite, or a charge outside 0 to 100,
the query is refused with 502 `ROBOT_STATE_INVALID`, naming the field. It never serves a value its schema forbids.

### A simulated robot

With Docker running (Git Bash on Windows):

```bash
scripts/sim.sh
```

It starts Gazebo with a TurtleBot3 and Nav2, headless, in a container (the first run builds a 4 GB image), then the
door in front of it on the same address as above; Ctrl-C stops both. An agent signed in as above finds:

- `navigate-to`, whose data is Nav2's goal, a pose in the map frame. The robot starts at x -2, y -0.5;
  `{ "pose": { "header": { "frame_id": "map" }, "pose": { "position": { "x": -0.55, "y": -0.55 }, "orientation": { "w": 1 } } } }`
  drives it between the pillars. Its run is `NavigateToProgressed` (where it is, how far it has left) about once a
  second, then `NavigateToSucceeded`; `NavigateToAborted` with Nav2's `error_code` when it cannot get there (204: the
  goal is outside the map), or `NavigateToCanceled` after a `cancel-skill` naming the command.
- `spin`, Nav2's turn in place by `target_yaw` radians.
- `get-pose`: where the robot believes it is, from its localisation, and `get-odom`, its odometry.

The door's config for it is [`testbed/nav2/best-robot.toml`](testbed/nav2/best-robot.toml), the same shape as below,
and it runs behind [`testbed/nav2/envelope.toml`](testbed/nav2/envelope.toml): the arena inside the walls as the
geofence, the centre pillar as a forbidden zone, and 0.6 m/s as the speed ceiling (see
[The owner's envelope and hold](#the-owners-envelope-and-hold)).

### In front of a ROS 2 robot

The robot runs rosbridge 2.x (`ros-jazzy-rosbridge-server` or newer: actions arrived in 2.0). A config file says
which of its actions and topics the door exposes:

```toml
rosbridge = "ws://127.0.0.1:9090"

[[skills]]
name = "navigate-to-pose"            # the command's name on the door, kebab-case
action = "/navigate_to_pose"
type = "nav2_msgs/action/NavigateToPose"
description = "Drives to a pose in the map frame."

[[topics]]
name = "battery"                     # read as the query get-battery
topic = "/battery_state"
type = "sensor_msgs/msg/BatteryState"
description = "The battery's state; percentage is 0 to 1."
```

```bash
BEST_ROBOT_ROBOT=rosbridge BEST_ROBOT_CONFIG=robot.toml best-robot
```

At start the door asks rosapi for each type and generates the schemas from it. A type rosapi doesn't know, or a
rosbridge that doesn't answer within fifteen seconds, stops it with the reason. The door then offers:

- **Each skill as a command** whose data is the action's goal. A goal with a field the type lacks is refused with a
  400 naming it, before anything reaches the robot. The run is told as events under the command's correlation id:
  `<Skill>Progressed` with its feedback, at most one a second (the latest, and the last before the end), then one of
  `<Skill>Succeeded`, `<Skill>Aborted` or `<Skill>Canceled` (with the result), `<Skill>Stopped` (with the reason and
  the result: the owner's envelope or hold stopped it), or `<Skill>Refused` (with why the robot never started it:
  rejected, no action server, or no connection to the robot when it was sent).
- **`cancel-skill`**, naming the id of the command that started a run. Only that run's sender can cancel it. It ends
  in `SkillCancelRequested`, and the run's own event says how it ended; with no such run in flight on the door, it ends
  in `SkillCancelRefused`. A run the door lost track of (it ended in `CommandFailed`) can only be stopped at the robot.
- **Each topic as a query**, `get-<name>`: the latest message and when it arrived, both null until one has.

Each integer field is held to its ROS width, and a float32 to its range, so a goal number the type cannot carry is
refused with a 400 rather than wrapped. A float the robot sends may be null: that is how rosbridge writes NaN and infinity.

A lost rosbridge is reconnected a second later; one that stops answering without closing its socket (a robot whose
power or cable was pulled) is given up after six silent seconds. A run it cut short ends in `CommandFailed`, since the robot may still
be running it. A command the door was stopped during also ends in `CommandFailed` at the next start, and is never sent
again: a goal sent twice moves a robot twice.

| Variable | Default | What it is |
|---|---|---|
| `BEST_ROBOT_LISTEN` | `127.0.0.1:7780` | Where the door listens, with a port other than 0. Loopback only: off it the key would travel in the clear, and the door refuses to start. |
| `BEST_ROBOT_ENDPOINT` | `http://<listen>` | The base URL agents reach it at. An https proxy on this machine sets it to its own address. |
| `BEST_ROBOT_DATA` | `~/.best-robot` | The admin token, the credentials (hashes only) and the event log. |
| `BEST_ROBOT_ROBOT` | `fake` | The robot behind the door. `fake` answers with a fixed state and moves nothing; `rosbridge` is a ROS 2 robot. |
| `BEST_ROBOT_CONFIG` | none | With `rosbridge`, the config file above. |
| `BEST_ROBOT_ENVELOPE` | none | The owner's envelope file below. Unset or empty, the skills are bounded by nothing but the robot's own limits. |

### The owner's envelope and hold

The robot's owner bounds what any agent may ask of it in a file of their own, never on the door:

```toml
frame = "map"                                      # the frame every place below is in
geofence = [[-2.4, -2.4], [2.4, -2.4], [2.4, 2.4], [-2.4, 2.4]]   # the polygon the robot stays inside

[[zones]]                                          # as many as the owner needs
name = "centre-pillar"
polygon = [[-0.35, -0.35], [0.35, -0.35], [0.35, 0.35], [-0.35, 0.35]]

[position]                                         # where the robot reports it is: a door topic, pointers to x and y
topic = "pose"
x = "/pose/pose/position/x"
y = "/pose/pose/position/y"

[speed]                                            # how fast it reports it moves, and the most it may, in m/s
max = 0.6
topic = "odom"
x = "/twist/twist/linear/x"
y = "/twist/twist/linear/y"
```

A skill that sends the robot somewhere says where its goal keeps the place, with JSON Pointers in the config:

```toml
[[skills]]
name = "navigate-to"
action = "/navigate_to_pose"
type = "nav2_msgs/action/NavigateToPose"
target = { frame = "/pose/header/frame_id", x = "/pose/pose/position/x", y = "/pose/pose/position/y" }
```

The door checks each pointer against the goal's type at start, and the envelope's topics against the config's. Then:

- A goal whose place is outside the geofence is refused with 400 `OUTSIDE_GEOFENCE`, one in a zone with 400
  `IN_FORBIDDEN_ZONE`, one in another frame with 400 `WRONG_FRAME`, and one without a place with 400 `NO_PLACE`,
  before anything reaches the robot. The message names the fix, and `details.nearest` is a place that would be
  admitted.
- While any skill runs, targeted or not, the door reads the position and speed topics every 200 ms. A position
  outside the fence or in a zone, a speed above the ceiling, or no speed reported for two seconds cancels the run,
  which ends in `<Skill>Stopped` with the reason.

The owner takes the robot over at the machine the door runs on, with the door's settings:

```bash
best-robot hold --now --reason "servicing the arm"
```

From then on every skill is refused with 409 `HELD`, naming the reason and since when. `--now` also stops the runs in
flight, each ending in `<Skill>Stopped` with the reason; without it, they run on. The hold is written to
`BEST_ROBOT_DATA` before it is in force, so a door restarted while held is still held. `best-robot release` gives
the robot back.

## Layout

| Crate | What it owns |
|---|---|
| `crates/robot` | The robot as three narrow ports the door depends on: `Robot` (state), `Skills` (run, progress, outcome, cancel) and `Topics` (latest message). Also their fakes. It knows nothing of HTTP or ROS. |
| `crates/rosbridge` | The ROS 2 adapter for `Skills` and `Topics`: the config, schemas generated from rosapi's typedefs, and one rosbridge websocket kept open. |
| `crates/best-robot` | The binary. `main.rs` is the composition root that chooses each part's adapter. `settings` reads the environment, `approver` admits on the machine's admin token, `surface` turns the parts into the catalogue (state, skills, topics, each with its wire shape next to its schema), `guard` wraps the skills in the owner's envelope and hold, `door` puts best-peer's door around them, `owner` serves the hold and release routes, and `cli` is the owner's commands (`approve`, `hold`, `release`). |

## Gate

`scripts/gate.sh all` (Git Bash on Windows) ends in `GATE GREEN` or `GATE RED`. CI runs the same script. Its last two
lanes need Docker: the rosbridge lane builds `testbed/rosbridge` (ROS 2 Jazzy, rosbridge, a test action and a test
topic) and drives the door over it; the nav2 lane builds `testbed/nav2` (the simulated robot above) and drives it
through the door to a pose and back, out of the map, and cancelled on the way, then, behind the envelope above, refuses
a goal outside the fence and one in the zone, refuses `spin` to a key approved `--may navigate-to`, and stops a
navigation with `hold --now`, still held across a restart of the door, until `release`.
