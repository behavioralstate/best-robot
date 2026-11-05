#!/usr/bin/env bash
# The simulated robot and its door, in one command (Git Bash on Windows, with Docker running): Gazebo with a
# TurtleBot3 and Nav2, headless, in a container (testbed/nav2), then the door in front of it, serving until Ctrl-C,
# which stops both. The first run builds a 4 GB image.
#
#   scripts/sim.sh
#
# The door listens where BEST_ROBOT_LISTEN says (127.0.0.1:7780 when not set) with the data in BEST_ROBOT_DATA
# (~/.best-robot when not set), as `best-robot` does, behind the owner's envelope in BEST_ROBOT_ENVELOPE
# (testbed/nav2/envelope.toml when not set; set it empty for none); the simulation's rosbridge is on 127.0.0.1:9092.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/testbed.sh

container=best-robot-sim
trap 'docker rm -f "$container" >/dev/null 2>&1' EXIT
echo "starting the simulated robot (Gazebo, TurtleBot3, Nav2)..."
testbed_up nav2 "$container" 9092 'lifecycle_manager_navigation\]: .{0,20}Managed nodes are active' 300
echo "the simulated robot is up; starting its door"
cargo build --release -q -p best-robot
BEST_ROBOT_ROBOT=rosbridge BEST_ROBOT_CONFIG=testbed/nav2/best-robot.toml \
  BEST_ROBOT_ENVELOPE="${BEST_ROBOT_ENVELOPE-testbed/nav2/envelope.toml}" target/release/best-robot
