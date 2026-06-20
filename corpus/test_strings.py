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

# = alignment: pad after sign/prefix
eq_sign: str = format(42, '+010')
assert eq_sign == "+000000042", f"= align sign: {eq_sign}"
eq_neg: str = format(-42, '010')
assert eq_neg == "-000000042", f"= align neg: {eq_neg}"
eq_hex: str = format(255, '#010x')
assert eq_hex == "0x000000ff", f"= align hex: {eq_hex}"
eq_bin: str = format(5, '#010b')
assert eq_bin == "0b00000101", f"= align bin: {eq_bin}"
eq_oct: str = format(8, '#010o')
assert eq_oct == "0o00000010", f"= align oct: {eq_oct}"
eq_neg_hex: str = format(-255, '#012x')
assert eq_neg_hex == "-0x0000000ff", f"= align neg hex: {eq_neg_hex}"

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

# Unicode numeric predicates — isdecimal ⊂ isdigit ⊂ isnumeric (§9, Numeric_Type).
# Superscript two (Numeric_Type=Digit): isdigit/isnumeric True, isdecimal False.
assert "²".isdigit() == True, "superscript-2 isdigit failed"
assert "²".isdecimal() == False, "superscript-2 isdecimal failed"
assert "²".isnumeric() == True, "superscript-2 isnumeric failed"
# Vulgar fraction one half (Numeric_Type=Numeric): only isnumeric True.
assert "½".isnumeric() == True, "1/2 isnumeric failed"
assert "½".isdigit() == False, "1/2 isdigit failed"
assert "½".isdecimal() == False, "1/2 isdecimal failed"
# Roman numeral eight (Numeric): only isnumeric.
assert "Ⅷ".isnumeric() == True, "roman-8 isnumeric failed"
assert "Ⅷ".isdigit() == False, "roman-8 isdigit failed"
# CJK numeral one (category Lo) — the old char::is_numeric path missed it.
assert "一".isnumeric() == True, "CJK-1 isnumeric failed"
assert "一".isdigit() == False, "CJK-1 isdigit failed"
# ASCII decimals satisfy all three; Arabic-Indic digits are decimal too.
assert "10".isdecimal() == True, "ascii isdecimal failed"
assert "10".isdigit() == True, "ascii isdigit failed"
assert "10".isnumeric() == True, "ascii isnumeric failed"
assert "٠١٢".isdecimal() == True, "arabic-indic isdecimal failed"
# Non-numeric and empty are False for all three.
assert "abc".isnumeric() == False, "letters isnumeric failed"
assert "".isdecimal() == False, "empty isdecimal failed"
assert "".isnumeric() == False, "empty isnumeric failed"
# A fraction beside a decimal: numeric overall, but not digit/decimal.
assert "1½".isnumeric() == True, "mixed isnumeric failed"
assert "1½".isdigit() == False, "mixed isdigit failed"
print("Unicode numeric predicate tests passed")

# Unicode alpha/case predicates — CPython parity beyond Rust's char::is_* (§9).
# isalpha is category L* only: Nl (Roman numerals) is NOT alpha.
assert "Ⅷ".isalpha() == False, "roman-8 isalpha failed"
assert "abⅧ".isalpha() == False, "mixed-roman isalpha failed"
assert "café".isalpha() == True, "accented isalpha failed"
# isalnum = isalpha OR any Numeric_Type, so the Roman numeral / fraction qualify.
assert "Ⅷ".isalnum() == True, "roman-8 isalnum failed"
assert "½".isalnum() == True, "fraction isalnum failed"
# Titlecase digraph ǅ (U+01C5) is neither upper nor lower and blocks isupper.
assert "ǅ".isupper() == False, "titlecase isupper failed"
assert "ǅ".islower() == False, "titlecase islower failed"
assert "Aǅ".isupper() == False, "titlecase-in-upper failed"
# The all-caps / all-lower digraph siblings ARE upper / lower.
assert "DŽ".isupper() == True, "caps-digraph isupper failed"
assert "ǆ".islower() == True, "lower-digraph islower failed"
print("Unicode alpha/case predicate tests passed")

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

# join() over non-list iterables (tuple/str/dict/set/tuple-var/deque): the
# arg is snapshotted to a list at lowering before rt_str_join reads it.
from collections import deque

join_tuple: str = ",".join(("a", "b", "c"))
assert join_tuple == "a,b,c", "join over fixed tuple should equal \"a,b,c\""
join_tuple_single: str = ",".join(("only",))
assert join_tuple_single == "only", "join over single-elem tuple should equal \"only\""
join_str_chars: str = ",".join("abc")
assert join_str_chars == "a,b,c", "join over str should join characters"
join_dict_keys: str = ",".join({"k1": 1, "k2": 2})
assert join_dict_keys == "k1,k2", "join over dict should join keys in insertion order"
join_tuple_var: tuple[str, ...] = ("p", "q", "r")
assert ",".join(join_tuple_var) == "p,q,r", "join over variable tuple should equal \"p,q,r\""
# set order is hash-table order (non-deterministic) — verify via sorted.
assert sorted(",".join({"a", "b", "c"}).split(",")) == ["a", "b", "c"], \
    "join over set should contain all elements"
join_deque: str = ",".join(deque(["x", "y", "z"]))
assert join_deque == "x,y,z", "join over deque should equal \"x,y,z\""

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

# ===== SECTION: f-string = (self-documenting / debug format, PEP 572) =====

# Basic types
dbg_int: int = 42
assert f"{dbg_int=}" == "dbg_int=42", f"f-string = with int failed: got {f'{dbg_int=}'}"

dbg_float: float = 3.14
assert f"{dbg_float=}" == "dbg_float=3.14", f"f-string = with float failed: got {f'{dbg_float=}'}"

dbg_str: str = "hello"
assert f"{dbg_str=}" == "dbg_str='hello'", f"f-string = with str failed: got {f'{dbg_str=}'}"

dbg_bool: bool = True
assert f"{dbg_bool=}" == "dbg_bool=True", f"f-string = with bool failed: got {f'{dbg_bool=}'}"

dbg_none_val: None = None
assert f"{dbg_none_val=}" == "dbg_none_val=None", f"f-string = with None failed: got {f'{dbg_none_val=}'}"

# With spaces around =
dbg_sp1: int = 10
assert f"{dbg_sp1 =}" == "dbg_sp1 =10", f"f-string = with leading space failed"
assert f"{dbg_sp1 = }" == "dbg_sp1 = 10", f"f-string = with spaces failed"

# With conversion flags
dbg_cv: str = "world"
assert f"{dbg_cv=!r}" == "dbg_cv='world'", f"f-string =!r failed: got {f'{dbg_cv=!r}'}"
assert f"{dbg_cv=!s}" == "dbg_cv=world", f"f-string =!s failed: got {f'{dbg_cv=!s}'}"
assert f"{dbg_cv=!a}" == "dbg_cv='world'", f"f-string =!a failed: got {f'{dbg_cv=!a}'}"

# With format spec
dbg_fval: float = 2.71828
assert f"{dbg_fval=:.2f}" == "dbg_fval=2.72", f"f-string = with format spec failed: got {f'{dbg_fval=:.2f}'}"

dbg_ival: int = 42
assert f"{dbg_ival=:>10}" == "dbg_ival=        42", f"f-string = with width failed"
assert f"{dbg_ival=:05d}" == "dbg_ival=00042", f"f-string = with zero-pad failed"

# Expressions
dbg_ex: int = 5
assert f"{dbg_ex + 1=}" == "dbg_ex + 1=6", f"f-string = with expression failed"
assert f"{dbg_ex * 2=}" == "dbg_ex * 2=10", f"f-string = with mul expression failed"

# Function calls
dbg_fname: str = "Alice"
assert f"{len(dbg_fname)=}" == "len(dbg_fname)=5", f"f-string = with function call failed"
assert f"{dbg_fname.upper()=}" == "dbg_fname.upper()='ALICE'", f"f-string = with method call failed"

# Multiple in one f-string
dbg_mx: int = 1
dbg_my: int = 2
assert f"{dbg_mx=}, {dbg_my=}" == "dbg_mx=1, dbg_my=2", f"f-string = multiple failed"

# In larger string context
assert f"Debug: {dbg_mx=}" == "Debug: dbg_mx=1", f"f-string = in context failed"

# List repr
dbg_lst: list[int] = [1, 2, 3]
assert f"{dbg_lst=}" == "dbg_lst=[1, 2, 3]", f"f-string = with list failed: got {f'{dbg_lst=}'}"

print("f-string = (debug format) tests passed")

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

# ===== SECTION: encode/decode errors= handlers and Unicode codecs (§9) =====
# errors= on a non-encodable character (é = U+00E9, latin-1 but not ascii).
assert "café".encode("ascii", "ignore") == b"caf", "encode ignore"
assert "café".encode("ascii", "replace") == b"caf?", "encode replace"
assert "café".encode("ascii", "backslashreplace") == b"caf\\xe9", "encode backslashreplace"
assert "café".encode("ascii", "xmlcharrefreplace") == b"caf&#233;", "encode xmlcharrefreplace"
# Keyword form of errors= (and encoding=).
assert "café".encode("ascii", errors="ignore") == b"caf", "encode errors= kwarg"
assert "café".encode(encoding="latin-1") == b"caf\xe9", "encode encoding= kwarg"
# decode errors= on an undecodable byte.
assert b"caf\xe9".decode("ascii", "ignore") == "caf", "decode ignore"
assert b"caf\xe9".decode("ascii", "replace") == "caf�", "decode replace"
assert b"caf\xe9".decode("ascii", errors="backslashreplace") == "caf\\xe9", "decode backslashreplace"
assert b"caf\xe9".decode("latin-1") == "café", "decode latin-1"
assert b"\xff\xfe".decode("utf-8", "replace") == "��", "decode utf-8 replace"
# Unicode codecs: explicit byte order (host-independent) and round trips.
assert "AB".encode("utf-16-le") == b"A\x00B\x00", "utf-16-le encode"
assert "AB".encode("utf-16-be") == b"\x00A\x00B", "utf-16-be encode"
assert "A".encode("utf-32-be") == b"\x00\x00\x00A", "utf-32-be encode"
assert b"\xff\xfeA\x00".decode("utf-16") == "A", "utf-16 LE BOM decode"
assert b"\xfe\xff\x00A".decode("utf-16") == "A", "utf-16 BE BOM decode"
_codec_s = "Hello, ☃ мир 𝕏!"
for _enc in ("utf-16", "utf-16-le", "utf-16-be", "utf-32", "utf-32-le", "utf-32-be"):
    assert _codec_s.encode(_enc).decode(_enc) == _codec_s, "codec round trip"
# An unrecognized encoding name raises LookupError in both directions.
try:
    "x".encode("definitely-not-a-codec")
    assert False, "unknown encoding should raise"
except LookupError:
    pass
try:
    b"x".decode("definitely-not-a-codec")
    assert False, "unknown decoding should raise"
except LookupError:
    pass
# strict (the default) raises on out-of-range data.
try:
    "café".encode("ascii")
    assert False, "ascii encode should raise"
except UnicodeEncodeError:
    pass
try:
    b"\xff".decode("utf-8")
    assert False, "utf-8 decode should raise"
except UnicodeDecodeError:
    pass
print("encode/decode errors and codec tests passed")

# ===== SECTION: single-byte mapping codecs (§9) =====
# Round trips across a representative spread of single-byte charmap codecs.
_sb_latin2 = "Příliš žluťoučký kůň"          # Czech (Latin-2)
assert _sb_latin2.encode("iso-8859-2").decode("iso-8859-2") == _sb_latin2, "iso-8859-2 round trip"
_sb_cyr = "Привет, мир"                       # Russian
for _enc in ("iso-8859-5", "cp1251", "koi8-r", "cp866", "mac-cyrillic"):
    assert _sb_cyr.encode(_enc).decode(_enc) == _sb_cyr, "cyrillic round trip"
_sb_polish = "Zażółć gęślą jaźń"
assert _sb_polish.encode("cp1250").decode("cp1250") == _sb_polish, "cp1250 round trip"
assert "Ελληνικά".encode("iso-8859-7").decode("iso-8859-7") == "Ελληνικά", "greek round trip"

# Exact byte mappings (high half differs per codec — pin a few).
assert "€".encode("cp1252") == b"\x80", "cp1252 euro at 0x80"
assert "€".encode("iso-8859-15") == b"\xa4", "latin-9 euro at 0xa4"
assert b"\xc0".decode("iso-8859-5") == "Р", "iso-8859-5 0xc0 -> Cyrillic R"
assert "ÿ".encode("latin-1") == b"\xff" and b"\xff".decode("mac-roman") == "ˇ", "mac-roman 0xff"

# Aliases resolve like CPython (normalized: lowercase, no -/_/space).
assert "č".encode("latin2") == "č".encode("iso8859-2"), "latin2 alias == iso8859-2"
assert "Ж".encode("windows-1251") == "Ж".encode("cp1251"), "windows-1251 alias"
assert b"\xe6".decode("KOI8-R") == b"\xe6".decode("koi8_r"), "koi8 case-insensitive + alias"

# errors= handlers on an unencodable character (☃ is in none of these).
assert "ab☃c".encode("iso-8859-2", "ignore") == b"abc", "single-byte encode ignore"
assert "ab☃c".encode("iso-8859-2", "replace") == b"ab?c", "single-byte encode replace"
assert "ab☃c".encode("iso-8859-2", errors="backslashreplace") == b"ab\\u2603c", "encode backslashreplace"

# decode of an undefined byte: strict raises, replace substitutes U+FFFD.
# 0x98 is the one byte cp1251 leaves undefined.
assert b"a\x98b".decode("cp1251", "replace") == "a�b", "single-byte decode replace"
assert b"a\x98b".decode("cp1251", "ignore") == "ab", "single-byte decode ignore"
try:
    b"\x98".decode("cp1251")
    assert False, "undefined byte should raise"
except UnicodeDecodeError:
    pass
# strict encode of an unmappable character raises UnicodeEncodeError.
try:
    "☃".encode("koi8-r")
    assert False, "unmappable encode should raise"
except UnicodeEncodeError:
    pass
print("single-byte codec tests passed")

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

# ===== Section: String ordering comparisons (lexicographic via rt_obj_cmp) =====
# Ordering ops `< > <= >=` on `str` operands route through `rt_obj_cmp`
# (which takes an op_tag third arg); regression-guards the comparison
# lowering's op_tag and the runtime's lexicographic Str arm.
_ord_a: str = "abc"
_ord_b: str = "abd"
assert (_ord_a < _ord_b) is True, "str <"
assert (_ord_a > _ord_b) is False, "str >"
assert (_ord_a <= _ord_a) is True, "str <="
assert (_ord_b >= _ord_a) is True, "str >="
_ord_words = ["banana", "apple", "cherry"]
_ord_min = _ord_words[0]
for _w in _ord_words:
    if _w < _ord_min:
        _ord_min = _w
assert _ord_min == "apple", f"str ordering manual min: {_ord_min}"
print("String ordering comparison tests passed!")

# Regression: min()/max() over a str-element iterable must type the result as
# the element type (str), not Int — otherwise the str pointer prints/compares
# as a raw integer. Covers list, tuple, set, deque, generator, and the
# multi-arg (variadic) form; mirrors the bytes coverage in
# test_collections_dict_set_bytes.py.
_mm_list: list[str] = ["c", "a", "b"]
assert min(_mm_list) == "a", f"min(list[str]): {min(_mm_list)}"
assert max(_mm_list) == "c", f"max(list[str]): {max(_mm_list)}"
_mm_tuple: tuple[str, str, str] = ("c", "a", "b")
assert min(_mm_tuple) == "a", "min(tuple[str])"
assert max(_mm_tuple) == "c", "max(tuple[str])"
_mm_set: set[str] = {"c", "a", "b"}
assert min(_mm_set) == "a", "min(set[str])"
assert max(_mm_set) == "c", "max(set[str])"
assert min(c for c in "cab") == "a", "min(str generator)"
assert max(c for c in "cab") == "c", "max(str generator)"
assert min("a", "b") == "a", "min(variadic str)"
assert max("a", "b") == "b", "max(variadic str)"
# Result is a real str (heap), usable in str context.
assert min(_mm_list) + "!" == "a!", "min(list[str]) concat"
print("min/max over str iterables tests passed!")

# Code-review regression: str.split/rsplit(None, maxsplit) keep the remainder
# (with interior whitespace) instead of dropping middle words; formerly part
# of test_review_wave2_runtime.py.
def _rv_str_split_maxsplit() -> None:
    print("a b c".split(None, 1))
    print("a b c".rsplit(None, 1))
    print("a b c".split(None, 0))
    print("  a  b  c  ".split(None, 1))
    print("  a  b  c  ".rsplit(None, 1))
    print("a b c d".split(None, 2))
    print("a b c d".rsplit(None, 2))
    print("a-b-c-d".split("-", 1))
    print("a-b-c-d".rsplit("-", 1))


_rv_str_split_maxsplit()

# ===== SECTION: Folded p19 — str methods (split/replace/strip/partition/encode/predicates) =====
# Folded from corpus/p19_str_methods.py. Cyrillic / café inputs exercise the
# codepoint char_len recount paths; predicates here are the ASCII-agreeing set.
def _fold_p19_str_methods() -> None:
    # split: whitespace / sep / maxsplit (RAW slot) / Cyrillic
    assert "  a b  c  ".split() == ["a", "b", "c"]
    assert "a,b,c".split(",") == ["a", "b", "c"]
    assert "a,b,c,d".split(",", 2) == ["a", "b", "c,d"]
    assert "".split() == []
    assert "раз два три".split() == ["раз", "два", "три"]
    assert "мир,труд,май".split(",") == ["мир", "труд", "май"]
    # rsplit: sep+maxsplit / no-arg whitespace / explicit None / Cyrillic
    assert "a-b-c".rsplit("-", 1) == ["a-b", "c"]
    assert "one two three".rsplit() == ["one", "two", "three"]
    assert "a b c".rsplit(None, 1) == ["a b", "c"]
    assert "раз-два-три".rsplit("-", 1) == ["раз-два", "три"]
    # splitlines: \n / \r\n / \r mix / Cyrillic / empty
    assert "a\nb\nc".splitlines() == ["a", "b", "c"]
    assert "a\r\nb\rc\n".splitlines() == ["a", "b", "c"]
    assert "".splitlines() == []
    assert "one line".splitlines() == ["one line"]
    assert "привет\nмир".splitlines() == ["привет", "мир"]
    # replace: ASCII / byte-ratio change (café → recount) / growth
    assert "a,b,c".replace(",", ";") == "a;b;c"
    assert "café".replace("é", "e") == "cafe"
    assert len("café".replace("é", "e")) == 4
    assert "aaa".replace("a", "bb") == "bbbbbb"
    assert "hello".replace("l", "L") == "heLLo"
    # lstrip / rstrip: whitespace + chars-set, codepoint-correct len
    assert "  hi  ".lstrip() == "hi  "
    assert "  hi  ".rstrip() == "  hi"
    assert "xxcafé".lstrip("x") == "café"
    assert "caféxx".rstrip("x") == "café"
    assert len("  café".lstrip()) == 4
    assert len("café  ".rstrip()) == 4
    assert "café".lstrip(None) == "café"
    # removeprefix / removesuffix: exact char_len subtract (Cyrillic)
    assert "foobar".removeprefix("foo") == "bar"
    assert "foobar".removesuffix("bar") == "foo"
    assert "hello".removeprefix("xyz") == "hello"
    assert "мир".removeprefix("ми") == "р"
    assert len("мир".removeprefix("ми")) == 1
    # expandtabs: RAW tabsize / default 8 / Cyrillic + tab
    assert "a\tb".expandtabs(4) == "a   b"
    assert "a\tb".expandtabs() == "a       b"
    assert "a\tб".expandtabs(4) == "a   б"
    assert "\tб".expandtabs(2) == "  б"
    # partition / rpartition: 3-tuple unpack through the gradual seam
    a, sep, b = "key=value".partition("=")
    assert a == "key" and sep == "=" and b == "value"
    assert "no-sep".partition("=") == ("no-sep", "", "")
    assert "a=b=c".partition("=") == ("a", "=", "b=c")
    assert "a=b=c".rpartition("=") == ("a=b", "=", "c")
    ca, csep, cb = "имя=значение".partition("=")
    assert ca == "имя" and csep == "=" and cb == "значение"
    # encode: utf-8 bytes, codepoint↔byte length
    assert "café".encode() == b"caf\xc3\xa9"
    assert len("café".encode()) == 5
    assert "x".encode("utf-8") == b"x"
    assert "abc".encode() == b"abc"
    # rindex: found (codepoint offset) / Cyrillic / miss → ValueError
    assert "abcabc".rindex("b") == 4
    assert "abcabc".rindex("a") == 3
    assert "абвабв".rindex("б") == 4
    miss_caught = False
    try:
        "abc".rindex("z")
    except ValueError:
        miss_caught = True
    assert miss_caught
    # predicates: ASCII-only (non-ASCII diverges → kept out)
    assert "123".isdigit() == True
    assert "12a".isdigit() == False
    assert "abc".isalpha() == True
    assert "abc1".isalpha() == False
    assert "abc123".isalnum() == True
    assert "abc!".isalnum() == False
    assert " \t".isspace() == True
    assert "a b".isspace() == False
    assert "ABC".isupper() == True
    assert "Abc".isupper() == False
    assert "abc".islower() == True
    assert "Abc".islower() == False
    assert "abc".isascii() == True
    assert "café".isascii() == False
    # interaction probes (cross green features)
    parts = "a,b,c".split(",")
    assert len(parts[0]) == 1
    assert f"first={parts[0]}" == "first=a"
    total = 0
    for x in "1,2,3,4".split(","):
        total += int(x)
    assert total == 10
    if "42".isdigit():
        flag = "numeric"
    else:
        flag = "other"
    assert flag == "numeric"
    joined = "-".join("a,b,c".split(","))
    assert joined == "a-b-c"


_fold_p19_str_methods()


# ===== SECTION: Folded p20 — bytes methods (startswith/find/count/replace/split/strip/join/decode) =====
# Folded from corpus/p20_bytes_methods.py. Non-ASCII byte content (b"\xc3\xa9")
# exercises the byte-accurate (non-codepoint) paths.
def _fold_p20_bytes_methods() -> None:
    # startswith / endswith
    assert b"hello".startswith(b"he") == True
    assert b"hello".startswith(b"lo") == False
    assert b"hello".endswith(b"lo") == True
    assert b"hello".endswith(b"he") == False
    assert b"caf\xc3\xa9".startswith(b"caf") == True
    # find / rfind (dedicated 2-arg fns, byte offsets)
    assert b"abcabc".find(b"b") == 1
    assert b"abcabc".rfind(b"b") == 4
    assert b"abcabc".find(b"z") == -1
    assert b"caf\xc3\xa9".find(b"\xc3\xa9") == 3
    # count
    assert b"abcabc".count(b"a") == 2
    assert b"aaaa".count(b"aa") == 2
    assert b"abc".count(b"z") == 0
    # replace (2-arg, no count)
    assert b"a,b,c".replace(b",", b";") == b"a;b;c"
    assert b"aaa".replace(b"a", b"bb") == b"bbbbbb"
    assert b"caf\xc3\xa9".replace(b"\xc3\xa9", b"e") == b"cafe"
    # split: whitespace / sep / maxsplit (RAW slot) / explicit None
    assert b"a,b,c".split(b",") == [b"a", b"b", b"c"]
    assert b"  a b  c  ".split() == [b"a", b"b", b"c"]
    assert b"a,b,c,d".split(b",", 1) == [b"a", b"b,c,d"]
    assert b"a b c".split(None) == [b"a", b"b", b"c"]
    assert b"".split() == []
    # rsplit: sep+maxsplit / no-arg whitespace
    assert b"a-b-c".rsplit(b"-", 1) == [b"a-b", b"c"]
    assert b"one two three".rsplit() == [b"one", b"two", b"three"]
    assert b"a b c".rsplit(None, 1) == [b"a b", b"c"]
    # strip / lstrip / rstrip (whitespace only — no chars)
    assert b"  hi  ".strip() == b"hi"
    assert b"  hi  ".lstrip() == b"hi  "
    assert b"  hi  ".rstrip() == b"  hi"
    assert b"\t\n x \r\n".strip() == b"x"
    # upper / lower (ASCII-only; non-ASCII bytes pass through)
    assert b"Hello".upper() == b"HELLO"
    assert b"Hello".lower() == b"hello"
    assert b"caf\xc3\xa9".upper() == b"CAF\xc3\xa9"
    # join (materializes the iterable, like str.join)
    assert b",".join([b"a", b"b", b"c"]) == b"a,b,c"
    assert b"".join([b"x", b"y", b"z"]) == b"xyz"
    assert b"-".join([b"solo"]) == b"solo"
    # decode round-trip with str.encode (§9-str)
    assert b"caf\xc3\xa9".decode() == "café"
    assert "café".encode() == b"caf\xc3\xa9"
    assert "café".encode().decode() == "café"
    assert b"abc".decode() == "abc"
    assert b"abc".decode("utf-8") == "abc"
    # interaction probes (cross green features; bytes NOT in an f-string)
    assert len(b"hello") == 5
    total = 0
    for byte in b"abc":
        total += byte
    assert total == 97 + 98 + 99
    parts = b"10,20,30".split(b",")
    assert len(parts) == 3
    acc = 0
    for p in parts:
        acc += len(p)
    assert acc == 6
    assert b",".join(b"a,b,c".split(b",")) == b"a,b,c"
    assert (b"ana" in b"banana") == True
    assert (b"xyz" in b"banana") == False
    assert (b"" in b"banana") == True
    assert (98 in b"abc") == True  # 98 == ord('b')
    assert (b"banana".find(b"a") != -1) == True
    assert b"banana".count(b"a") == 3


_fold_p20_bytes_methods()


# ===== SECTION: Folded p49 — str/bytes method arguments (replace count, find/index start/end, encode/decode) =====
# Folded from corpus/p49_str_method_args.py. count + search start/end ride RAW
# i64 slots; encode/decode honor utf-8/ascii/latin-1 and raise on out-of-range.
def _fold_p49_str_method_args() -> None:
    # replace count (str)
    assert "aaaa".replace("a", "b", 2) == "bbaa"
    assert "aaaa".replace("a", "b") == "bbbb"
    assert "aaaa".replace("a", "b", 0) == "aaaa"
    assert "aaaa".replace("a", "b", 100) == "bbbb"
    assert "abc".replace("", "X", 2) == "XaXbc"
    assert "abc".replace("", "X") == "XaXbXcX"
    assert "one two one two one".replace("one", "1", 1) == "1 two one two one"
    assert "café cafē".replace("caf", "C", 1) == "Cé cafē"
    # replace count (bytes)
    assert b"aaaa".replace(b"a", b"b", 2) == b"bbaa"
    assert b"aaaa".replace(b"a", b"b") == b"bbbb"
    assert b"xyxyxy".replace(b"xy", b"Z", 2) == b"ZZxy"
    # find / rfind / index / rindex with start / end (str)
    s = "abcabcabc"
    assert s.find("bc", 2) == 4
    assert s.find("bc", 2, 4) == -1
    assert s.find("bc", 2, 5) == -1
    assert s.find("abc", -3) == 6
    assert s.rfind("bc", 0, 4) == 1
    assert s.rfind("bc") == 7
    assert s.rfind("bc", 0, -1) == 4
    assert s.index("bc", 3) == 4
    assert s.rindex("bc", 0, 7) == 4
    assert s.find("zzz") == -1
    assert s.find("bc", 100) == -1
    assert "café".find("é") == 3
    assert "café".find("é", 2) == 3
    assert "café".find("f", 0, 3) == 2
    assert "café".find("f", 0, 2) == -1
    assert "naïve naïve".find("ï", 4) == 8
    # index / rindex miss → ValueError (assert the caught outcome)
    def index_miss(fn) -> bool:
        try:
            fn()
            return False
        except ValueError:
            return True
    assert index_miss(lambda: "abc".index("z", 0, 2))
    assert index_miss(lambda: "abcabc".rindex("z"))
    assert index_miss(lambda: "abcabc".index("bc", 0, 2))
    # find / rfind with start / end (bytes)
    bs = b"abcabc"
    assert bs.find(b"bc", 2) == 4
    assert bs.find(b"bc", 2, 4) == -1
    assert bs.find(b"bc", -4) == 4
    assert bs.rfind(b"bc", 0, 4) == 1
    assert bs.rfind(b"bc") == 4
    assert bs.count(b"bc") == 2
    # encode / decode correct paths
    assert "café".encode("utf-8") == b"caf\xc3\xa9"
    assert "café".encode("UTF_8") == b"caf\xc3\xa9"
    assert "hello".encode("ascii") == b"hello"
    assert "café".encode("latin-1") == b"caf\xe9"
    assert "café".encode("latin1") == b"caf\xe9"
    assert b"caf\xc3\xa9".decode("utf-8") == "café"
    assert b"hello".decode("ascii") == "hello"
    assert b"caf\xe9".decode("latin-1") == "café"
    assert b"caf\xe9".decode("iso-8859-1") == "café"
    assert "Ωmega".encode("utf-8").decode("utf-8") == "Ωmega"
    # encode / decode error paths (assert the caught outcome; precise type AND a
    # super-catch through the MRO, e.g. UnicodeError ⊂ ValueError).
    enc_ascii_unicode = False
    try:
        "café".encode("ascii")
    except UnicodeError:
        enc_ascii_unicode = True
    assert enc_ascii_unicode
    enc_ascii_value = False
    try:
        "café".encode("ascii")
    except ValueError:
        enc_ascii_value = True
    assert enc_ascii_value
    enc_latin1_unicode = False
    try:
        "€".encode("latin-1")
    except UnicodeError:
        enc_latin1_unicode = True
    assert enc_latin1_unicode
    enc_unknown_lookup = False
    try:
        "x".encode("zzz-codec")
    except LookupError:
        enc_unknown_lookup = True
    assert enc_unknown_lookup
    dec_utf8_unicode = False
    try:
        b"\xff\xfe".decode("utf-8")
    except UnicodeDecodeError:
        dec_utf8_unicode = True
    assert dec_utf8_unicode
    dec_utf8_value = False
    try:
        b"\xff\xfe".decode("utf-8")
    except ValueError:
        dec_utf8_value = True
    assert dec_utf8_value
    dec_ascii_unicode = False
    try:
        b"\xe9".decode("ascii")
    except UnicodeError:
        dec_ascii_unicode = True
    assert dec_ascii_unicode
    dec_unknown_lookup = False
    try:
        b"x".decode("zzz-codec")
    except LookupError:
        dec_unknown_lookup = True
    assert dec_unknown_lookup


_fold_p49_str_method_args()


# ===== SECTION: Folded p50 — Unicode-aware predicates (isalpha/isalnum/isupper/islower/isdigit/isspace) =====
# Folded from corpus/p50_unicode_predicates.py. Restricted to codepoints where
# Rust char::is_* and CPython categories agree (Latin/Cyrillic/Greek/ASCII).
def _fold_p50_unicode_predicates() -> None:
    # isalpha (accented Latin, Cyrillic, Greek)
    assert "café".isalpha() == True
    assert "über".isalpha() == True
    assert "Привет".isalpha() == True
    assert "Ωμέγα".isalpha() == True
    assert "naïve".isalpha() == True
    assert "café!".isalpha() == False
    assert "abc".isalpha() == True
    assert "abc1".isalpha() == False
    assert "".isalpha() == False
    # isupper / islower (Unicode case)
    assert "Ñ".isupper() == True
    assert "ñ".islower() == True
    assert "ÜBER".isupper() == True
    assert "über".islower() == True
    assert "ПРИВЕТ".isupper() == True
    assert "привет".islower() == True
    assert "Привет".isupper() == False
    assert "Привет".islower() == False
    assert "ABC".isupper() == True
    assert "abc".islower() == True
    assert "Abc".isupper() == False
    assert "ÅÄÖ".isupper() == True
    assert "123".isupper() == False
    assert "".isupper() == False
    # isalnum (letters + digits across scripts)
    assert "café123".isalnum() == True
    assert "Привет42".isalnum() == True
    assert "abc123".isalnum() == True
    assert "abc 123".isalnum() == False
    assert "".isalnum() == False
    # isdigit (ASCII digits — the agreeing set)
    assert "123".isdigit() == True
    assert "0".isdigit() == True
    assert "12a".isdigit() == False
    assert "".isdigit() == False
    # isspace (Unicode whitespace — space / NBSP / thin-space are distinct cases)
    assert " ".isspace() == True
    assert "\t\n\r ".isspace() == True
    assert " ".isspace() == True  # U+00A0 no-break space
    assert " ".isspace() == True  # U+2009 thin space
    assert "a b".isspace() == False
    assert "".isspace() == False
    # isascii (unchanged)
    assert "hello".isascii() == True
    assert "café".isascii() == False
    assert "".isascii() == True


_fold_p50_unicode_predicates()


# ===== SECTION: Folded p8h — codepoint-correct string model (len/slice/iter/case/align) =====
# Folded from corpus/p8h_unicode.py. Cyrillic + emoji exercise codepoint
# indexing, slicing, iteration, reversal, case folding, and width alignment.
def _fold_p8h_unicode() -> None:
    s = "привет"
    assert len(s) == 6
    assert s[0] == "п"
    assert s[2] == "и"
    assert s[-1] == "т"
    assert s[-3] == "в"
    assert s[1:4] == "рив"
    assert s[:3] == "при"
    assert s[3:] == "вет"
    assert s[-4:-1] == "иве"
    assert s[::-1] == "тевирп"
    assert s[::2] == "пие"
    assert s[1::2] == "рвт"
    assert s[5:1:-1] == "теви"

    m = "aбвgд"  # mixed ASCII + Cyrillic
    assert len(m) == 5
    assert m[1] == "б"
    assert m[3] == "g"
    assert m[::-1] == "дgвбa"

    e = "x😀y"
    assert len(e) == 3
    assert e[0] == "x"
    assert e[1] == "😀"
    assert e[2] == "y"
    assert e[::-1] == "y😀x"
    assert e[1:] == "😀y"

    # Iteration walks codepoints
    s_chars = []
    for ch in s:
        s_chars.append(ch)
    assert s_chars == ["п", "р", "и", "в", "е", "т"]
    chars = []
    for ch in e:
        chars.append(ch)
    assert chars == ["x", "😀", "y"]

    # reversed()
    rev_chars = []
    for ch in reversed(s):
        rev_chars.append(ch)
    assert rev_chars == ["т", "е", "в", "и", "р", "п"]

    # find / index / count are codepoint-based
    assert s.find("вет") == 3
    assert s.index("р") == 1
    assert "абабаб".count("аб") == 3
    assert s.find("нет") == -1

    # in / concat
    assert ("ив" in s) == True
    assert ("xy" in e) == False
    assert s + "!" + e == "привет!x😀y"

    # case conversions (Unicode-aware)
    assert s.upper() == "ПРИВЕТ"
    assert "ПРИВЕТ".lower() == "привет"
    assert "straße".upper() == "STRASSE"
    assert "привет мир".title() == "Привет Мир"
    assert "привет".capitalize() == "Привет"
    assert "ПрИвЕт".swapcase() == "пРиВеТ"

    # alignment widths are in characters
    assert "[" + "пр".center(6) + "]" == "[  пр  ]"
    assert "[" + "пр".ljust(5, "-") + "]" == "[пр---]"
    assert "[" + "пр".rjust(5, "*") + "]" == "[***пр]"
    assert "[" + "ab".center(5) + "]" == "[  ab ]"
    assert "-42".zfill(6) == "-00042"
    assert "пр".zfill(4) == "00пр"

    # ord/chr round-trip
    assert ord("ё") == 1105
    assert chr(1105) == "ё"
    assert ord("😀") == 128512
    assert chr(128512) == "😀"

    # f-strings with unicode values
    name = "мир"
    assert f"привет, {name}!" == "привет, мир!"
    assert f"{name:>6}" == "   мир"
    assert f"{name:*^7}" == "**мир**"


_fold_p8h_unicode()


# User classes for the folded format suites below. The compiler's typed subset
# does not support nested class definitions, so these are hoisted to module
# scope (formerly local to test_format_spec.py / p29_format.py).
class _FmtPoint:
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def __format__(self, spec: str) -> str:
        if spec == "polar":
            return "Point(" + str(self.x) + "," + str(self.y) + ")"
        return "(" + str(self.x) + ", " + str(self.y) + ")"


class _FmtTemperature:
    def __init__(self, celsius: float):
        self.celsius = celsius

    def __format__(self, spec: str) -> str:
        if spec == "F":
            return str(self.celsius * 9 / 5 + 32) + "F"
        return str(self.celsius) + "C"


class _FmtLabelled:
    def __init__(self, label: str):
        self.label = label

    def __str__(self) -> str:
        return "label:" + self.label


# ===== SECTION: Folded test_format_spec — PEP 3101 format spec mini-language =====
# Folded from corpus/test_format_spec.py. Integer/float type chars, width/align,
# sign, zero-pad, grouping, truncation, bool, nested specs, format()/.format().
def _fold_test_format_spec() -> None:
    # Integer type characters
    assert f"{42:d}" == "42"
    assert f"{42:b}" == "101010"
    assert f"{42:o}" == "52"
    assert f"{42:x}" == "2a"
    assert f"{42:X}" == "2A"
    assert f"{42:#x}" == "0x2a"
    assert f"{42:#b}" == "0b101010"
    assert f"{42:#o}" == "0o52"
    assert f"{255:x}" == "ff"
    assert f"{255:X}" == "FF"
    assert f"{0:d}" == "0"
    assert f"{-42:d}" == "-42"
    # Width and alignment
    assert f"{42:5}" == "   42"
    assert f"{42:<5}" == "42   "
    assert f"{42:>5}" == "   42"
    assert f"{42:^5}" == " 42  "
    assert f"{'hi':>10}" == "        hi"
    assert f"{'hi':<10}" == "hi        "
    assert f"{'hi':^10}" == "    hi    "
    assert f"{'a':*^7}" == "***a***"
    # Sign
    assert f"{42:+d}" == "+42"
    assert f"{-42:+d}" == "-42"
    assert f"{42: d}" == " 42"
    assert f"{-42: d}" == "-42"
    # Zero-padding
    assert f"{42:05d}" == "00042"
    assert f"{-42:05d}" == "-0042"
    assert f"{42:08b}" == "00101010"
    # Grouping separators
    assert f"{1234567:,}" == "1,234,567"
    assert f"{1234567:_}" == "1_234_567"
    assert f"{1234567.89:,.2f}" == "1,234,567.89"
    assert f"{1000000:,d}" == "1,000,000"
    # Float type characters
    assert f"{3.14:.2f}" == "3.14"
    assert f"{0.0001234:.2e}" == "1.23e-04"
    assert f"{0.25:.1%}" == "25.0%"
    assert f"{1.0:.0f}" == "1"
    assert f"{3.14159:.4f}" == "3.1416"
    assert f"{1234.5:10.2f}" == "   1234.50"
    assert f"{0.0:.4f}" == "0.0000"
    # String truncation (precision)
    assert f"{'abcdef':.3}" == "abc"
    assert f"{'hello':10}" == "hello     "
    assert f"{'hello':>10}" == "     hello"
    # Bool formatting (treated as int subclass for numeric specs)
    assert f"{True:5}" == "    1"
    assert f"{False:5}" == "    0"
    assert f"{True:d}" == "1"
    assert f"{False:d}" == "0"
    # §F.2 regression: non-literal expression must get correct alignment
    def fmt_int_var(x: int) -> str:
        return f"{x:5d}"
    assert fmt_int_var(2) == "    2"
    assert fmt_int_var(1000) == " 1000"
    assert fmt_int_var(-3) == "   -3"
    def fmt_float_var(x: float) -> str:
        return f"{x:.3f}"
    assert fmt_float_var(3.14159) == "3.142"
    assert fmt_float_var(0.0) == "0.000"
    # Nested (dynamic) format specs
    n = 4
    assert f"{3.14159:.{n}f}" == "3.1416"
    width = 8
    assert f"{42:{width}d}" == "      42"
    # format() builtin — parallel with f-strings
    assert format(42, "5d") == "   42"
    assert format(3.14, ".2f") == "3.14"
    assert format("hello", ">10") == "     hello"
    assert format(True, "d") == "1"
    assert format(255, "x") == "ff"
    assert format(1234567, ",") == "1,234,567"
    # .format() method — positional and keyword
    assert "Hello {:>10}!".format("world") == "Hello      world!"
    assert "{0:5d} + {1:5d}".format(1, 2) == "    1 +     2"
    assert "{name:>10}".format(name="Alice") == "     Alice"
    assert "{:.4f}".format(3.14159) == "3.1416"
    # User-class __format__ (class hoisted to module scope: _FmtPoint)
    p = _FmtPoint(3, 4)
    assert f"{p:polar}" == "Point(3,4)"
    assert f"{p}" == "(3, 4)"
    assert format(p, "polar") == "Point(3,4)"


_fold_test_format_spec()


# ===== SECTION: Folded p29 — .format() / !a-ascii() / dynamic specs / user __format__ =====
# Folded from corpus/p29_format.py. Auto/manual/keyword positional fields, brace
# escapes, conversion flags, ascii() builtin + !a, dynamic specs, user classes.
# A few specs (e.g. the "Hello {:>10}!" .format() group, format()/ascii bool
# cases) overlap the test_format_spec fold but are kept here so this function
# probes the full auto/manual/keyword field-numbering surface as one unit.
def _fold_p29_format() -> None:
    # str.format(): auto-indexed positional fields
    assert "{}".format("only") == "only"
    assert "{} + {} = {}".format(1, 2, 3) == "1 + 2 = 3"
    assert "Hello {}!".format("world") == "Hello world!"
    assert "{} and {}".format("a", "b") == "a and b"
    # str.format(): explicit (manual) positional indices, including reuse + reorder
    assert "{0}".format("x") == "x"
    assert "{0} {1} {0}".format("a", "b") == "a b a"
    assert "{2}{1}{0}".format("c", "b", "a") == "abc"
    assert "{1} + {0} = {2}".format(1, 2, 3) == "2 + 1 = 3"
    # str.format(): keyword fields, and mixed positional + keyword
    assert "{name}".format(name="Alice") == "Alice"
    assert "{x} and {y} and {x}".format(x="foo", y="bar") == "foo and bar and foo"
    assert "{0} and {name}".format("first", name="second") == "first and second"
    assert "{0}: {item}, {1}: {value}".format("key", "val", item="apple", value=42) == "key: apple, val: 42"
    # str.format(): brace escapes and zero placeholders
    assert "No placeholders".format() == "No placeholders"
    assert "".format() == ""
    assert "Use {{}} for placeholders".format() == "Use {} for placeholders"
    assert "{{{}}}".format(42) == "{42}"
    assert "{{literal}}".format() == "{literal}"
    # str.format(): static format specs inside fields (auto / indexed / keyword)
    assert "Hello {:>10}!".format("world") == "Hello      world!"
    assert "{0:5d} + {1:5d}".format(1, 2) == "    1 +     2"
    assert "{name:>10}".format(name="Alice") == "     Alice"
    assert "{:.4f}".format(3.14159) == "3.1416"
    assert "{:*^7}".format("a") == "***a***"
    assert "{0:>5}{1:<5}".format("x", "y") == "    xy    "
    assert "{val:08.2f}".format(val=3.14159) == "00003.14"
    assert "{:x}".format(255) == "ff"
    assert "{:,}".format(1234567) == "1,234,567"
    # str.format(): conversion flags inside fields
    assert "{!r}".format("hi") == "'hi'"
    assert "{0!r}".format("hi") == "'hi'"
    assert "{name!r}".format(name="hi") == "'hi'"
    assert "{!s}".format(42) == "42"
    assert "{!r} and {!s}".format("a", "b") == "'a' and b"
    # format() builtin (single + spec)
    assert format(42) == "42"
    assert format(3.14) == "3.14"
    assert format("hi") == "hi"
    assert format(42, "5d") == "   42"
    assert format(3.14159, ".2f") == "3.14"
    assert format("hello", ">10") == "     hello"
    assert format(255, "#x") == "0xff"
    assert format(True, "d") == "1"
    assert format(7, "05") == "00007"
    # ascii() builtin and the f-string `!a` conversion (ASCII content)
    assert ascii("hello") == "'hello'"
    assert ascii(42) == "42"
    assert ascii(3.14) == "3.14"
    assert ascii(True) == "True"
    assert ascii(None) == "None"
    assert ascii([1, 2, 3]) == "[1, 2, 3]"
    a_str = "world"
    assert f"{a_str!a}" == "'world'"
    a_int = 42
    assert f"{a_int!a}" == "42"
    assert "{!a}".format("z") == "'z'"
    # Dynamic (nested) f-string format specs reuse the same engine
    prec = 3
    assert f"{3.14159:.{prec}f}" == "3.142"
    wid = 6
    assert f"{42:{wid}d}" == "    42"
    assert f"{'hi':>{wid}}" == "    hi"
    # User-class __format__ via f-string, format() and bare {p}
    # (class hoisted to module scope: _FmtTemperature)
    t = _FmtTemperature(100.0)
    assert f"{t}" == "100.0C"
    assert f"{t:F}" == "212.0F"
    assert format(t, "F") == "212.0F"
    assert "{}".format(t) == "100.0C"
    assert "{0:F}".format(t) == "212.0F"

    # A class with __str__ but no __format__: empty spec routes to __str__.
    # (class hoisted to module scope: _FmtLabelled)
    lab = _FmtLabelled("hi")
    assert f"{lab}" == "label:hi"
    assert "{}".format(lab) == "label:hi"


_fold_p29_format()


print("All string tests passed!")
