"""The gate's stand-in for a robot's ROS 2 stack, behind rosbridge.

- Action `/fibonacci` (example_interfaces/action/Fibonacci): one feedback per step, a tenth of a second apart, then the
  sequence. A goal with order above 40 is rejected; a goal with order 0 is aborted; a cancel is accepted.
- Topic `/battery_state` (sensor_msgs/msg/BatteryState): 87% charge, published every half second.
"""

import time

import rclpy
from example_interfaces.action import Fibonacci
from rclpy.action import ActionServer, CancelResponse, GoalResponse
from rclpy.callback_groups import ReentrantCallbackGroup
from rclpy.executors import MultiThreadedExecutor
from rclpy.node import Node
from sensor_msgs.msg import BatteryState


class Testbed(Node):
    def __init__(self):
        super().__init__("testbed")
        group = ReentrantCallbackGroup()
        ActionServer(
            self,
            Fibonacci,
            "fibonacci",
            execute_callback=self.execute,
            goal_callback=lambda goal: GoalResponse.REJECT if goal.order > 40 else GoalResponse.ACCEPT,
            cancel_callback=lambda _: CancelResponse.ACCEPT,
            callback_group=group,
        )
        self.battery = self.create_publisher(BatteryState, "battery_state", 10)
        self.create_timer(0.5, self.publish_battery)
        self.get_logger().info("testbed ready")

    def publish_battery(self):
        msg = BatteryState()
        msg.percentage = 0.87
        msg.present = True
        self.battery.publish(msg)

    def execute(self, handle):
        order = handle.request.order
        if order == 0:
            handle.abort()
            return Fibonacci.Result()
        sequence = [0, 1]
        for i in range(1, order):
            if handle.is_cancel_requested:
                handle.canceled()
                return Fibonacci.Result(sequence=sequence)
            sequence.append(sequence[i] + sequence[i - 1])
            handle.publish_feedback(Fibonacci.Feedback(sequence=sequence))
            time.sleep(0.1)
        handle.succeed()
        return Fibonacci.Result(sequence=sequence)


def main():
    rclpy.init()
    executor = MultiThreadedExecutor()
    executor.add_node(Testbed())
    executor.spin()


if __name__ == "__main__":
    main()
