#!/usr/bin/env bash
# Gazebo, the TurtleBot3 and Nav2 headless, and rosbridge on 9090; the container ends when either launch does.
set -e
source /opt/ros/jazzy/setup.bash
ros2 launch nav2_bringup tb3_simulation_launch.py headless:=True use_rviz:=False params_file:=/nav2_params.yaml &
ros2 launch rosbridge_server rosbridge_websocket_launch.xml port:=9090 &
wait -n
