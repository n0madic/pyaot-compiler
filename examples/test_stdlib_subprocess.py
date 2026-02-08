# Test file for subprocess module

import subprocess

# Test subprocess.run with simple command (echo)
result1: subprocess.CompletedProcess = subprocess.run(["echo", "hello"])
print("result1.returncode:", result1.returncode)
assert result1.returncode == 0, "echo should return 0"
print("subprocess.run echo test passed!")

# Test subprocess.run with capture_output
result2: subprocess.CompletedProcess = subprocess.run(["echo", "test output"], True, False)
print("result2.returncode:", result2.returncode)
assert result2.returncode == 0, "echo should return 0"
stdout2: str | None = result2.stdout
assert stdout2 is not None, "stdout should not be None"
print("Captured stdout:", stdout2)
assert "test output" in stdout2, "stdout should contain 'test output'"
print("subprocess.run with capture_output test passed!")

# Test subprocess.run - access args
args2: list[str] = result2.args
print("Command args:", args2)
assert len(args2) == 2, "Should have 2 args"
assert args2[0] == "echo", "First arg should be 'echo'"
assert args2[1] == "test output", "Second arg should be 'test output'"
print("CompletedProcess.args test passed!")

# Test subprocess.run with non-zero exit (false command)
result3: subprocess.CompletedProcess = subprocess.run(["false"])
print("result3.returncode:", result3.returncode)
assert result3.returncode != 0, "false should return non-zero"
print("subprocess.run false test passed!")

# Test subprocess.run without capture_output - stdout/stderr should be None
result4: subprocess.CompletedProcess = subprocess.run(["echo", "not captured"], False, False)
stdout4: str | None = result4.stdout
stderr4: str | None = result4.stderr
assert stdout4 is None, "stdout should be None when not captured"
assert stderr4 is None, "stderr should be None when not captured"
print("subprocess.run without capture test passed!")

# Test subprocess.run with stderr capture
result5: subprocess.CompletedProcess = subprocess.run(["sh", "-c", "echo error >&2"], True, False)
stderr5: str | None = result5.stderr
assert stderr5 is not None, "stderr should not be None"
print("Captured stderr:", stderr5)
assert "error" in stderr5, "stderr should contain 'error'"
print("subprocess.run stderr capture test passed!")

print("All subprocess tests passed!")
