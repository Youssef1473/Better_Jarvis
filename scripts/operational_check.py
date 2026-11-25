#!/usr/bin/env python3
"""
Better_Jarvis Operational Environment Verification Fixture
Exercises Python process supervision, environment isolation, and JSON validation.
"""

import subprocess
import sys
import os
import json
import time

def test_environment_and_exit_propagation():
    # 1. Test clean exit code propagation
    cmd = [sys.executable, "-c", "import sys; sys.exit(0)"]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert res.returncode == 0, "Clean subprocess should exit with 0"

    # 2. Test failure exit code propagation
    cmd_err = [sys.executable, "-c", "import sys; sys.exit(42)"]
    res_err = subprocess.run(cmd_err, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert res_err.returncode == 42, "Subprocess should propagate exact error code 42"

    # 3. Test environment variable isolation
    custom_env = os.environ.copy()
    custom_env["BETTER_JARVIS_OP_TEST"] = "TRUE"
    cmd_env = [sys.executable, "-c", "import os, sys; sys.exit(0 if os.environ.get('BETTER_JARVIS_OP_TEST') == 'TRUE' else 1)"]
    res_env = subprocess.run(cmd_env, env=custom_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert res_env.returncode == 0, "Subprocess should observe explicit environment variables"

    # 4. Test JSON telemetry output
    telemetry = {
        "timestamp": time.time(),
        "status": "OPERATIONAL",
        "checks": [
            {"name": "exit_code_zero", "passed": True},
            {"name": "exit_code_propagation", "passed": True},
            {"name": "env_isolation", "passed": True}
        ]
    }
    dumped = json.dumps(telemetry)
    loaded = json.loads(dumped)
    assert loaded["status"] == "OPERATIONAL"
    assert len(loaded["checks"]) == 3
    print(f"  [PASS] Python operational fixture passed cleanly ({len(loaded['checks'])} checks).")

if __name__ == "__main__":
    print("--- Running Python Operational Environment Verification ---")
    test_environment_and_exit_propagation()
    print("--- Python Operational Verification Passed Successfully! ---")
    sys.exit(0)
