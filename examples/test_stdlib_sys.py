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

# Test sys.path — module search path. Lazily initialised on first read
# from: exe dir, CWD, PYTHONPATH entries. The list is a process-wide
# singleton, so user mutations persist (mutations don't affect imports —
# those are resolved at compile time — but the list surface matches
# CPython for portability).
paths: list[str] = sys.path
assert len(paths) >= 1, "sys.path must have at least one entry (exe dir or cwd)"

# Every entry is a non-empty string
for p in paths:
    assert isinstance(p, str), "every sys.path entry must be a str"
    assert len(p) > 0, "sys.path entries must be non-empty"

# Mutation persists via the singleton. Capture the count, append, re-read
# `sys.path` (returns the SAME list), confirm both operations see it.
before: int = len(sys.path)
sys.path.append("/pyaot/test/marker")
after: int = len(sys.path)
assert after == before + 1, "append must persist on the cached singleton list"
assert sys.path[-1] == "/pyaot/test/marker", "appended entry must be the last element"

# pop restores the count — proves we're operating on the same list each time
popped: str = sys.path.pop()
assert popped == "/pyaot/test/marker"
assert len(sys.path) == before, "pop must reduce the cached list back to the original count"

print("sys.path tests passed")
print("All sys module tests passed!")
