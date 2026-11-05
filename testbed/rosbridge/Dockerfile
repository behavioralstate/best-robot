# The ROS 2 side of the gate's rosbridge lane: rosbridge on 9090 and one node with a test action and a test topic.
# Pinned to Jazzy, whose rosbridge (2.x) carries actions.
FROM ros:jazzy-ros-base

RUN apt-get update \
 && apt-get install -y --no-install-recommends ros-jazzy-rosbridge-server ros-jazzy-example-interfaces \
 && rm -rf /var/lib/apt/lists/*

COPY testbed_node.py /testbed_node.py
COPY run.sh /run.sh
RUN chmod +x /run.sh

EXPOSE 9090
CMD ["/run.sh"]
