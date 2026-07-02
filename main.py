"""
main.py — end-to-end demo.

Expected output (POST-TASK-4 happy path):
    Physics.calculate_force(10.0, 9.8) = 98.0
"""

import glucore


def main():
    core = glucore.load_core()
    physics = glucore.load_module(core, "physics")
    result = physics.calculate_force(10.0, 9.8) 
   
    print(f"Physics.calculate_force(10.0, 9.8) = {result}")
    print(physics.greet("Hii, From Rust"))


if __name__ == "__main__":
    main()
