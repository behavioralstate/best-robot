#!/usr/bin/env bash
# rosbridge on 9090 and the testbed node; the container ends when either does.
set -e
source /opt/ros/jazzy/setup.bash
ros2 launch rosbridge_server rosbridge_websocket_launch.xml port:=9090 &
python3 /testbed_node.py &
wait -n
