# Test file I/O operations

# Test 1: Basic write and read
f = open("/tmp/test_aot_file.txt", "w")
f.write("Hello, World!\n")
f.write("Line 2\n")
f.write("Line 3\n")
f.close()

# Test 2: Read entire file
f = open("/tmp/test_aot_file.txt", "r")
content = f.read()
f.close()
# Check content starts with "Hello" using startswith
assert content.startswith("Hello"), "content.startswith(\"Hello\") should be True"
assert content.find("Line 2") >= 0, "content.find(\"Line 2\") should be greater than = 0"
assert content.find("Line 3") >= 0, "content.find(\"Line 3\") should be greater than = 0"

# Test 3: Read with readline
f = open("/tmp/test_aot_file.txt", "r")
line1 = f.readline()
line2 = f.readline()
line3 = f.readline()
f.close()
assert line1.startswith("Hello"), "line1.startswith(\"Hello\") should be True"
assert line2.startswith("Line 2"), "assertion failed: line2.startswith(\"Line 2\")"
assert line3.startswith("Line 3"), "assertion failed: line3.startswith(\"Line 3\")"

# Test 4: Read with readlines
f = open("/tmp/test_aot_file.txt", "r")
lines = f.readlines()
f.close()
assert len(lines) == 3, "len(lines) should equal 3"
assert lines[0].startswith("Hello"), "lines[0].startswith(\"Hello\") should be True"
assert lines[1].startswith("Line 2"), "assertion failed: lines[1].startswith(\"Line 2\")"
assert lines[2].startswith("Line 3"), "assertion failed: lines[2].startswith(\"Line 3\")"

# Test 5: Context manager (with statement)
with open("/tmp/test_aot_file.txt", "r") as f:
    content = f.read()
assert content.startswith("Hello"), "content.startswith(\"Hello\") should be True"

# Test 6: Write returns bytes written
f = open("/tmp/test_aot_file2.txt", "w")
n = f.write("Test data")
f.close()
assert n == 9, "n should equal 9"

# Test 7: Append mode
f = open("/tmp/test_aot_file.txt", "a")
f.write("Appended line\n")
f.close()

f = open("/tmp/test_aot_file.txt", "r")
content = f.read()
f.close()
assert content.find("Appended line") >= 0, "content.find(\"Appended line\") should be greater than = 0"

# Test 8: Read with n parameter
f = open("/tmp/test_aot_file.txt", "r")
first5 = f.read(5)
f.close()
assert len(first5) == 5, "len(first5) should equal 5"
assert first5 == "Hello", "first5 should equal \"Hello\""

# Test 9: Write with context manager
with open("/tmp/test_aot_file3.txt", "w") as f:
    f.write("Written with context manager\n")

with open("/tmp/test_aot_file3.txt", "r") as f:
    content = f.read()
assert content.find("context manager") >= 0, "content.find(\"context manager\") should be greater than = 0"

# Test 10: File iteration (for line in file)
iter_file = open("/tmp/test_aot_iter_file.txt", "w")
iter_file.write("alpha\nbeta\ngamma\n")
iter_file.close()

iter_file_lines: list[str] = []
for iter_file_line in open("/tmp/test_aot_iter_file.txt", "r"):
    iter_file_lines.append(iter_file_line.strip())
assert len(iter_file_lines) == 3, f"file iter: expected 3 lines, got {len(iter_file_lines)}"
assert iter_file_lines[0] == "alpha", f"file iter[0]: expected alpha, got {iter_file_lines[0]}"
assert iter_file_lines[1] == "beta", f"file iter[1]: expected beta, got {iter_file_lines[1]}"
assert iter_file_lines[2] == "gamma", f"file iter[2]: expected gamma, got {iter_file_lines[2]}"
print("File iteration tests passed!")

# Test 11: r+ mode (read-write, file must exist)
rw_file = "/tmp/test_aot_rw.txt"
f = open(rw_file, "w")
f.write("original content")
f.close()

f = open(rw_file, "r+")
rw_content = f.read()
assert rw_content == "original content", f"r+ read failed: {rw_content}"
# Write overwrites from current position (end of file after read)
f.write(" appended")
f.close()

f = open(rw_file, "r")
rw_result = f.read()
f.close()
assert rw_result == "original content appended", f"r+ write failed: {rw_result}"
print("r+ mode test passed!")

# Test 12: w+ mode (write-read, truncates)
wp_file = "/tmp/test_aot_wp.txt"
f = open(wp_file, "w")
f.write("will be truncated")
f.close()

f = open(wp_file, "w+")
# w+ truncates the file, so read should return empty
wp_content = f.read()
assert wp_content == "", f"w+ should truncate, got: {wp_content}"
f.write("new content")
f.close()

f = open(wp_file, "r")
wp_result = f.read()
f.close()
assert wp_result == "new content", f"w+ write failed: {wp_result}"
print("w+ mode test passed!")

# Test 13: a+ mode (append-read)
ap_file = "/tmp/test_aot_ap.txt"
f = open(ap_file, "w")
f.write("base")
f.close()

f = open(ap_file, "a+")
f.write(" extra")
f.close()

f = open(ap_file, "r")
ap_result = f.read()
f.close()
assert ap_result == "base extra", f"a+ write failed: {ap_result}"
print("a+ mode test passed!")

# Test 14: Invalid mode raises ValueError (not IOError)
got_value_error = False
try:
    f = open("/tmp/test_aot_bad_mode.txt", "z")
except ValueError:
    got_value_error = True
assert got_value_error, "invalid mode should raise ValueError"
print("ValueError for invalid mode test passed!")

# Test 15: Clean up temporary files with os.remove
import os

os.remove("/tmp/test_aot_file.txt")
os.remove("/tmp/test_aot_file2.txt")
os.remove("/tmp/test_aot_file3.txt")
os.remove("/tmp/test_aot_iter_file.txt")
os.remove(rw_file)
os.remove(wp_file)
os.remove(ap_file)

print("All file I/O tests passed!")
