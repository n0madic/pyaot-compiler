# Test file for sys module

import sys

# Test sys.argv
args: list[str] = sys.argv
assert len(args) >= 1, "len(args) should be greater than = 1"
print(args[0])  # Should print program name

# Test argv length is at least 1
print("argv length:", len(args))

# Test sys.intern - intern strings for memory efficiency
s1: str = "test_string"
s2: str = sys.intern(s1)
# s2 should be interned version of s1
assert s2 == s1, "interned string should equal original"

# Test interning an already interned string (should return same object in CPython)
s3: str = "another_test"
interned1: str = sys.intern(s3)
interned2: str = sys.intern(s3)
assert interned1 == interned2, "interning same string twice should give same result"

# Test interning with string literals
literal: str = sys.intern("literal_string")
assert literal == "literal_string", "interned literal should equal original"

# Test that interned strings work normally in operations
interned_str: str = sys.intern("hello")
assert interned_str.upper() == "HELLO", "interned string should support methods"
assert len(interned_str) == 5, "interned string should have correct length"

print("sys.intern tests passed")
print("All sys module tests passed!")
