# Test file for re module

import re

# Test re.search
m = re.search(r"\d+", "abc123def")
# m should not be None
group0: str = m.group(0)
print("Found:", group0)
assert group0 == "123", "group0 should equal \"123\""

# Test re.match
m2 = re.match(r"\d+", "123abc")
match_str: str = m2.group(0)
print("Match start:", match_str)

# Test re.sub
result: str = re.sub(r"\d+", "X", "a1b2c3")
print("Substituted:", result)
assert result == "aXbXcX", "result should equal \"aXbXcX\""

# Test match.span()
span_match = re.search(r"world", "hello world")
span_result = span_match.span()
assert span_result == (6, 11), f"Expected (6, 11), got {span_result}"

# Test span() with match at beginning
span_match2 = re.match(r"hello", "hello world")
span_result2 = span_match2.span()
assert span_result2 == (0, 5), f"Expected (0, 5), got {span_result2}"

print("All re module tests passed!")
