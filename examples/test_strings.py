# Consolidated test file for string operations

# ===== SECTION: String literals and concatenation =====

# Test basic string assignment and printing
s: str = "Hello"
print(s)

# Test string concatenation
str_a: str = "Hello"
str_b: str = " World"
str_c: str = str_a + str_b
print(str_c)

# Test string length
length: int = len(s)
print(length)
assert length == 5, "len failed"

# ===== SECTION: String methods =====

# Test string upper/lower
lower_str: str = "hello world"
upper_result: str = lower_str.upper()
print(upper_result)

upper_str: str = "HELLO WORLD"
lower_result: str = upper_str.lower()
print(lower_result)

# Test string strip
padded: str = "  hello  "
stripped: str = padded.strip()
print(stripped)

# Test string startswith/endswith
test_str: str = "Hello World"
starts: bool = test_str.startswith("Hello")
assert starts, "startswith failed"
print(starts)

ends: bool = test_str.endswith("World")
assert ends, "endswith failed"
print(ends)

not_starts: bool = test_str.startswith("World")
assert not not_starts, "startswith should be false"

not_ends: bool = test_str.endswith("Hello")
assert not not_ends, "endswith should be false"

# Test string find
haystack: str = "Hello World"
pos: int = haystack.find("World")
print(pos)
assert pos == 6, "find failed"

not_found: int = haystack.find("xyz")
assert not_found == -1, "find should return -1 for not found"

# Test string replace
original: str = "Hello World"
replaced: str = original.replace("World", "Python")
print(replaced)

# ===== SECTION: String slicing and indexing =====

text: str = "Hello World"
slice1: str = text[0:5]
print(slice1)

slice2: str = text[6:11]
print(slice2)

# Test string indexing
first_char: str = text[0]
print(first_char)

mid_char: str = text[6]
print(mid_char)

# Test string slicing in loop (chunking)
chunk_text: str = "Hello, World!"
chunks: list[str] = []
for i in range(0, len(chunk_text), 3):
    chunk: str = chunk_text[i:i+3]
    chunks.append(chunk)
assert chunks == ["Hel", "lo,", " Wo", "rld", "!"], "string slicing in loop failed"

# Test string slicing to end (text[7:])
end_slice: str = chunk_text[7:]
assert end_slice == "World!", "string slice [7:] failed"

# Test string slicing from start (text[:5])
start_slice: str = chunk_text[:5]
assert start_slice == "Hello", "string slice [:5] failed"

# Test string negative indexing
assert chunk_text[-1] == "!", "string negative index [-1] failed"
assert chunk_text[-6:] == "World!", "string negative slice [-6:] failed"

# ===== SECTION: String multiplication =====

repeated: str = "ab" * 3
print(repeated)

single: str = "x" * 5
print(single)

# ===== SECTION: .format() method =====

# Basic single placeholder
name: str = "World"
greeting: str = "Hello {}!".format(name)
assert greeting == "Hello World!", "basic format failed"

# Multiple placeholders
fmt_a: int = 1
fmt_b: int = 2
fmt_result: str = "{} + {} = {}".format(fmt_a, fmt_b, fmt_a + fmt_b)
assert fmt_result == "1 + 2 = 3", "multiple placeholders failed"

# Format with different types
fmt_x: int = 42
fmt_pi: float = 3.14
fmt_flag: bool = True
mixed: str = "int={}, float={}, bool={}".format(fmt_x, fmt_pi, fmt_flag)
print(mixed)

# Format starting with placeholder
fmt_start: str = "{} is the answer".format(fmt_x)
assert fmt_start == "42 is the answer", "start with placeholder failed"

# Format ending with placeholder
fmt_end: str = "The answer is {}".format(fmt_x)
assert fmt_end == "The answer is 42", "end with placeholder failed"

# Format with only placeholder
only: str = "{}".format(fmt_x)
assert only == "42", "only placeholder failed"

# Format with no placeholders
no_ph: str = "No placeholders".format()
assert no_ph == "No placeholders", "no placeholders failed"

# Empty format string
empty_fmt: str = "".format()
assert empty_fmt == "", "empty format failed"

# String argument (no conversion needed)
inner: str = "world"
nested_fmt: str = "hello {}!".format(inner)
assert nested_fmt == "hello world!", "string arg failed"

# Escaped braces
escaped: str = "Use {{}} for placeholders".format()
assert escaped == "Use {} for placeholders", "escaped braces failed"

# Multiple escaped and real placeholders
complex_esc: str = "{{{}}}".format(fmt_x)
assert complex_esc == "{42}", "complex escaped failed"

# ===== SECTION: Advanced .format() specifications =====

# Positional index placeholders
idx_basic: str = "{0} {1} {0}".format("a", "b")
assert idx_basic == "a b a", "index basic failed"

idx_math: str = "{1} + {0} = {2}".format(1, 2, 3)
assert idx_math == "2 + 1 = 3", "index math failed"

idx_reverse: str = "{2}{1}{0}".format("c", "b", "a")
assert idx_reverse == "abc", "index reverse failed"

# Named placeholders (keyword arguments)
named_basic: str = "{name} is {age}".format(name="Alice", age=30)
assert named_basic == "Alice is 30", "named basic failed"

named_hello: str = "Hello, {person}!".format(person="Bob")
assert named_hello == "Hello, Bob!", "named hello failed"

named_multi: str = "{x} and {y} and {x}".format(x="foo", y="bar")
assert named_multi == "foo and bar and foo", "named multi failed"

# Mixed positional and named
mixed_pn: str = "{0} and {name}".format("first", name="second")
assert mixed_pn == "first and second", "mixed positional and named failed"

mixed_complex: str = "{0}: {item}, {1}: {value}".format("key", "val", item="apple", value=42)
assert mixed_complex == "key: apple, val: 42", "mixed complex failed"

# Width alignment - right align (>)
right_basic: str = "{:>10}".format("hi")
assert right_basic == "        hi", "right align basic failed"

right_idx: str = "{0:>5}".format("x")
assert right_idx == "    x", "right align with index failed"

right_named: str = "{val:>8}".format(val="abc")
assert right_named == "     abc", "right align with name failed"

# Width alignment - left align (<)
left_basic: str = "{:<10}".format("hi")
assert left_basic == "hi        ", "left align basic failed"

left_idx: str = "{0:<5}".format("xy")
assert left_idx == "xy   ", "left align with index failed"

# Width alignment - center (^)
center_basic: str = "{:^10}".format("hi")
assert center_basic == "    hi    ", "center align basic failed"

center_odd: str = "{:^9}".format("ab")
assert center_odd == "   ab    ", "center align odd width failed"

# Fill character with alignment
fill_right: str = "{:*>10}".format("hi")
assert fill_right == "********hi", "fill right failed"

fill_left: str = "{:-<10}".format("hi")
assert fill_left == "hi--------", "fill left failed"

fill_center: str = "{:=^10}".format("hi")
assert fill_center == "====hi====", "fill center failed"

fill_dot: str = "{:.>6}".format("x")
assert fill_dot == ".....x", "fill dot failed"

# Combined: index/name + format spec
combined_idx: str = "{0:>10}".format("test")
assert combined_idx == "      test", "combined index spec failed"

combined_name: str = "{name:<10}".format(name="Bob")
assert combined_name == "Bob       ", "combined name spec failed"

combined_fill: str = "{item:_^12}".format(item="hi")
assert combined_fill == "_____hi_____", "combined fill spec failed"

# Width smaller than content (no padding)
no_pad: str = "{:>3}".format("hello")
assert no_pad == "hello", "no padding when width smaller failed"

no_pad_center: str = "{:^2}".format("abcde")
assert no_pad_center == "abcde", "no padding center failed"

# Numeric with width
num_right: str = "{:>5}".format(42)
assert num_right == "   42", "numeric right align failed"

num_left: str = "{:<5}".format(7)
assert num_left == "7    ", "numeric left align failed"

num_fill: str = "{:0>4}".format(99)
assert num_fill == "0099", "numeric zero fill failed"

# Float with precision and width
float_prec: str = "{:>10.2f}".format(3.14159)
assert float_prec == "      3.14", "float precision and width failed"

float_fill: str = "{:*>8.1f}".format(7.89)
assert float_fill == "*****7.9", "float fill and precision failed"

float_named: str = "{pi:.3f}".format(pi=3.14159265)
assert float_named == "3.142", "named float precision failed"

# Edge cases
single_idx: str = "{0}".format("only")
assert single_idx == "only", "single index failed"

empty_spec: str = "{0:}".format("test")
assert empty_spec == "test", "empty spec after colon failed"

print("Advanced .format() tests passed!")

# ===== SECTION: Format spec type conversions =====

# Format spec: hex, oct, bin
hex_lower: str = "{:x}".format(255)
assert hex_lower == "ff", f"hex lower failed: got {hex_lower}"

hex_upper: str = "{:X}".format(255)
assert hex_upper == "FF", f"hex upper failed: got {hex_upper}"

oct_fmt: str = "{:o}".format(255)
assert oct_fmt == "377", f"oct format failed: got {oct_fmt}"

bin_fmt: str = "{:b}".format(10)
assert bin_fmt == "1010", f"bin format failed: got {bin_fmt}"

# Format spec: default alignment for strings vs numbers
str_default_align: str = "{:10}".format("hi")
assert str_default_align == "hi        ", f"string default align failed: got '{str_default_align}'"

num_default_align: str = "{:10}".format(42)
assert num_default_align == "        42", f"number default align failed: got '{num_default_align}'"

print("Format spec type conversion tests passed!")

# ===== SECTION: Grouping option (:, and :_) =====

# Integer grouping with comma
r_group_comma: str = f"{1000000:,}"
assert r_group_comma == "1,000,000", f"comma grouping failed: got '{r_group_comma}'"

# Integer grouping with underscore
r_group_under: str = f"{1000000:_}"
assert r_group_under == "1_000_000", f"underscore grouping failed: got '{r_group_under}'"

# Small number (no grouping needed)
r_group_small: str = f"{42:,}"
assert r_group_small == "42", f"small number grouping failed: got '{r_group_small}'"

# Negative number with grouping
r_group_neg: str = f"{-1234567:,}"
assert r_group_neg == "-1,234,567", f"negative grouping failed: got '{r_group_neg}'"

# Zero with grouping
r_group_zero: str = f"{0:,}"
assert r_group_zero == "0", f"zero grouping failed: got '{r_group_zero}'"

# Float with grouping and precision
r_group_float: str = f"{1234567.89:,.2f}"
assert r_group_float == "1,234,567.89", f"float grouping failed: got '{r_group_float}'"

print("Grouping option tests passed!")

# ===== SECTION: F-string width and alignment =====

# Right-align string
fa_right: str = f"{'hello':>10}"
assert fa_right == "     hello", f"f-string right-align failed: got '{fa_right}'"

# Left-align string
fa_left: str = f"{'hello':<10}"
assert fa_left == "hello     ", f"f-string left-align failed: got '{fa_left}'"

# Center-align string
fa_center: str = f"{'hello':^11}"
assert fa_center == "   hello   ", f"f-string center-align failed: got '{fa_center}'"

# Zero-pad integer
fa_zeropad: str = f"{42:05d}"
assert fa_zeropad == "00042", f"f-string zero-pad failed: got '{fa_zeropad}'"

# Right-align integer with spaces (default for numbers)
fa_int_right: str = f"{42:8d}"
assert fa_int_right == "      42", f"f-string int right-align failed: got '{fa_int_right}'"

# Left-align integer
fa_int_left: str = f"{42:<8d}"
assert fa_int_left == "42      ", f"f-string int left-align failed: got '{fa_int_left}'"

# Fill with custom char
fa_custom_fill: str = f"{'hi':*^10}"
assert fa_custom_fill == "****hi****", f"f-string custom fill failed: got '{fa_custom_fill}'"

# Width with no alignment (string defaults to left)
fa_str_default: str = f"{'ab':5}"
assert fa_str_default == "ab   ", f"f-string default string align failed: got '{fa_str_default}'"

# Width with no alignment (int defaults to right)
fa_int_default: str = f"{7:5}"
assert fa_int_default == "    7", f"f-string default int align failed: got '{fa_int_default}'"

# Zero-pad hex
fa_hex_pad: str = f"{255:08x}"
assert fa_hex_pad == "000000ff", f"f-string zero-pad hex failed: got '{fa_hex_pad}'"

# Grouping combined with width
fa_group_width: str = f"{1000000:>15,}"
assert fa_group_width == "      1,000,000", f"f-string grouping+width failed: got '{fa_group_width}'"

print("F-string width and alignment tests passed!")

# ===== SECTION: F-string interpolation =====

# Test str() builtin with different types
s1: str = str(42)
assert s1 == "42", "str(int) failed"

s2: str = str(True)
assert s2 == "True", "str(bool) failed"

s3: str = str(False)
assert s3 == "False", "str(bool False) failed"

# Test str() with float
pi_val: float = 3.14159
s4: str = str(pi_val)
assert s4 == "3.14159", "str(float) failed"

s5: str = str(2.5)
assert s5 == "2.5", "str(2.5) failed"

# Test basic f-string interpolation
f_name: str = "World"
f_greeting: str = f"Hello {f_name}!"
assert f_greeting == "Hello World!", "basic f-string failed"

# Test f-string with integers
f_x: int = 42
f_msg: str = f"The answer is {f_x}"
assert f_msg == "The answer is 42", "int interpolation failed"

# Test f-string with expressions
f_a: int = 1
f_b: int = 2
f_result: str = f"{f_a} + {f_b} = {f_a + f_b}"
assert f_result == "1 + 2 = 3", "expression interpolation failed"

# Test f-string with booleans
f_flag: bool = True
bool_str: str = f"Flag is {f_flag}"
assert bool_str == "Flag is True", "bool interpolation failed"

# Test f-string with multiple variables
f_x2: int = 10
f_y2: int = 20
multi: str = f"x={f_x2}, y={f_y2}"
assert multi == "x=10, y=20", "multi variable f-string failed"

# Test empty f-string
empty_f: str = f""
assert empty_f == "", "empty f-string failed"

# Test f-string with only literal
only_lit: str = f"Just text"
assert only_lit == "Just text", "literal-only f-string failed"

# Test f-string starting with variable
start_var: str = f"{f_x} is the answer"
assert start_var == "42 is the answer", "f-string starting with var failed"

# Test f-string ending with variable
end_var: str = f"The answer is {f_x}"
assert end_var == "The answer is 42", "f-string ending with var failed"

# Test nested string variable
inner_str: str = "world"
nested_f: str = f"hello {inner_str}!"
assert nested_f == "hello world!", "nested string var failed"

# ===== SECTION: Float formatting with precision =====

pi: float = 3.14159
formatted_pi: str = f"Pi is approximately {pi:.2f}"
assert formatted_pi == "Pi is approximately 3.14", "float format :.2f failed"

# Test with more decimal places
e: float = 2.71828
formatted_e: str = f"e = {e:.3f}"
assert formatted_e == "e = 2.718", "float format :.3f failed"

# Test multiple expressions with formatting
x_val: int = 10
y_val: int = 20
multi_expr: str = f"Sum of {x_val} and {y_val} is {x_val + y_val}"
assert multi_expr == "Sum of 10 and 20 is 30", "f-string with expression failed"

# ===== SECTION: String in operator (substring check) =====

# Basic substring check
haystack: str = "Hello World"
assert "Hello" in haystack, "in operator basic failed"
assert "World" in haystack, "in operator basic 2 failed"
assert "lo Wo" in haystack, "in operator middle failed"

# Check for single character
assert "H" in haystack, "in operator single char start failed"
assert "d" in haystack, "in operator single char end failed"
assert "o" in haystack, "in operator single char middle failed"

# Empty string is always in any string
assert "" in haystack, "in operator empty needle failed"
assert "" in "", "in operator empty in empty failed"

# Not found cases
assert "xyz" not in haystack, "not in operator failed"
assert "hello" not in haystack, "not in case-sensitive failed"
assert "HELLO" not in haystack, "not in case-sensitive 2 failed"

# Needle longer than haystack
assert "Hello World!" not in haystack, "not in longer needle failed"

# Full string match
assert "Hello World" in haystack, "in operator full match failed"

# Edge cases
short: str = "ab"
assert "ab" in short, "in operator exact match short failed"
assert "a" in short, "in operator first char failed"
assert "b" in short, "in operator last char failed"
assert "abc" not in short, "not in longer than string failed"

print("string in operator tests passed")

# ===== SECTION: String predicate methods =====
# Test str.isdigit(), str.isalpha(), str.isalnum(), str.isspace(), str.isupper(), str.islower()

# isdigit() tests
digits = "123"
assert digits.isdigit() == True, "isdigit() on digits failed"
assert "abc".isdigit() == False, "isdigit() on letters failed"
assert "12a3".isdigit() == False, "isdigit() on mixed failed"
assert "".isdigit() == False, "isdigit() on empty failed"
if digits.isdigit():
    pass  # Branch test - should be taken
else:
    assert False, "isdigit() branch test failed"

# isalpha() tests
letters = "abc"
assert letters.isalpha() == True, "isalpha() on letters failed"
assert "123".isalpha() == False, "isalpha() on digits failed"
assert "ab1c".isalpha() == False, "isalpha() on mixed failed"
assert "".isalpha() == False, "isalpha() on empty failed"

# isalnum() tests
alnum = "abc123"
assert alnum.isalnum() == True, "isalnum() on alphanumeric failed"
assert "abc".isalnum() == True, "isalnum() on letters failed"
assert "123".isalnum() == True, "isalnum() on digits failed"
assert "abc 123".isalnum() == False, "isalnum() with space failed"
assert "".isalnum() == False, "isalnum() on empty failed"

# isspace() tests
spaces = "   "
assert spaces.isspace() == True, "isspace() on spaces failed"
assert "\t\n".isspace() == True, "isspace() on whitespace failed"
assert "  a  ".isspace() == False, "isspace() with letter failed"
assert "".isspace() == False, "isspace() on empty failed"

# isupper() tests
upper = "HELLO"
assert upper.isupper() == True, "isupper() on uppercase failed"
assert "hello".isupper() == False, "isupper() on lowercase failed"
assert "Hello".isupper() == False, "isupper() on mixed case failed"
assert "HELLO123".isupper() == True, "isupper() with digits failed"
assert "".isupper() == False, "isupper() on empty failed"

# islower() tests
lower = "hello"
assert lower.islower() == True, "islower() on lowercase failed"
assert "HELLO".islower() == False, "islower() on uppercase failed"
assert "Hello".islower() == False, "islower() on mixed case failed"
assert "hello123".islower() == True, "islower() with digits failed"
assert "".islower() == False, "islower() on empty failed"

# Test storing predicate results in variables
result = "test".isalpha()
assert result == True, "storing isalpha() result failed"

# Test using predicates in comparisons
if "999".isdigit() == True:
    pass  # Expected
else:
    assert False, "predicate == True comparison failed"

print("String predicate tests passed")

# ===== SECTION: Additional string methods =====

# split() with no args (splits on whitespace)
split_ws: list[str] = "hello world test".split()
assert split_ws == ["hello", "world", "test"]
split_comma: list[str] = "a,b,c".split(",")
assert split_comma == ["a", "b", "c"]
split_max: list[str] = "a,b,c,d".split(",", 2)
assert split_max == ["a", "b", "c,d"]
split_empty: list[str] = "".split(",")
assert split_empty == [""], "split_empty should equal [\"\"]"

# join()
join_dash: str = "-".join(["a", "b", "c"])
assert join_dash == "a-b-c", "join_dash should equal \"a-b-c\""
join_empty_sep: str = "".join(["a", "b", "c"])
assert join_empty_sep == "abc", "join_empty_sep should equal \"abc\""
join_single: str = ",".join(["only"])
assert join_single == "only", "join_single should equal \"only\""
empty_list: list[str] = []
join_empty_list: str = ",".join(empty_list)
assert join_empty_list == "", "join_empty_list should equal \"\""

print("split() and join() tests passed")

# count()
assert "hello".count("l") == 2, "\"hello\".count(\"l\") should equal 2"
assert "hello".count("x") == 0, "\"hello\".count(\"x\") should equal 0"
assert "hello".count("ll") == 1, "\"hello\".count(\"ll\") should equal 1"
assert "test".count("") == 5, "\"test\".count(\"\") should equal 5"

print("count() tests passed")

# title(), capitalize(), swapcase()
title1: str = "hello world".title()
assert title1 == "Hello World", "title1 should equal \"Hello World\""
title2: str = "HELLO WORLD".title()
assert title2 == "Hello World", "title2 should equal \"Hello World\""
cap1: str = "hello world".capitalize()
assert cap1 == "Hello world", "cap1 should equal \"Hello world\""
swap1: str = "Hello World".swapcase()
assert swap1 == "hELLO wORLD", "swap1 should equal \"hELLO wORLD\""

print("title(), capitalize(), swapcase() tests passed")

# lstrip(), rstrip()
lstrip1: str = "   hello".lstrip()
assert lstrip1 == "hello", "lstrip1 should equal \"hello\""
lstrip2: str = "xxxhello".lstrip("x")
assert lstrip2 == "hello", "lstrip2 should equal \"hello\""
rstrip1: str = "hello   ".rstrip()
assert rstrip1 == "hello", "rstrip1 should equal \"hello\""
rstrip2: str = "helloyyy".rstrip("y")
assert rstrip2 == "hello", "rstrip2 should equal \"hello\""

print("lstrip(), rstrip() tests passed")

# center(), ljust(), rjust(), zfill()
center1: str = "hi".center(6)
assert center1 == "  hi  ", "center1 should equal \"  hi  \""
center2: str = "hi".center(6, "-")
assert center2 == "--hi--", "center2 should equal \"--hi--\""
ljust1: str = "hi".ljust(5)
assert ljust1 == "hi   ", "ljust1 should equal \"hi   \""
rjust1: str = "hi".rjust(5)
assert rjust1 == "   hi", "rjust1 should equal \"   hi\""
zfill1: str = "42".zfill(5)
assert zfill1 == "00042", "zfill1 should equal \"00042\""
zfill2: str = "-42".zfill(5)
assert zfill2 == "-0042", "zfill2 should equal \"-0042\""

print("center(), ljust(), rjust(), zfill() tests passed")

# ===== SECTION: F-string conversion flags (!r, !s) =====

# Test !r with string - should add quotes
fstr_r: str = "hello"
fstr_r_result: str = f"{fstr_r!r}"
assert fstr_r_result == "'hello'", f"f-string !r with string failed: got {fstr_r_result}"

# Test !r with integer - same as str for int
fstr_r_int: int = 42
fstr_r_int_result: str = f"{fstr_r_int!r}"
assert fstr_r_int_result == "42", f"f-string !r with int failed: got {fstr_r_int_result}"

# Test !r with float
fstr_r_float: float = 3.14
fstr_r_float_result: str = f"{fstr_r_float!r}"
assert fstr_r_float_result == "3.14", f"f-string !r with float failed: got {fstr_r_float_result}"

# Test !r with boolean
fstr_r_bool: bool = True
fstr_r_bool_result: str = f"{fstr_r_bool!r}"
assert fstr_r_bool_result == "True", f"f-string !r with bool failed: got {fstr_r_bool_result}"

# Test !r with None
fstr_r_none_result: str = f"{None!r}"
assert fstr_r_none_result == "None", f"f-string !r with None failed: got {fstr_r_none_result}"

# Test !s (explicit str conversion)
fstr_s_val: int = 123
fstr_s_result: str = f"{fstr_s_val!s}"
assert fstr_s_result == "123", f"f-string !s with int failed: got {fstr_s_result}"

# Test !s with string - should NOT add quotes
fstr_s_str: str = "hello"
fstr_s_str_result: str = f"{fstr_s_str!s}"
assert fstr_s_str_result == "hello", f"f-string !s with string failed: got {fstr_s_str_result}"

# Test mixed conversions in same f-string
mixed_val: str = "world"
mixed_result: str = f"str: {mixed_val!s}, repr: {mixed_val!r}"
assert mixed_result == "str: world, repr: 'world'", f"f-string mixed failed: got {mixed_result}"

# Test !r in larger f-string context
name_repr: str = "Alice"
greeting_repr: str = f"Hello, {name_repr!r}!"
assert greeting_repr == "Hello, 'Alice'!", f"f-string !r in context failed: got {greeting_repr}"

# Test !a (ascii conversion) with ASCII string - should be same as !r
fstr_a_ascii: str = "hello"
fstr_a_ascii_result: str = f"{fstr_a_ascii!a}"
assert fstr_a_ascii_result == "'hello'", f"f-string !a with ASCII string failed: got {fstr_a_ascii_result}"

# Test !a with int - same as str for int
fstr_a_int: int = 42
fstr_a_int_result: str = f"{fstr_a_int!a}"
assert fstr_a_int_result == "42", f"f-string !a with int failed: got {fstr_a_int_result}"

# Test !a with float
fstr_a_float: float = 3.14
fstr_a_float_result: str = f"{fstr_a_float!a}"
assert fstr_a_float_result == "3.14", f"f-string !a with float failed: got {fstr_a_float_result}"

# Test !a with boolean
fstr_a_bool: bool = True
fstr_a_bool_result: str = f"{fstr_a_bool!a}"
assert fstr_a_bool_result == "True", f"f-string !a with bool failed: got {fstr_a_bool_result}"

# Test !a with None
fstr_a_none_result: str = f"{None!a}"
assert fstr_a_none_result == "None", f"f-string !a with None failed: got {fstr_a_none_result}"

# Test ascii() builtin directly with ASCII string
ascii_test1: str = ascii("hello")
assert ascii_test1 == "'hello'", f"ascii() with ASCII string failed: got {ascii_test1}"

# Test ascii() builtin with int
ascii_test2: str = ascii(42)
assert ascii_test2 == "42", f"ascii() with int failed: got {ascii_test2}"

print("f-string conversion flag (!r, !s, !a) tests passed")

# ===== SECTION: String interning (compile-time constants) =====
# These tests verify that string interning works correctly for memory efficiency.
# Identical string literals should be deduplicated by the runtime.

# Test that identical string literals work correctly
intern_s1: str = "hello"
intern_s2: str = "hello"
intern_s3: str = "world"

assert intern_s1 == intern_s2, "identical string literals should be equal"
assert intern_s1 != intern_s3, "different string literals should not be equal"

# Test string operations on interned strings
assert len(intern_s1) == 5, "interned string length check"
assert intern_s1.upper() == "HELLO", "interned string upper"
assert intern_s1 + " " + intern_s3 == "hello world", "interned string concat"

# Test dict key interning (keys should be deduplicated)
intern_d1: dict[str, int] = {"key": 1, "value": 10}
intern_d2: dict[str, int] = {"key": 2, "value": 20}
intern_d3: dict[str, int] = {"key": 3, "value": 30}

# Verify dict operations work correctly with interned keys
assert intern_d1["key"] == 1, "dict with interned key 1"
assert intern_d2["key"] == 2, "dict with interned key 2"
assert intern_d3["key"] == 3, "dict with interned key 3"
assert intern_d1["value"] == 10, "dict with interned value key 1"
assert intern_d2["value"] == 20, "dict with interned value key 2"
assert intern_d3["value"] == 30, "dict with interned value key 3"

# Test empty string interning
empty1: str = ""
empty2: str = ""
assert empty1 == empty2, "empty strings should be equal"
assert len(empty1) == 0, "empty string length should be 0"

# Test interning with dict update
intern_d4: dict[str, int] = {}
intern_d4["shared_key"] = 100
intern_d4["shared_key"] = 200  # Same key, should use interned version
assert intern_d4["shared_key"] == 200, "dict key update with interning"

# Test multiple dicts with same keys
dicts: list[dict[str, int]] = []
for i in range(5):
    d: dict[str, int] = {"common_key": i}
    dicts.append(d)

for i in range(5):
    assert dicts[i]["common_key"] == i, f"dict {i} common_key check"

print("String interning tests passed")

# ===== SECTION: New string methods (removeprefix, removesuffix, expandtabs, splitlines, partition, rpartition) =====

# str.removeprefix() - prefix present
prefix_test1: str = "HelloWorld".removeprefix("Hello")
assert prefix_test1 == "World", "removeprefix basic should equal \"World\""

# str.removeprefix() - prefix not present
prefix_test2: str = "HelloWorld".removeprefix("Bye")
assert prefix_test2 == "HelloWorld", "removeprefix no match should return original"

# str.removeprefix() - empty prefix
prefix_test3: str = "test".removeprefix("")
assert prefix_test3 == "test", "removeprefix empty should return original"

# str.removeprefix() - prefix longer than string
prefix_test4: str = "hi".removeprefix("hello")
assert prefix_test4 == "hi", "removeprefix too long should return original"

# str.removeprefix() - entire string is prefix
prefix_test5: str = "hello".removeprefix("hello")
assert prefix_test5 == "", "removeprefix entire string should return empty"

# str.removesuffix() - suffix present
suffix_test1: str = "HelloWorld".removesuffix("World")
assert suffix_test1 == "Hello", "removesuffix basic should equal \"Hello\""

# str.removesuffix() - suffix not present
suffix_test2: str = "HelloWorld".removesuffix("Bye")
assert suffix_test2 == "HelloWorld", "removesuffix no match should return original"

# str.removesuffix() - empty suffix
suffix_test3: str = "test".removesuffix("")
assert suffix_test3 == "test", "removesuffix empty should return original"

# str.removesuffix() - suffix longer than string
suffix_test4: str = "hi".removesuffix("hello")
assert suffix_test4 == "hi", "removesuffix too long should return original"

# str.removesuffix() - entire string is suffix
suffix_test5: str = "hello".removesuffix("hello")
assert suffix_test5 == "", "removesuffix entire string should return empty"

print("removeprefix() and removesuffix() tests passed")

# str.expandtabs() - default tabsize (8)
tabs_test1: str = "hello\tworld".expandtabs(8)
assert tabs_test1 == "hello   world", "expandtabs default should expand to 8"

# str.expandtabs() - custom tabsize
tabs_test2: str = "a\tb\tc".expandtabs(4)
assert tabs_test2 == "a   b   c", "expandtabs(4) should work"

# str.expandtabs() - tab at start
tabs_test3: str = "\thello".expandtabs(4)
assert tabs_test3 == "    hello", "expandtabs at start should work"

# str.expandtabs() - multiple tabs
tabs_test4: str = "a\t\tb".expandtabs(4)
assert tabs_test4 == "a       b", "expandtabs multiple tabs should work"

# str.expandtabs() - with newline (resets column)
tabs_test5: str = "hello\n\tworld".expandtabs(8)
assert tabs_test5 == "hello\n        world", "expandtabs after newline should reset column"

# str.expandtabs() - tabsize 0 (removes tabs)
tabs_test6: str = "a\tb\tc".expandtabs(0)
assert tabs_test6 == "abc", "expandtabs(0) should remove tabs"

# str.expandtabs() - no tabs
tabs_test7: str = "hello world".expandtabs(8)
assert tabs_test7 == "hello world", "expandtabs with no tabs should return same"

print("expandtabs() tests passed")

# str.splitlines() - basic usage
lines_test1: list[str] = "hello\nworld".splitlines()
assert lines_test1 == ["hello", "world"], "splitlines basic should work"

# str.splitlines() - with \r
lines_test2: list[str] = "a\rb\rc".splitlines()
assert lines_test2 == ["a", "b", "c"], "splitlines with \\r should work"

# str.splitlines() - with \r\n
lines_test3: list[str] = "line1\r\nline2\r\nline3".splitlines()
assert lines_test3 == ["line1", "line2", "line3"], "splitlines with \\r\\n should work"

# str.splitlines() - empty string
lines_test4: list[str] = "".splitlines()
assert lines_test4 == [], "splitlines empty should return empty list"

# str.splitlines() - single line no newline
lines_test5: list[str] = "hello".splitlines()
assert lines_test5 == ["hello"], "splitlines single line should work"

# str.splitlines() - ending with newline
lines_test6: list[str] = "hello\n".splitlines()
assert lines_test6 == ["hello"], "splitlines ending with newline should not add empty"

# str.splitlines() - mixed line endings
lines_test7: list[str] = "a\nb\rc\r\nd".splitlines()
assert lines_test7 == ["a", "b", "c", "d"], "splitlines mixed endings should work"

print("splitlines() tests passed")

# str.partition() - separator found
part_test1: tuple[str, str, str] = "hello-world".partition("-")
assert part_test1 == ("hello", "-", "world"), "partition found should work"

# str.partition() - separator not found
part_test2: tuple[str, str, str] = "hello".partition("-")
assert part_test2 == ("hello", "", ""), "partition not found should return (str, '', '')"

# str.partition() - separator at start
part_test3: tuple[str, str, str] = "-hello".partition("-")
assert part_test3 == ("", "-", "hello"), "partition at start should work"

# str.partition() - separator at end
part_test4: tuple[str, str, str] = "hello-".partition("-")
assert part_test4 == ("hello", "-", ""), "partition at end should work"

# str.partition() - multiple occurrences (first one)
part_test5: tuple[str, str, str] = "a-b-c".partition("-")
assert part_test5 == ("a", "-", "b-c"), "partition multiple should use first"

# str.partition() - multi-char separator
part_test6: tuple[str, str, str] = "hello::world".partition("::")
assert part_test6 == ("hello", "::", "world"), "partition multi-char should work"

print("partition() tests passed")

# str.rpartition() - separator found
rpart_test1: tuple[str, str, str] = "hello-world".rpartition("-")
assert rpart_test1 == ("hello", "-", "world"), "rpartition found should work"

# str.rpartition() - separator not found
rpart_test2: tuple[str, str, str] = "hello".rpartition("-")
assert rpart_test2 == ("", "", "hello"), "rpartition not found should return ('', '', str)"

# str.rpartition() - separator at start
rpart_test3: tuple[str, str, str] = "-hello".rpartition("-")
assert rpart_test3 == ("", "-", "hello"), "rpartition at start should work"

# str.rpartition() - separator at end
rpart_test4: tuple[str, str, str] = "hello-".rpartition("-")
assert rpart_test4 == ("hello", "-", ""), "rpartition at end should work"

# str.rpartition() - multiple occurrences (last one)
rpart_test5: tuple[str, str, str] = "a-b-c".rpartition("-")
assert rpart_test5 == ("a-b", "-", "c"), "rpartition multiple should use last"

# str.rpartition() - multi-char separator
rpart_test6: tuple[str, str, str] = "hello::world::test".rpartition("::")
assert rpart_test6 == ("hello::world", "::", "test"), "rpartition multi-char should work"

print("rpartition() tests passed")

# ===== SECTION: str.rfind() and str.rindex() =====

# rfind - basic
rfind_test1: int = "hello world hello".rfind("hello")
assert rfind_test1 == 12, f"rfind should find last occurrence, got {rfind_test1}"

rfind_test2: int = "hello world".rfind("xyz")
assert rfind_test2 == -1, "rfind should return -1 for missing"

rfind_test3: int = "abcabc".rfind("abc")
assert rfind_test3 == 3, "rfind should find last abc"

rfind_test4: int = "hello".rfind("")
assert rfind_test4 == 5, f"rfind empty should return len, got {rfind_test4}"

rfind_test5: int = "".rfind("")
assert rfind_test5 == 0, "rfind empty in empty should return 0"

print("rfind() tests passed")

# rindex - basic (same as rfind but raises ValueError on not found)
rindex_test1: int = "hello world hello".rindex("hello")
assert rindex_test1 == 12, f"rindex should find last occurrence, got {rindex_test1}"

rindex_test2: int = "abcabc".rindex("abc")
assert rindex_test2 == 3, "rindex should find last abc"

print("rindex() tests passed")

# ===== SECTION: str.index() =====
str_index_test1: int = "hello world".index("world")
assert str_index_test1 == 6, f"str.index should find substring, got {str_index_test1}"

str_index_test2: int = "abcabc".index("abc")
assert str_index_test2 == 0, "str.index should find first occurrence"

print("str.index() tests passed")

# ===== SECTION: str.rsplit() =====
rsplit_test1: list[str] = "a,b,c".rsplit(",")
assert rsplit_test1 == ["a", "b", "c"], f"rsplit basic should work, got {rsplit_test1}"

rsplit_test2: list[str] = "a,b,c,d".rsplit(",", 2)
assert rsplit_test2 == ["a,b", "c", "d"], f"rsplit with maxsplit should work, got {rsplit_test2}"

rsplit_test3: list[str] = "hello".rsplit(",")
assert rsplit_test3 == ["hello"], "rsplit no match should return whole string"

print("rsplit() tests passed")

# ===== SECTION: str.isascii() =====
isascii_test1: bool = "hello".isascii()
assert isascii_test1 == True, "ASCII string should be ascii"

isascii_test2: bool = "".isascii()
assert isascii_test2 == True, "empty string should be ascii"

isascii_test3: bool = "hello123!@#".isascii()
assert isascii_test3 == True, "ASCII with symbols should be ascii"

print("isascii() tests passed")

# ===== SECTION: str.encode() =====
encode_test1: bytes = "hello".encode()
assert encode_test1 == b"hello", "encode should produce bytes"

encode_test2: bytes = "".encode()
assert encode_test2 == b"", "encode empty should produce empty bytes"

encode_test3: bytes = "hello".encode("utf-8")
assert encode_test3 == b"hello", "encode utf-8 should work"

print("encode() tests passed")

# ===== SECTION: string module constants =====
import string
assert string.digits == "0123456789", "string.digits failed"
assert string.ascii_lowercase == "abcdefghijklmnopqrstuvwxyz", "string.ascii_lowercase failed"
assert string.ascii_uppercase == "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "string.ascii_uppercase failed"
assert string.ascii_letters == "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ", "string.ascii_letters failed"
assert string.hexdigits == "0123456789abcdefABCDEF", "string.hexdigits failed"
assert string.octdigits == "01234567", "string.octdigits failed"
print("string module constants tests passed")

# ===== SECTION: Chained method calls preserve types =====

_chain_s = "  Hello World  "
_chain_trimmed = _chain_s.strip().upper()
assert _chain_trimmed == "HELLO WORLD", "chained methods: strip().upper()"

_chain_parts = "a,b,c".split(",")
assert len(_chain_parts) == 3, "method return: split() → list[str]"
assert _chain_parts[0] == "a", "method return: split() first element"

print("Chained method type inference tests passed!")

print("All string tests passed!")
