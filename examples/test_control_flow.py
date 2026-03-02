# Consolidated test file for control flow

# ===== SECTION: While loops (basic, break, continue) =====

# Test while loop - sum from 1 to 10
sum1: int = 0
i: int = 1

while i <= 10:
    sum1 = sum1 + i
    i = i + 1

assert sum1 == 55, "sum of 1 to 10 should be 55"

# While loop with break and continue
sum_while: int = 0
count_while: int = 0
while count_while < 5:
    count_while = count_while + 1
    if count_while == 2:
        continue
    if count_while == 4:
        break
    sum_while = sum_while + count_while
# 1 + 3 = 4 (skips 2, breaks at 4)
assert sum_while == 4, "while with break/continue failed"

# ===== SECTION: For loops with range(n), range(start, stop), range(start, stop, step) =====

# Test for loop with range(n) - sum from 0 to 9
total: int = 0
j: int = 0
for j in range(10):
    total = total + j

assert total == 45, "sum of 0 to 9 should be 45"

# Test for loop with range(start, stop) - sum from 5 to 14
range_sum: int = 0
k: int = 0
for k in range(5, 15):
    range_sum = range_sum + k

assert range_sum == 95, "sum of 5 to 14 should be 95"

# Basic range with positive step
sum_step1: int = 0
for step_i in range(0, 10, 2):
    sum_step1 = sum_step1 + step_i
# 0 + 2 + 4 + 6 + 8 = 20
assert sum_step1 == 20, "range(0, 10, 2) should give sum 20"

# Range with step=3
sum_step2: int = 0
for step_j in range(1, 15, 3):
    sum_step2 = sum_step2 + step_j
# 1 + 4 + 7 + 10 + 13 = 35
assert sum_step2 == 35, "range(1, 15, 3) should give sum 35"

# Negative step - countdown from 10 to 1
sum_step3: int = 0
for step_k in range(10, 0, -1):
    sum_step3 = sum_step3 + step_k
# 10 + 9 + 8 + 7 + 6 + 5 + 4 + 3 + 2 + 1 = 55
assert sum_step3 == 55, "range(10, 0, -1) should give sum 55"

# Negative step with step=-2
sum_step4: int = 0
for step_m in range(10, 0, -2):
    sum_step4 = sum_step4 + step_m
# 10 + 8 + 6 + 4 + 2 = 30
assert sum_step4 == 30, "range(10, 0, -2) should give sum 30"

# Negative step with step=-3
sum_step5: int = 0
for step_n in range(15, 0, -3):
    sum_step5 = sum_step5 + step_n
# 15 + 12 + 9 + 6 + 3 = 45
assert sum_step5 == 45, "range(15, 0, -3) should give sum 45"

# Empty range tests
count_empty: int = 0
for ep in range(5, 5, -1):
    count_empty = count_empty + 1
assert count_empty == 0, "range(5, 5, -1) should be empty"

count_empty2: int = 0
for eq in range(1, 10, -1):
    count_empty2 = count_empty2 + 1
assert count_empty2 == 0, "range(1, 10, -1) should be empty"

count_empty3: int = 0
for er in range(10, 5, 1):
    count_empty3 = count_empty3 + 1
assert count_empty3 == 0, "range(10, 5, 1) should be empty"

# Single iteration
count_single: int = 0
for es in range(5, 6, 1):
    count_single = count_single + 1
assert count_single == 1, "range(5, 6, 1) should iterate once"

# Negative range bounds with negative step
sum_neg: int = 0
for et in range(-1, -6, -1):
    sum_neg = sum_neg + et
# -1 + -2 + -3 + -4 + -5 = -15
assert sum_neg == -15, "range(-1, -6, -1) should give sum -15"

# Positive to negative with negative step
sum_pos_neg: int = 0
for eu in range(3, -3, -1):
    sum_pos_neg = sum_pos_neg + eu
# 3 + 2 + 1 + 0 + -1 + -2 = 3
assert sum_pos_neg == 3, "range(3, -3, -1) should give sum 3"

# Large step that exceeds range
count_large: int = 0
for ev in range(0, 5, 10):
    count_large = count_large + 1
assert count_large == 1, "range(0, 5, 10) should iterate once (only 0)"

# Large negative step that exceeds range
count_large_neg: int = 0
for ew in range(5, 0, -10):
    count_large_neg = count_large_neg + 1
assert count_large_neg == 1, "range(5, 0, -10) should iterate once (only 5)"

# ===== SECTION: For loops with iterables (list, tuple, string, dict) =====

# Test basic list iteration
nums: list[int] = [10, 20, 30, 40, 50]
list_total: int = 0
for num in nums:
    list_total = list_total + num
assert list_total == 150, "sum of list elements should be 150"

# Test empty list iteration (should not execute body)
empty_list: list[int] = []
empty_count: int = 0
for x in empty_list:
    empty_count = empty_count + 1
assert empty_count == 0, "empty list should have 0 iterations"

# Test single element list
single: list[int] = [42]
single_total: int = 0
for s in single:
    single_total = single_total + s
assert single_total == 42, "single element list sum should be 42"

# Test basic tuple iteration
coords: tuple[int, int, int] = (100, 200, 300)
coord_sum: int = 0
for c in coords:
    coord_sum = coord_sum + c
assert coord_sum == 600, "sum of tuple elements should be 600"

# Test single element tuple
single_tuple: tuple[int] = (99,)
single_t: int = 0
for st in single_tuple:
    single_t = single_t + st
assert single_t == 99, "single tuple element sum should be 99"

# Test string iteration (characters)
text: str = "HELLO"
char_count: int = 0
for ch in text:
    char_count = char_count + 1
assert char_count == 5, "string 'HELLO' should have 5 characters"

# Test empty string iteration
empty_str: str = ""
empty_char_count: int = 0
for ec in empty_str:
    empty_char_count = empty_char_count + 1
assert empty_char_count == 0, "empty string should have 0 iterations"

# Test single character string
single_char: str = "X"
single_char_count: int = 0
for sc in single_char:
    single_char_count = single_char_count + 1
assert single_char_count == 1, "single char string should have 1 iteration"

# Test dict iteration (iterates over keys)
ages: dict[str, int] = {"alice": 30, "bob": 25, "carol": 35}
key_count: int = 0
for name in ages:
    key_count = key_count + 1
assert key_count == 3, "dict should have 3 keys"

# Test empty dict iteration
empty_dict: dict[str, int] = {}
empty_dict_count: int = 0
for ed in empty_dict:
    empty_dict_count = empty_dict_count + 1
assert empty_dict_count == 0, "empty dict should have 0 iterations"

# ===== SECTION: Nested loops =====

# Test nested range loops
inner_sum: int = 0
for a in range(3):
    for b in range(4):
        inner_sum = inner_sum + 1
assert inner_sum == 12, "3 * 4 iterations should be 12"

# Test nested list loops
outer_list: list[int] = [1, 2]
inner_list: list[int] = [10, 20, 30]
nested_sum: int = 0
for ol in outer_list:
    for il in inner_list:
        nested_sum = nested_sum + ol + il
# (1+10)+(1+20)+(1+30)+(2+10)+(2+20)+(2+30) = 11+21+31+12+22+32 = 129
assert nested_sum == 129, "nested list sum should be 129"

# Test mixed range and list
range_list_sum: int = 0
items: list[int] = [100, 200]
for ri in range(3):
    for item in items:
        range_list_sum = range_list_sum + ri + item
# i=0: 100+200 = 300; i=1: 101+201 = 302; i=2: 102+202 = 304; total = 906
assert range_list_sum == 906, "mixed range+list sum should be 906"

# Nested loops with different steps
nested_total: int = 0
for na in range(0, 6, 2):
    for nb in range(3, 0, -1):
        nested_total = nested_total + 1
# outer: 0, 2, 4 (3 iterations) * inner: 3, 2, 1 (3 iterations) = 9
assert nested_total == 9, "nested range loops should give 9 iterations"

# Triple nested loops
triple: int = 0
l1: list[int] = [1, 2]
l2: list[int] = [1, 2]
l3: list[int] = [1, 2]
for x1 in l1:
    for y1 in l2:
        for z1 in l3:
            triple = triple + 1
# 2 * 2 * 2 = 8
assert triple == 8, "triple nested should be 8 iterations"

# Nested string iteration
char_pairs: int = 0
str1: str = "AB"
str2: str = "12"
for c1 in str1:
    for c2 in str2:
        char_pairs = char_pairs + 1
# 2 * 2 = 4 pairs: A1, A2, B1, B2
assert char_pairs == 4, "string nested should be 4 pairs"

# Nested for over list and string
nested_count: int = 0
words: list[str] = ["hi", "bye"]
for word in words:
    for letter in word:
        nested_count = nested_count + 1
assert nested_count == 5, "hi(2) + bye(3) = 5 characters"

# Accumulating strings from iteration
result: str = ""
letters: list[str] = ["a", "b", "c"]
for letter in letters:
    result = result + letter
assert result == "abc", "concatenated letters should be 'abc'"

# ===== SECTION: Break/continue in loops =====

# Test for loop with break - find first number > 5
found: int = 0
m: int = 0
for m in range(20):
    if m > 5:
        found = m
        break
assert found == 6, "first number > 5 should be 6"

# Test for loop with continue - sum only even numbers from 0 to 9
even_sum: int = 0
n: int = 0
for n in range(10):
    if n % 2 == 1:
        continue
    even_sum = even_sum + n
assert even_sum == 20, "sum of even numbers 0,2,4,6,8 should be 20"

# Test list with break
first_big: int = 0
values: list[int] = [1, 2, 10, 20, 30]
for v in values:
    if v >= 10:
        first_big = v
        break
assert first_big == 10, "first element >= 10 should be 10"

# Test list with continue
even_only: int = 0
mixed: list[int] = [1, 2, 3, 4, 5, 6]
for mx in mixed:
    if mx % 2 == 1:
        continue
    even_only = even_only + mx
assert even_only == 12, "sum of even elements (2+4+6) should be 12"

# Test nested with break in inner loop
break_count: int = 0
outer: list[int] = [1, 2, 3]
inner: list[int] = [1, 2, 3, 4, 5]
for o in outer:
    for ii in inner:
        if ii > 2:
            break
        break_count = break_count + 1
# For each outer iteration, inner runs 2 times (1, 2) then breaks: 3 * 2 = 6
assert break_count == 6, "break in inner should give 6 iterations"

# ===== SECTION: Ternary operator =====

num: int = 10
ternary_result: str = "between" if num > 5 and num < 15 else "not between"
assert ternary_result == "between", "ternary operator failed"

ternary_result2: str = "big" if num > 20 else "small"
assert ternary_result2 == "small", "ternary operator 2 failed"

# Nested if-else test
nested_result: str = ""
if num > 5:
    if num < 15:
        nested_result = "between 5 and 15"
    else:
        nested_result = "greater than 15"
else:
    nested_result = "less than or equal to 5"
assert nested_result == "between 5 and 15", "nested if-else failed"

# ===== SECTION: Augmented assignments (+=, -=, *=, /=, //=, %=, **=, &=, |=, ^=, <<=, >>=) =====

# Test basic arithmetic augmented assignments with integers
x_aug: int = 10
x_aug += 5
assert x_aug == 15, "x += 5 failed"

x_aug -= 3
assert x_aug == 12, "x -= 3 failed"

x_aug *= 2
assert x_aug == 24, "x *= 2 failed"

# Test floor division
y_aug: int = 25
y_aug //= 3
assert y_aug == 8, "y //= 3 failed"

# Test modulo
z_aug: int = 17
z_aug %= 5
assert z_aug == 2, "z %= 5 failed"

# Test power
p_aug: int = 2
p_aug **= 3
assert p_aug == 8, "p **= 3 failed"

# Test with floats
f_aug: float = 10.0
f_aug += 2.5
assert f_aug == 12.5, "f += 2.5 failed"

f_aug -= 3.5
assert f_aug == 9.0, "f -= 3.5 failed"

f_aug *= 2.0
assert f_aug == 18.0, "f *= 2.0 failed"

f_aug /= 3.0
assert f_aug == 6.0, "f /= 3.0 failed"

# Test floor division with floats
g_aug: float = 17.0
g_aug //= 3.0
assert g_aug == 5.0, "g //= 3.0 failed"

# Test modulo with floats
h_aug: float = 17.5
h_aug %= 5.0
assert h_aug == 2.5, "h %= 5.0 failed"

# Test bitwise augmented assignments (integers only)
a_aug: int = 12  # 0b1100
a_aug &= 10      # 12 & 10 = 8
assert a_aug == 8, "a &= failed"

b_aug: int = 12  # 0b1100
b_aug |= 3       # 12 | 3 = 15
assert b_aug == 15, "b |= failed"

c_aug: int = 12  # 0b1100
c_aug ^= 10      # 12 ^ 10 = 6
assert c_aug == 6, "c ^= failed"

# Test shift operators
d_aug: int = 1
d_aug <<= 4          # 1 << 4 = 16
assert d_aug == 16, "d <<= 4 failed"

e_aug: int = 64
e_aug >>= 2          # 64 >> 2 = 16
assert e_aug == 16, "e >>= 2 failed"

# Test augmented assignment in loops
total_aug: int = 0
for i_aug in range(1, 6):
    total_aug += i_aug
assert total_aug == 15, "loop += failed (1+2+3+4+5=15)"

product_aug: int = 1
for i_aug in range(1, 5):
    product_aug *= i_aug
assert product_aug == 24, "loop *= failed (1*2*3*4=24)"

# ===== SECTION: Augmented assignments with lists/fields =====

# Test augmented assignment with list indexing
nums_aug: list[int] = [10, 20, 30]
nums_aug[0] += 5
assert nums_aug[0] == 15, "nums[0] += 5 failed"

nums_aug[1] -= 5
assert nums_aug[1] == 15, "nums[1] -= 5 failed"

nums_aug[2] *= 2
assert nums_aug[2] == 60, "nums[2] *= 2 failed"

# Test augmented assignment with variable index
idx: int = 1
nums_aug[idx] += 10
assert nums_aug[1] == 25, "nums[idx] += 10 failed"

# Test augmented assignment with negative values
neg_aug: int = 5
neg_aug += -3
assert neg_aug == 2, "neg += -3 failed"

neg_aug *= -1
assert neg_aug == -2, "neg *= -1 failed"

# Test chained augmented operations (each on separate lines)
chain: int = 100
chain -= 10
chain //= 3
chain *= 2
assert chain == 60, "chained ops failed (100-10=90, 90//3=30, 30*2=60)"

# Test with expressions on RHS
base: int = 10
base += 2 * 3
assert base == 16, "base += 2 * 3 failed"

base -= 4 + 2
assert base == 10, "base -= 4 + 2 failed"

# Test string concatenation with +=
s_aug: str = "Hello"
s_aug += " "
s_aug += "World"
assert s_aug == "Hello World", "string += failed"

# Test string multiplication with *=
repeat: str = "ab"
repeat *= 3
assert repeat == "ababab", "string *= failed"

# Test augmented assignment with class fields
class Counter:
    count: int

    def __init__(self, start: int) -> None:
        self.count = start

    def increment(self) -> None:
        self.count += 1

    def add(self, n_add: int) -> None:
        self.count += n_add

counter = Counter(0)
counter.count += 10
assert counter.count == 10, "counter.count += 10 failed"

counter.increment()
assert counter.count == 11, "counter.increment() failed"

counter.add(5)
assert counter.count == 16, "counter.add(5) failed"

# Test augmented assignment on field with other operators
class Stats:
    value: int

    def __init__(self, v: int) -> None:
        self.value = v

stats = Stats(100)
stats.value -= 20
assert stats.value == 80, "stats.value -= 20 failed"

stats.value *= 2
assert stats.value == 160, "stats.value *= 2 failed"

stats.value //= 5
assert stats.value == 32, "stats.value //= 5 failed"

# ===== SECTION: for...else =====

# Basic for...else - else executes when loop completes normally
for_else_result: str = ""
for fe_i in range(3):
    pass
else:
    for_else_result = "completed"
assert for_else_result == "completed", "for...else basic failed"

# for...else with break - else should NOT execute
for_else_break: str = "no_break"
for fe_j in range(10):
    if fe_j == 3:
        for_else_break = "broke"
        break
else:
    for_else_break = "completed"
assert for_else_break == "broke", "for...else with break failed"

# for...else search pattern
def find_value(items: list[int], target: int) -> str:
    for item in items:
        if item == target:
            return "found"
    else:
        return "not found"
    return "unreachable"

assert find_value([1, 2, 3, 4, 5], 3) == "found", "find_value found failed"
assert find_value([1, 2, 3, 4, 5], 99) == "not found", "find_value not found failed"

# for...else with range
fe_sum: int = 0
for fe_k in range(5):
    fe_sum += fe_k
else:
    fe_sum += 100
assert fe_sum == 110, "for...else sum should be 0+1+2+3+4+100=110"

print("for...else tests passed!")

# ===== SECTION: while...else =====

# Basic while...else
we_counter: int = 0
we_result: str = ""
while we_counter < 3:
    we_counter += 1
else:
    we_result = "finished"
assert we_result == "finished", "while...else basic failed"
assert we_counter == 3, "while...else counter failed"

# while...else with break
we_break_result: str = "no_break"
we_n: int = 0
while we_n < 10:
    if we_n == 5:
        we_break_result = "broke_at_5"
        break
    we_n += 1
else:
    we_break_result = "completed"
assert we_break_result == "broke_at_5", "while...else with break failed"

# while...else that never enters body (condition false from start)
we_never: str = ""
while False:
    we_never = "entered"
else:
    we_never = "else_ran"
assert we_never == "else_ran", "while...else with false condition should run else"

print("while...else tests passed!")

# ===== SECTION: Walrus operator :=  =====

# Basic walrus operator in if
walrus_list: list[int] = [1, 2, 3, 4, 5]
if (walrus_n := len(walrus_list)) > 3:
    assert walrus_n == 5, "walrus in if failed"

# Walrus with nested expression
walrus_x: int = 10
if (walrus_double := walrus_x * 2) > 15:
    assert walrus_double == 20, "walrus nested expr failed"

# TODO: Walrus in while condition needs re-evaluation on each iteration

print("Walrus operator tests passed!")

# ===== SECTION: Multiple assignment (a = b = value) =====

multi_a: int = 0
multi_b: int = 0
multi_a = multi_b = 42
assert multi_a == 42, "multiple assignment a failed"
assert multi_b == 42, "multiple assignment b failed"

# Multiple assignment with expression
multi_c: int = 0
multi_d: int = 0
multi_c = multi_d = 10 + 20
assert multi_c == 30, "multiple assignment with expr c failed"
assert multi_d == 30, "multiple assignment with expr d failed"

# Multiple string assignment
multi_s1: str = ""
multi_s2: str = ""
multi_s1 = multi_s2 = "hello"
assert multi_s1 == "hello", "multiple assignment string s1 failed"
assert multi_s2 == "hello", "multiple assignment string s2 failed"

print("Multiple assignment tests passed!")

# === Implicit truthiness in if/while ===

# String truthiness
truthiness_str_full: str = "hello"
truthiness_str_empty: str = ""
assert ("yes" if truthiness_str_full else "no") == "yes", "non-empty string should be truthy"
assert ("yes" if truthiness_str_empty else "no") == "no", "empty string should be falsy"

# List truthiness
truthiness_list_full: list[int] = [1, 2, 3]
truthiness_list_empty: list[int] = []
assert ("yes" if truthiness_list_full else "no") == "yes", "non-empty list should be truthy"
assert ("yes" if truthiness_list_empty else "no") == "no", "empty list should be falsy"

# Dict truthiness
truthiness_dict_full: dict[str, int] = {"a": 1}
truthiness_dict_empty: dict[str, int] = {}
assert ("yes" if truthiness_dict_full else "no") == "yes", "non-empty dict should be truthy"
assert ("yes" if truthiness_dict_empty else "no") == "no", "empty dict should be falsy"

# Int truthiness
assert ("yes" if 42 else "no") == "yes", "non-zero int should be truthy"
assert ("yes" if 0 else "no") == "no", "zero int should be falsy"

# While with list truthiness
truthiness_while_items: list[int] = [1, 2, 3]
truthiness_while_count: int = 0
while truthiness_while_items:
    truthiness_while_items.pop()
    truthiness_while_count = truthiness_while_count + 1
assert truthiness_while_count == 3, f"while truthiness: expected 3, got {truthiness_while_count}"

print("Implicit truthiness tests passed!")

print("All control flow tests passed!")
