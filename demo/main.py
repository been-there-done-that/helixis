import os
import sys
import time
import json

print("\n" + "="*50)
print("[Helixis Sandbox Environment] Execution Started!")
print("="*50)

# Check our isolated workspace
workspace = os.environ.get("TASK_WORKSPACE", "/tmp")
print(f"Executing inside securely mapped scratch directory: {workspace}")

# Simulate computation
print("Loading payload parameters...")
time.sleep(0.5)

print("Computing complex generic workloads...")
for i in range(3):
    print(f"Step {i+1}/3: Processing matrix sequence...")
    time.sleep(0.5)
    
result = {"compute": "Success", "latency_ms": 1500, "code": 0}

print("\n" + "="*50)
print(f"[Helixis Sandbox Environment] Job Finished!")
print(f"Final output: {json.dumps(result)}")
print("="*50 + "\n")

sys.exit(0)
