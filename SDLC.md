# How best-robot is built and shipped

> This is the one place the lifecycle of this repository is written down. Open work of any kind (a slice, a bug, a
> decision) is an issue on the Behavioralstate board, never a file: the epic
> [behavioralstate/spec#260](https://github.com/behavioralstate/spec/issues/260) and its sub-issues are the plan.

## 1. Intake

- Work is a slice of the epic, or an issue filed next to it on the same board and made its sub-issue, with the
  problem, the outcome from the agent's side, numbered decisions, and a "Done when" list. Every one carries the
  `best-robot` label; the epic carries `epic` beside it.
- **The BEST spec is stable** ([behavioralstate/spec#37](https://github.com/behavioralstate/spec/issues/37) holds the
  rule). A behaviour the spec does not mention is best-robot's own, documented in its README: never an entry on #37, a
  spec issue or a spec pull request. A session that believes the spec lacks a sentence brings the proof to the owner
  in the chat and stops.
- The board's Status: **Backlog** until the first change; **In progress** from the first change until the issue
  closes; **Done** when it closes, every "Done when" ticked. The epic is In progress while any of its issues is.

## 2. Design rules every change is held to

- BEST is the surface: a capability exists only when it has a schema, a catalogue entry and a description an agent
  can act on; a refusal says what to do; sign-in is the spec's.
- **Skills, never motors.** Nothing below the skill level is on the door: no velocity command, no joint target. The
  robot's own controllers keep the real-time loops, and the emergency stop is physical, never a command.
- **The door is best-peer's.** `best-peer-door` is a git dependency pinned to a best-peer release tag. A change the
  door needs is made in best-peer and released there, then the pin moves; it is never copied here.
- **Depend on abstractions this workspace owns.** The door knows the robot's ports (`Robot`, `Skills`, `Topics` in
  `crates/robot`), never ROS or an adapter; `main.rs` is the one place an adapter is chosen. The door's tests use the
  fakes. An adapter is thin and proven against the ROS 2 testbed (`testbed/rosbridge`, in the gate's rosbridge lane),
  and against the simulated robot (`testbed/nav2`: Gazebo, a TurtleBot3, Nav2, in the gate's nav2 lane), and on a real
  robot by hand, with the proof on the slice's issue. No real robot is ever needed by CI.
- **Never send a goal twice.** A command the door stopped during ends in `CommandFailed` at the next start and is
  never run again; a run whose rosbridge was lost ends the same way, since the robot may still be running it.
- **The owner bounds the robot, never an agent.** The envelope and the hold are read from the owner's file and set by
  the owner's commands on the machine; no command on the door widens them. A change that lets a goal past them, or a
  run go on after a breach, is proven in the nav2 lane. They sit above the robot's own limits and its physical
  emergency stop, never in their place.
- **Small and cohesive.** A module owns one thing (the settings, the approver, each part of the surface, the guard,
  the door, the owner's routes, the owner's commands); the wire shape of a fact lives next to its schema. Nothing is added before a slice needs it.
- Facts are events; an event is versioned, never mutated; a redelivery is idempotent; nothing is clamped silently.
- The README's "Try it" is the operator's path, kept true by the change that moves it.

## 3. Build

- Tests first where one applies, against the fakes.
- `scripts/gate.sh all` (in Git Bash on Windows, with Docker running) runs `cargo fmt --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo test --workspace`, a release build, then `best-validate` on the
  served door, anonymous and signed in through the device flow and the real `best-robot approve`, then the rosbridge
  lane: the ROS 2 testbed in a container, the adapter's testbed tests, the door over it validated signed in, and a goal
  sent through the door and read back as events; then the nav2 lane: the simulated robot driven through the door to a
  pose and back, out of the map and cancelled on the way, each read back as events, then behind the owner's envelope:
  a goal outside the geofence and one in a forbidden zone refused, a key scoped to one skill refused another, and
  `best-robot hold --now` stopping a navigation, held across a restart, until `best-robot release`, and a navigation
  stopped above a speed ceiling on the robot's odometry. It ends in `GATE GREEN` or
  `GATE RED`; CI runs the same script.

## 4. Ship

- **Every change is made on `main` and pushed there**, one commit per piece of work, the subject one plain sentence
  saying what changed and why, with no type prefix; the one prefix is `Review:` on a commit that answers the review.
- **Two gates before every push, both mandatory:**
  1. **The local proof**: `scripts/gate.sh all` ending in `GATE GREEN`.
  2. **The review**: the `workspace-review` skill, run on this repository, on what `main` holds that `origin/main`
     does not. Every finding is fixed on `main` with a test, in a `Review:` commit that names it, or set aside with
     its reason in that message. The local gate runs last, on the head that is pushed.
- `git pull --rebase` before the gates; a red `main` is fixed before anything else is pushed.
- A release, and the workflow that builds it, comes with the first slice that needs someone else to run the binary.

## 5. What we do not do

- No push without `GATE GREEN` and the review; no force-push to `main`.
- No plan file, no `TODO`, no dated narrative: open work is an issue.
