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

# Test 10: Clean up temporary files with os.remove
import os

os.remove("/tmp/test_aot_file.txt")
os.remove("/tmp/test_aot_file2.txt")
os.remove("/tmp/test_aot_file3.txt")

print("All file I/O tests passed!")
