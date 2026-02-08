# Test file for os module

import os
from os.path import join, exists, basename, dirname, split, isdir, isfile, abspath

# Test os.environ
env: dict[str, str] = os.environ
print("environ keys count:", len(env))

# Test os.path.join
path: str = join("home", "user", "file.txt")
print("Joined path:", path)

# Test with from import
path2: str = join("var", "log", "app.log")
print("Joined path2:", path2)

# Test os.path.exists
exists1: bool = exists(".")
assert exists1 == True, "exists1 should equal True"

exists2: bool = exists("nonexistent_xyz123.txt")
assert exists2 == False, "exists2 should equal False"

# Test with from import
exists3: bool = exists("/")
assert exists3 == True, "exists3 should equal True"

print("os.path.exists tests passed!")

# ============= Test current directory operations =============

# Test os.getcwd
cwd_orig: str = os.getcwd()
print("Current directory:", cwd_orig)
assert len(cwd_orig) > 0, "cwd should not be empty"

# Test os.chdir - save original and restore after test
test_tmp_dir: str = "/tmp"
os.chdir(test_tmp_dir)
cwd_tmp: str = os.getcwd()
print("Changed to:", cwd_tmp)
assert cwd_tmp == "/tmp" or cwd_tmp == "/private/tmp", "Should be in /tmp"

# Change back to original immediately
os.chdir(cwd_orig)
cwd_back: str = os.getcwd()
assert cwd_back == cwd_orig, "Should be back to original directory"
print("os.getcwd/chdir tests passed!")

# ============= Test directory and file operations =============

# Create test directory structure
test_dir: str = "/tmp/test_os_dir_12345"
test_subdir: str = join(test_dir, "subdir")
test_file: str = join(test_dir, "test.txt")

# Clean up if exists from previous run
if exists(test_dir):
    if exists(test_file):
        os.remove(test_file)
    if exists(test_subdir):
        os.rmdir(test_subdir)
    os.rmdir(test_dir)

# Test os.mkdir
os.mkdir(test_dir)
assert exists(test_dir), "test_dir should exist"
print("os.mkdir test passed!")

# Test os.listdir on empty directory
empty_list: list[str] = os.listdir(test_dir)
assert len(empty_list) == 0, "empty directory should have 0 entries"
print("os.listdir empty test passed!")

# Create a file using open/write
file_obj = open(test_file, "w")
file_obj.write("test content")
file_obj.close()

# Test os.listdir with content
dir_list: list[str] = os.listdir(test_dir)
assert len(dir_list) == 1, "directory should have 1 entry"
assert dir_list[0] == "test.txt", "entry should be test.txt"
print("os.listdir test passed!")

# Test os.path.isfile and os.path.isdir (using imported functions)
is_file_test: bool = isfile(test_file)
assert is_file_test == True, "test_file should be a file"

is_dir_test: bool = isdir(test_file)
assert is_dir_test == False, "test_file should not be a directory"

is_dir_test2: bool = isdir(test_dir)
assert is_dir_test2 == True, "test_dir should be a directory"

is_file_test2: bool = isfile(test_dir)
assert is_file_test2 == False, "test_dir should not be a file"
print("os.path.isfile/isdir tests passed!")

# Test os.path.basename, dirname, split
base_name: str = basename(test_file)
assert base_name == "test.txt", "basename should be test.txt"

dir_name: str = dirname(test_file)
assert dir_name == test_dir, "dirname should be test_dir"

split_result: tuple[str, str] = split(test_file)
assert split_result[0] == test_dir, "split[0] should be test_dir"
assert split_result[1] == "test.txt", "split[1] should be test.txt"

# Test with from imports
base_name2: str = basename("/home/user/document.pdf")
assert base_name2 == "document.pdf", "basename should be document.pdf"

dir_name2: str = dirname("/home/user/document.pdf")
assert dir_name2 == "/home/user", "dirname should be /home/user"

split_result2: tuple[str, str] = split("/home/user/document.pdf")
assert split_result2[0] == "/home/user", "split2[0] should be /home/user"
assert split_result2[1] == "document.pdf", "split2[1] should be document.pdf"

print("os.path.basename/dirname/split tests passed!")

# Test os.path.abspath
abs_path: str = abspath(".")
assert len(abs_path) > 0, "absolute path should not be empty"
assert abs_path[0] == "/", "absolute path should start with /"

# Test abspath with absolute path (note: may resolve symlinks like /tmp -> /private/tmp on macOS)
abs_path2: str = abspath(test_file)
# Check that it's absolute and contains the directory name
assert abs_path2[0] == "/", "absolute path should start with /"
assert "test_os_dir_12345" in abs_path2, "should contain directory name"
assert "test.txt" in abs_path2, "should contain file name"
print("os.path.abspath tests passed!")

# Test os.rename
test_file_renamed: str = join(test_dir, "renamed.txt")
os.rename(test_file, test_file_renamed)
assert exists(test_file_renamed), "renamed file should exist"
assert not exists(test_file), "old file should not exist"
print("os.rename test passed!")

# Test os.replace (rename back)
os.replace(test_file_renamed, test_file)
assert exists(test_file), "replaced file should exist"
assert not exists(test_file_renamed), "old file should not exist after replace"
print("os.replace test passed!")

# Test os.makedirs with nested directories
nested_dir: str = join(test_dir, "a", "b", "c")
os.makedirs(nested_dir)
assert exists(nested_dir), "nested directory should exist"

# Test makedirs with exist_ok=True (should not raise error)
os.makedirs(nested_dir, exist_ok=True)
assert exists(nested_dir), "nested directory should still exist"
print("os.makedirs tests passed!")

# Clean up nested directories (from deepest to shallowest)
os.rmdir(nested_dir)
os.rmdir(join(test_dir, "a", "b"))
os.rmdir(join(test_dir, "a"))

# Test os.rmdir
os.mkdir(test_subdir)
assert exists(test_subdir), "subdir should exist"
os.rmdir(test_subdir)
assert not exists(test_subdir), "subdir should be removed"
print("os.rmdir test passed!")

# Test os.remove
os.remove(test_file)
assert not exists(test_file), "file should be removed"
print("os.remove test passed!")

# Clean up test directory
os.rmdir(test_dir)
assert not exists(test_dir), "test_dir should be removed"

print("Directory and file operations tests passed!")

# ============= Test environment variables =============

# Test os.getenv with existing variable
path_env: str | None = os.getenv("PATH")
if path_env is not None:
    assert len(path_env) > 0, "PATH should not be empty"

# Test os.getenv with non-existent variable
nonexist: str | None = os.getenv("NONEXISTENT_VAR_XYZ123")
assert nonexist is None, "non-existent var should be None"

# Test os.getenv with default value
default_val: str | None = os.getenv("NONEXISTENT_VAR_XYZ123", "default_value")
assert default_val == "default_value", "should return default value"

print("os.getenv tests passed!")

# ============= Test OS information =============

# Test os.name
os_name: str = os.name
print("os.name:", os_name)
assert os_name == "posix" or os_name == "nt", "os.name should be posix or nt"
print("os.name test passed!")

print("\nAll os module tests passed!")
