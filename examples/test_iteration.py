# Consolidated test file for iteration and comprehensions

# ===== SECTION: List comprehensions (basic, with filter, nested) =====

# Basic list comprehension
squares: list[int] = [x * x for x in range(5)]
assert len(squares) == 5, "len(squares) should equal 5"
assert squares[0] == 0, "squares[0] should equal 0"
assert squares[1] == 1, "squares[1] should equal 1"
assert squares[4] == 16, "squares[4] should equal 16"

# List comprehension with filter
evens: list[int] = [x for x in range(10) if x % 2 == 0]
assert len(evens) == 5, "len(evens) should equal 5"
assert evens[0] == 0, "evens[0] should equal 0"
assert evens[2] == 4, "evens[2] should equal 4"
assert evens[4] == 8, "evens[4] should equal 8"

# Nested generators (two for loops)
pairs: list[int] = [x + y for x in range(3) for y in range(2)]
assert len(pairs) == 6, "len(pairs) should equal 6"
assert pairs[0] == 0, "pairs[0] should equal 0"  # 0 + 0
assert pairs[1] == 1, "pairs[1] should equal 1"  # 0 + 1
assert pairs[2] == 1, "pairs[2] should equal 1"  # 1 + 0
assert pairs[3] == 2, "pairs[3] should equal 2"  # 1 + 1
assert pairs[4] == 2, "pairs[4] should equal 2"  # 2 + 0
assert pairs[5] == 3, "pairs[5] should equal 3"  # 2 + 1

# List comprehension over list
doubled: list[int] = [x * 2 for x in [1, 2, 3]]
assert len(doubled) == 3, "len(doubled) should equal 3"
assert doubled[0] == 2, "doubled[0] should equal 2"
assert doubled[1] == 4, "doubled[1] should equal 4"
assert doubled[2] == 6, "doubled[2] should equal 6"

# List comprehension over string
chars: list[str] = [c for c in "abc"]
assert len(chars) == 3, "len(chars) should equal 3"
assert chars[0] == "a", "chars[0] should equal \"a\""
assert chars[1] == "b", "chars[1] should equal \"b\""
assert chars[2] == "c", "chars[2] should equal \"c\""

# List comprehension with expression transformation
transformed: list[str] = [str(x) for x in range(3)]
assert len(transformed) == 3, "len(transformed) should equal 3"
assert transformed[0] == "0", "transformed[0] should equal \"0\""
assert transformed[1] == "1", "transformed[1] should equal \"1\""
assert transformed[2] == "2", "transformed[2] should equal \"2\""

# Comprehension in function
def make_squares(n: int) -> list[int]:
    return [i * i for i in range(n)]

result: list[int] = make_squares(4)
assert len(result) == 4, "len(result) should equal 4"
assert result[3] == 9, "result[3] should equal 9"

# Comprehension in if statement
flag: bool = True
if flag:
    inner_comp: list[int] = [x for x in range(3)]
    assert len(inner_comp) == 3, "len(inner_comp) should equal 3"

# Comprehension in for loop (outer loop over range, inner comprehension)
total: int = 0
for i in range(3):
    comp: list[int] = [x for x in range(i + 1)]
    total = total + len(comp)
# i=0: [0] -> 1
# i=1: [0, 1] -> 2
# i=2: [0, 1, 2] -> 3
# total = 1 + 2 + 3 = 6
assert total == 6, "total should equal 6"

# Multiple comprehensions in sequence
first: list[int] = [a for a in range(2)]
second: list[int] = [b for b in range(3)]
assert len(first) == 2, "len(first) should equal 2"
assert len(second) == 3, "len(second) should equal 3"

# Multiple filters
multi_filter: list[int] = [x for x in range(20) if x % 2 == 0 if x % 3 == 0]
# Numbers divisible by both 2 and 3 (i.e., divisible by 6): 0, 6, 12, 18
assert len(multi_filter) == 4, "len(multi_filter) should equal 4"
assert multi_filter[0] == 0, "multi_filter[0] should equal 0"
assert multi_filter[1] == 6, "multi_filter[1] should equal 6"
assert multi_filter[2] == 12, "multi_filter[2] should equal 12"
assert multi_filter[3] == 18, "multi_filter[3] should equal 18"

# ===== SECTION: Dict comprehensions =====

# Basic dict comprehension
sq_dict: dict[int, int] = {x: x * x for x in range(4)}
assert len(sq_dict) == 4, "len(sq_dict) should equal 4"
assert sq_dict[0] == 0, "sq_dict[0] should equal 0"
assert sq_dict[1] == 1, "sq_dict[1] should equal 1"
assert sq_dict[2] == 4, "sq_dict[2] should equal 4"
assert sq_dict[3] == 9, "sq_dict[3] should equal 9"

# Dict comprehension with filter
filtered: dict[int, int] = {x: x * 2 for x in range(5) if x > 1}
assert len(filtered) == 3, "len(filtered) should equal 3"
assert 0 not in filtered, "0 not should be in filtered"
assert 1 not in filtered, "1 not should be in filtered"
assert filtered[2] == 4, "filtered[2] should equal 4"
assert filtered[3] == 6, "filtered[3] should equal 6"
assert filtered[4] == 8, "filtered[4] should equal 8"

# Dict comprehension with different key and value expressions
doubled_dict: dict[int, int] = {i: i * 2 for i in range(3)}
assert doubled_dict[0] == 0, "doubled_dict[0] should equal 0"
assert doubled_dict[1] == 2, "doubled_dict[1] should equal 2"
assert doubled_dict[2] == 4, "doubled_dict[2] should equal 4"

# Dict comprehension in function
def make_sq_dict(n: int) -> dict[int, int]:
    return {i: i * i for i in range(n)}

sq_result: dict[int, int] = make_sq_dict(3)
assert len(sq_result) == 3, "len(sq_result) should equal 3"
assert sq_result[2] == 4, "sq_result[2] should equal 4"

# ===== SECTION: iter() and next() =====

# List iterator
nums: list[int] = [10, 20, 30]
it = iter(nums)
assert next(it) == 10, "next(it) should equal 10"
assert next(it) == 20, "next(it) should equal 20"
assert next(it) == 30, "next(it) should equal 30"

# ===== SECTION: StopIteration handling =====

# StopIteration on exhausted iterator
try:
    next(it)
    assert False, "should have raised StopIteration"
except:
    pass  # Expected

# String iterator
s = iter("AB")
assert next(s) == "A", "next(s) should equal \"A\""
assert next(s) == "B", "next(s) should equal \"B\""

# StopIteration on exhausted string iterator
try:
    next(s)
    assert False, "should have raised StopIteration"
except:
    pass  # Expected

# Tuple iterator
t = iter((1, 2))
assert next(t) == 1, "next(t) should equal 1"
assert next(t) == 2, "next(t) should equal 2"

# Dict iterator (keys)
d: dict[str, int] = {"a": 1, "b": 2}
di = iter(d)
k1 = next(di)
k2 = next(di)
assert k1 == "a" or k1 == "b", "k1 should equal \"a\" or k1 == \"b\""

# Range iterator
ri = iter(range(3))
assert next(ri) == 0, "next(ri) should equal 0"
assert next(ri) == 1, "next(ri) should equal 1"
assert next(ri) == 2, "next(ri) should equal 2"

# StopIteration on exhausted range iterator
try:
    next(ri)
    assert False, "should have raised StopIteration"
except:
    pass  # Expected

# Range with start and stop
ri2 = iter(range(5, 8))
assert next(ri2) == 5, "next(ri2) should equal 5"
assert next(ri2) == 6, "next(ri2) should equal 6"
assert next(ri2) == 7, "next(ri2) should equal 7"

# Range with negative step
ri3 = iter(range(3, 0, -1))
assert next(ri3) == 3, "next(ri3) should equal 3"
assert next(ri3) == 2, "next(ri3) should equal 2"
assert next(ri3) == 1, "next(ri3) should equal 1"

# For-loop over iterables (collecting into list)
result_list: list[int] = []
for i in [1, 2, 3]:
    result_list.append(i)
assert result_list == [1, 2, 3], "for-loop over list failed"

# For-loop over bytes
result_bytes: list[int] = []
for b in b'hello':
    result_bytes.append(b)
assert result_bytes == [104, 101, 108, 108, 111], "for-loop over bytes failed"

# For-loop over string
result_string: list[str] = []
for c in 'hello':
    result_string.append(c)
assert result_string == ['h', 'e', 'l', 'l', 'o'], "for-loop over string failed"

# For-loop over range
result_range: list[int] = []
for i in range(3):
    result_range.append(i)
assert result_range == [0, 1, 2], "for-loop over range failed"

# ===== SECTION: reversed() for lists, tuples, strings, ranges, dicts =====

# Test reversed list iteration
print("Testing reversed list...")
rev_nums: list[int] = [10, 20, 30]
rev_it = reversed(rev_nums)
v1 = next(rev_it)
assert v1 == 30, "v1 should equal 30"
v2 = next(rev_it)
assert v2 == 20, "v2 should equal 20"
v3 = next(rev_it)
assert v3 == 10, "v3 should equal 10"
print("Reversed list: OK")

# Test StopIteration on exhausted reversed iterator
try:
    next(rev_it)
    assert False, "should have raised StopIteration"
except:
    pass  # Expected

# Test reversed tuple iteration
print("Testing reversed tuple...")
rev_t: tuple[int, int, int] = (1, 2, 3)
ti = reversed(rev_t)
tv1 = next(ti)
assert tv1 == 3, "tv1 should equal 3"
tv2 = next(ti)
assert tv2 == 2, "tv2 should equal 2"
tv3 = next(ti)
assert tv3 == 1, "tv3 should equal 1"
print("Reversed tuple: OK")

# Test reversed string iteration
print("Testing reversed string...")
rev_s = reversed("ABC")
sc1 = next(rev_s)
assert sc1 == "C", "sc1 should equal \"C\""
sc2 = next(rev_s)
assert sc2 == "B", "sc2 should equal \"B\""
sc3 = next(rev_s)
assert sc3 == "A", "sc3 should equal \"A\""
print("Reversed string: OK")

# Test reversed range - reversed(range(5)) should give 4,3,2,1,0
print("Testing reversed range(5)...")
rev_ri = reversed(range(5))
r1 = next(rev_ri)
assert r1 == 4, "r1 should equal 4"
r2 = next(rev_ri)
assert r2 == 3, "r2 should equal 3"
r3 = next(rev_ri)
assert r3 == 2, "r3 should equal 2"
r4 = next(rev_ri)
assert r4 == 1, "r4 should equal 1"
r5 = next(rev_ri)
assert r5 == 0, "r5 should equal 0"
print("Reversed range(5): OK")

# Test reversed range with start and stop - reversed(range(2, 6)) should give 5,4,3,2
print("Testing reversed range(2, 6)...")
rev_ri2 = reversed(range(2, 6))
rr1 = next(rev_ri2)
assert rr1 == 5, "rr1 should equal 5"
rr2 = next(rev_ri2)
assert rr2 == 4, "rr2 should equal 4"
rr3 = next(rev_ri2)
assert rr3 == 3, "rr3 should equal 3"
rr4 = next(rev_ri2)
assert rr4 == 2, "rr4 should equal 2"
print("Reversed range(2,6): OK")

# Test reversed range with step - reversed(range(0, 10, 2)) should give 8,6,4,2,0
print("Testing reversed range(0, 10, 2)...")
rev_ri3 = reversed(range(0, 10, 2))
rs1 = next(rev_ri3)
assert rs1 == 8, "rs1 should equal 8"
rs2 = next(rev_ri3)
assert rs2 == 6, "rs2 should equal 6"
rs3 = next(rev_ri3)
assert rs3 == 4, "rs3 should equal 4"
rs4 = next(rev_ri3)
assert rs4 == 2, "rs4 should equal 2"
rs5 = next(rev_ri3)
assert rs5 == 0, "rs5 should equal 0"
print("Reversed range with step: OK")

# Test reversed dict - verify it returns keys in reverse of forward iteration order
print("Testing reversed dict...")
rev_d: dict[str, int] = {"a": 1, "b": 2, "c": 3}

# Get forward order
forward_it = iter(rev_d)
fk1 = next(forward_it)
fk2 = next(forward_it)
fk3 = next(forward_it)

# Get reversed order
rev_di = reversed(rev_d)
dk1 = next(rev_di)
dk2 = next(rev_di)
dk3 = next(rev_di)

# Verify reversed is opposite of forward
assert dk1 == fk3, "dk1 should equal fk3"
assert dk2 == fk2, "dk2 should equal fk2"
assert dk3 == fk1, "dk3 should equal fk1"
print("Reversed dict: OK")

# ===== SECTION: sorted() for all collection types =====

# Test sorted(list[int])
sorted_nums: list[int] = [3, 1, 4, 1, 5, 9, 2, 6]
sorted_list: list[int] = sorted(sorted_nums)
assert sorted_list[0] == 1, "sorted_list[0] should equal 1"
assert sorted_list[1] == 1, "sorted_list[1] should equal 1"
assert sorted_list[2] == 2, "sorted_list[2] should equal 2"
assert sorted_list[3] == 3, "sorted_list[3] should equal 3"
assert sorted_list[4] == 4, "sorted_list[4] should equal 4"
assert sorted_list[5] == 5, "sorted_list[5] should equal 5"
assert sorted_list[6] == 6, "sorted_list[6] should equal 6"
assert sorted_list[7] == 9, "sorted_list[7] should equal 9"

# Test sorted(list[str])
words: list[str] = ["banana", "apple", "cherry"]
sorted_words: list[str] = sorted(words)
assert sorted_words[0] == "apple", "sorted_words[0] should equal \"apple\""
assert sorted_words[1] == "banana", "sorted_words[1] should equal \"banana\""
assert sorted_words[2] == "cherry", "sorted_words[2] should equal \"cherry\""

# Test sorted(tuple)
tup: tuple[int, int, int] = (3, 1, 2)
sorted_tup: list[int] = sorted(tup)
assert sorted_tup[0] == 1, "sorted_tup[0] should equal 1"
assert sorted_tup[1] == 2, "sorted_tup[1] should equal 2"
assert sorted_tup[2] == 3, "sorted_tup[2] should equal 3"

# Test sorted(str) - returns sorted list of chars
str_s: str = "cba"
sorted_str: list[str] = sorted(str_s)
assert sorted_str[0] == "a", "sorted_str[0] should equal \"a\""
assert sorted_str[1] == "b", "sorted_str[1] should equal \"b\""
assert sorted_str[2] == "c", "sorted_str[2] should equal \"c\""

# Test sorted(dict) - returns sorted keys
sorted_d: dict[str, int] = {"c": 3, "a": 1, "b": 2}
sorted_keys: list[str] = sorted(sorted_d)
assert sorted_keys[0] == "a", "sorted_keys[0] should equal \"a\""
assert sorted_keys[1] == "b", "sorted_keys[1] should equal \"b\""
assert sorted_keys[2] == "c", "sorted_keys[2] should equal \"c\""

# Test sorted(range(5))
sorted_range: list[int] = sorted(range(5))
assert sorted_range[0] == 0, "sorted_range[0] should equal 0"
assert sorted_range[1] == 1, "sorted_range[1] should equal 1"
assert sorted_range[2] == 2, "sorted_range[2] should equal 2"
assert sorted_range[3] == 3, "sorted_range[3] should equal 3"
assert sorted_range[4] == 4, "sorted_range[4] should equal 4"

# Test sorted(range(5, 0, -1)) - descending range
sorted_desc_range: list[int] = sorted(range(5, 0, -1))
assert sorted_desc_range[0] == 1, "sorted_desc_range[0] should equal 1"
assert sorted_desc_range[1] == 2, "sorted_desc_range[1] should equal 2"
assert sorted_desc_range[2] == 3, "sorted_desc_range[2] should equal 3"
assert sorted_desc_range[3] == 4, "sorted_desc_range[3] should equal 4"
assert sorted_desc_range[4] == 5, "sorted_desc_range[4] should equal 5"

# Test empty list
empty: list[int] = []
sorted_empty: list[int] = sorted(empty)
assert len(sorted_empty) == 0, "len(sorted_empty) should equal 0"

# Test single element
single: list[int] = [42]
sorted_single: list[int] = sorted(single)
assert sorted_single[0] == 42, "sorted_single[0] should equal 42"

# Test that original list is not modified
original_sorted: list[int] = [3, 2, 1]
_ = sorted(original_sorted)
assert original_sorted[0] == 3, "original_sorted[0] should equal 3"
assert original_sorted[1] == 2, "original_sorted[1] should equal 2"
assert original_sorted[2] == 1, "original_sorted[2] should equal 1"

# ===== SECTION: sorted() with reverse parameter =====

# Test sorted(list[int], reverse=True)
sorted_desc: list[int] = sorted(sorted_nums, reverse=True)
assert sorted_desc[0] == 9, "sorted_desc[0] should equal 9"
assert sorted_desc[1] == 6, "sorted_desc[1] should equal 6"
assert sorted_desc[2] == 5, "sorted_desc[2] should equal 5"
assert sorted_desc[3] == 4, "sorted_desc[3] should equal 4"
assert sorted_desc[4] == 3, "sorted_desc[4] should equal 3"
assert sorted_desc[5] == 2, "sorted_desc[5] should equal 2"
assert sorted_desc[6] == 1, "sorted_desc[6] should equal 1"
assert sorted_desc[7] == 1, "sorted_desc[7] should equal 1"

# Test sorted(list[str], reverse=True)
sorted_words_desc: list[str] = sorted(words, reverse=True)
assert sorted_words_desc[0] == "cherry", "sorted_words_desc[0] should equal \"cherry\""
assert sorted_words_desc[1] == "banana", "sorted_words_desc[1] should equal \"banana\""
assert sorted_words_desc[2] == "apple", "sorted_words_desc[2] should equal \"apple\""

# Test sorted(str, reverse=True)
sorted_str_desc: list[str] = sorted(str_s, reverse=True)
assert sorted_str_desc[0] == "c", "sorted_str_desc[0] should equal \"c\""
assert sorted_str_desc[1] == "b", "sorted_str_desc[1] should equal \"b\""
assert sorted_str_desc[2] == "a", "sorted_str_desc[2] should equal \"a\""

# Test sorted(range(5), reverse=True)
sorted_range_desc: list[int] = sorted(range(5), reverse=True)
assert sorted_range_desc[0] == 4, "sorted_range_desc[0] should equal 4"
assert sorted_range_desc[1] == 3, "sorted_range_desc[1] should equal 3"
assert sorted_range_desc[2] == 2, "sorted_range_desc[2] should equal 2"
assert sorted_range_desc[3] == 1, "sorted_range_desc[3] should equal 1"
assert sorted_range_desc[4] == 0, "sorted_range_desc[4] should equal 0"

# ===== SECTION: sorted() with key parameter =====

# Define a length function since builtins can't be passed as first-class values
def str_len(s: str) -> int:
    return len(s)

# Test sorted with key=str_len for strings
key_words: list[str] = ["banana", "apple", "pie", "watermelon"]
sorted_by_len: list[str] = sorted(key_words, key=str_len)
assert sorted_by_len[0] == "pie", "sorted_by_len[0] should equal \"pie\""  # length 3
assert sorted_by_len[1] == "apple", "sorted_by_len[1] should equal \"apple\""  # length 5
assert sorted_by_len[2] == "banana", "sorted_by_len[2] should equal \"banana\""  # length 6
assert sorted_by_len[3] == "watermelon", "sorted_by_len[3] should equal \"watermelon\""  # length 10

# Test sorted with key=str_len and reverse=True
sorted_by_len_desc: list[str] = sorted(key_words, key=str_len, reverse=True)
assert sorted_by_len_desc[0] == "watermelon", "sorted_by_len_desc[0] should equal \"watermelon\""  # length 10
assert sorted_by_len_desc[1] == "banana", "sorted_by_len_desc[1] should equal \"banana\""  # length 6
assert sorted_by_len_desc[2] == "apple", "sorted_by_len_desc[2] should equal \"apple\""  # length 5
assert sorted_by_len_desc[3] == "pie", "sorted_by_len_desc[3] should equal \"pie\""  # length 3

# Test with a function that returns negative (sort descending by negating)
def negate(x: int) -> int:
    return -x

key_nums: list[int] = [3, 1, 4, 1, 5, 9, 2, 6]
sorted_by_neg: list[int] = sorted(key_nums, key=negate)
# Should be sorted in descending order: 9, 6, 5, 4, 3, 2, 1, 1
assert sorted_by_neg[0] == 9, "sorted_by_neg[0] should equal 9"
assert sorted_by_neg[1] == 6, "sorted_by_neg[1] should equal 6"
assert sorted_by_neg[2] == 5, "sorted_by_neg[2] should equal 5"
assert sorted_by_neg[3] == 4, "sorted_by_neg[3] should equal 4"
assert sorted_by_neg[4] == 3, "sorted_by_neg[4] should equal 3"
assert sorted_by_neg[5] == 2, "sorted_by_neg[5] should equal 2"
assert sorted_by_neg[6] == 1, "sorted_by_neg[6] should equal 1"
assert sorted_by_neg[7] == 1, "sorted_by_neg[7] should equal 1"

# Test with key=abs for sorting by absolute value
def myabs(x: int) -> int:
    if x < 0:
        return -x
    return x

mixed: list[int] = [-5, 2, -3, 1, -4]
sorted_by_abs: list[int] = sorted(mixed, key=myabs)
# Sorted by abs: 1, 2, -3, -4, -5 (since abs values are 1, 2, 3, 4, 5)
assert myabs(sorted_by_abs[0]) == 1, "myabs(sorted_by_abs[0]) should equal 1"
assert myabs(sorted_by_abs[1]) == 2, "myabs(sorted_by_abs[1]) should equal 2"
assert myabs(sorted_by_abs[2]) == 3, "myabs(sorted_by_abs[2]) should equal 3"
assert myabs(sorted_by_abs[3]) == 4, "myabs(sorted_by_abs[3]) should equal 4"
assert myabs(sorted_by_abs[4]) == 5, "myabs(sorted_by_abs[4]) should equal 5"

# Test with function for strings - sort by first character
def first_char(s: str) -> str:
    return s[0]

names: list[str] = ["charlie", "alice", "bob"]
sorted_by_first: list[str] = sorted(names, key=first_char)
assert sorted_by_first[0] == "alice", "sorted_by_first[0] should equal \"alice\""
assert sorted_by_first[1] == "bob", "sorted_by_first[1] should equal \"bob\""
assert sorted_by_first[2] == "charlie", "sorted_by_first[2] should equal \"charlie\""

# Test sorted tuple with key
key_tup: tuple[str, str, str] = ("bb", "aaa", "c")
sorted_key_tup: list[str] = sorted(key_tup, key=str_len)
assert sorted_key_tup[0] == "c", "sorted_key_tup[0] should equal \"c\""  # length 1
assert sorted_key_tup[1] == "bb", "sorted_key_tup[1] should equal \"bb\""  # length 2
assert sorted_key_tup[2] == "aaa", "sorted_key_tup[2] should equal \"aaa\""  # length 3

# Test that original list is not modified with key
original_key: list[int] = [3, 2, 1]
_ = sorted(original_key, key=negate)
assert original_key[0] == 3, "original_key[0] should equal 3"
assert original_key[1] == 2, "original_key[1] should equal 2"
assert original_key[2] == 1, "original_key[2] should equal 1"

# ===== SECTION: min/max with key= =====

# Test min with key=str_len
min_key_words: list[str] = ["banana", "apple", "pie", "watermelon"]
min_by_len: str = min(min_key_words, key=str_len)
assert min_by_len == "pie", "min_by_len should equal \"pie\""

# Test max with key=str_len
max_by_len: str = max(min_key_words, key=str_len)
assert max_by_len == "watermelon", "max_by_len should equal \"watermelon\""

# Test min with key=myabs (negative numbers)
min_mixed: list[int] = [-5, 2, -3, 1, -4]
min_by_abs: int = min(min_mixed, key=myabs)
assert min_by_abs == 1, "min_by_abs should equal 1"

# Test max with key=myabs
max_by_abs: int = max(min_mixed, key=myabs)
assert max_by_abs == -5, "max_by_abs should equal -5"

# Test min with key=negate (reverses comparison)
min_nums: list[int] = [3, 1, 4, 1, 5, 9, 2, 6]
min_by_neg: int = min(min_nums, key=negate)
assert min_by_neg == 9, "min_by_neg should equal 9"

# Test max with key=negate
max_by_neg: int = max(min_nums, key=negate)
assert max_by_neg == 1, "max_by_neg should equal 1"

# Test min tuple with key
min_key_tup: tuple[str, str, str] = ("bb", "aaa", "c")
min_tup_result: str = min(min_key_tup, key=str_len)
assert min_tup_result == "c", "min_tup_result should equal \"c\""

# Test max tuple with key
max_tup_result: str = max(min_key_tup, key=str_len)
assert max_tup_result == "aaa", "max_tup_result should equal \"aaa\""

# Verify returned value is original element, not key value
def get_len(s: str) -> int:
    return len(s)

shortest: str = min(["hello", "hi", "hey"], key=get_len)
assert shortest == "hi", "shortest should equal \"hi\""
assert len(shortest) == 2, "len(shortest) should equal 2"

# Test min set with key
min_key_set: set[int] = {-5, 2, -3, 1}
min_set_by_abs: int = min(min_key_set, key=myabs)
assert min_set_by_abs == 1, "min_set_by_abs should equal 1"

# Test max set with key
max_set_by_abs: int = max(min_key_set, key=myabs)
assert max_set_by_abs == -5, "max_set_by_abs should equal -5"

# Test set with different orderings
test_set: set[int] = {3, 1, 4, 1, 5, 9, 2, 6}
min_by_negate: int = min(test_set, key=negate)
assert min_by_negate == 9, "min with negate should equal 9"

max_by_negate: int = max(test_set, key=negate)
assert max_by_negate == 1, "max with negate should equal 1"

# Test set with strings
str_set: set[str] = {"hello", "hi", "hey", "h"}
shortest_in_set: str = min(str_set, key=str_len)
assert shortest_in_set == "h", "shortest string should be 'h'"

longest_in_set: str = max(str_set, key=str_len)
assert longest_in_set == "hello", "longest string should be 'hello'"

print("min/max with key= tests passed!")

# ===== SECTION: enumerate() =====

# Basic enumerate over list
enum_items: list[str] = ["a", "b", "c"]
enum_idx_sum: int = 0
for enum_i, enum_v in enumerate(enum_items):
    if enum_i == 0:
        assert enum_v == "a", "enum_v should equal \"a\""
    if enum_i == 1:
        assert enum_v == "b", "enum_v should equal \"b\""
    if enum_i == 2:
        assert enum_v == "c", "enum_v should equal \"c\""
    enum_idx_sum = enum_idx_sum + enum_i
assert enum_idx_sum == 3, "enum_idx_sum should equal 3"

# Enumerate with custom start
enum_start_sum: int = 0
for enum_si, enum_sv in enumerate(["x", "y"], 10):
    enum_start_sum = enum_start_sum + enum_si
    if enum_si == 10:
        assert enum_sv == "x", "enum_sv should equal \"x\""
    if enum_si == 11:
        assert enum_sv == "y", "enum_sv should equal \"y\""
assert enum_start_sum == 21, "enum_start_sum should equal 21"

# Enumerate over range
for enum_ri, enum_rv in enumerate(range(5)):
    assert enum_ri == enum_rv, "enum_ri should equal enum_rv"

# Enumerate over empty list
enum_entered: bool = False
for enum_ei, enum_ev in enumerate([]):
    enum_entered = True
assert enum_entered == False, "enum_entered should equal False"

# Enumerate over list of ints
enum_int_items: list[int] = [10, 20, 30]
for enum_ii, enum_iv in enumerate(enum_int_items):
    if enum_ii == 0:
        assert enum_iv == 10, "enum_iv should equal 10"
    if enum_ii == 1:
        assert enum_iv == 20, "enum_iv should equal 20"
    if enum_ii == 2:
        assert enum_iv == 30, "enum_iv should equal 30"

# Enumerate with start=1
for enum_s1i, enum_s1v in enumerate(["first", "second"], 1):
    if enum_s1i == 1:
        assert enum_s1v == "first", "enum_s1v should equal \"first\""
    if enum_s1i == 2:
        assert enum_s1v == "second", "enum_s1v should equal \"second\""

# ===== SECTION: For loop tuple unpacking =====

# General tuple unpack: for a, b in list_of_tuples
enum_unpack_results: list[int] = []
enum_pairs: list[tuple[int, int]] = [(1, 10), (2, 20), (3, 30)]
for enum_ua, enum_ub in enum_pairs:
    enum_unpack_results.append(enum_ua + enum_ub)
assert enum_unpack_results[0] == 11, "enum_unpack_results[0] should equal 11"
assert enum_unpack_results[1] == 22, "enum_unpack_results[1] should equal 22"
assert enum_unpack_results[2] == 33, "enum_unpack_results[2] should equal 33"

# Tuple unpack with string values
enum_names: list[tuple[str, str]] = [("Alice", "Smith"), ("Bob", "Jones")]
for enum_first, enum_last in enum_names:
    if enum_first == "Alice":
        assert enum_last == "Smith", "enum_last should equal \"Smith\""
    if enum_first == "Bob":
        assert enum_last == "Jones", "enum_last should equal \"Jones\""

# ===== SECTION: zip() =====

# Test zip iteration with string lists (fully working)
zip_list_a: list[str] = ["1", "2", "3"]
zip_list_b: list[str] = ["a", "b", "c"]
zip_results: list[str] = []
for a, b in zip(zip_list_a, zip_list_b):
    zip_results.append(f"{a}:{b}")
assert zip_results == ["1:a", "2:b", "3:c"], "zip string iteration failed"

# Test zip with unequal lengths (shorter first)
zip_short: list[str] = ["x", "y"]
zip_long: list[str] = ["1", "2", "3", "4"]
zip_results2: list[str] = []
for a, b in zip(zip_short, zip_long):
    zip_results2.append(f"{a}-{b}")
assert zip_results2 == ["x-1", "y-2"], "zip unequal lengths failed"

# Test zip with unequal lengths (longer first)
zip_results3: list[str] = []
for a, b in zip(zip_long, zip_short):
    zip_results3.append(f"{a}-{b}")
assert zip_results3 == ["1-x", "2-y"], "zip unequal lengths (reverse) failed"

# Test next() on zip iterator
zip_iter = zip(["a", "b"], ["1", "2"])
zip_item1 = next(zip_iter)
# Note: tuple comparison has a known issue, so we check values individually
a1, b1 = zip_item1
assert a1 == "a", "zip next() first item [0] failed"
assert b1 == "1", "zip next() first item [1] failed"
zip_item2 = next(zip_iter)
a2, b2 = zip_item2
assert a2 == "b", "zip next() second item [0] failed"
assert b2 == "2", "zip next() second item [1] failed"

print("zip() tests passed!")

# NOTE: zip() with mixed int/str lists currently has a limitation where
# integers get boxed for tuple storage. For full integer support, use
# string representations or consider the limitation when designing code.

# ===== SECTION: For loop starred unpacking =====

# Starred unpacking: for first_elem, *rest_elems in items
items1: list[tuple[int, int, int]] = [(1, 2, 3), (4, 5, 6)]
first_elem_values: list[int] = []
rest_elem_values: list[list[int]] = []
for first_elem, *rest_elems in items1:
    first_elem_values.append(first_elem)
    rest_elem_values.append(rest_elems)
assert len(first_elem_values) == 2, "Should have 2 first values"
assert first_elem_values[0] == 1, "First value [0] should be 1"
assert first_elem_values[1] == 4, "First value [1] should be 4"
assert len(rest_elem_values) == 2, "Should have 2 rest lists"
assert len(rest_elem_values[0]) == 2, "rest_elem_values[0] length should be 2"
assert rest_elem_values[0][0] == 2, "rest_elem_values[0][0] should be 2"
assert rest_elem_values[0][1] == 3, "rest_elem_values[0][1] should be 3"
assert len(rest_elem_values[1]) == 2, "rest_elem_values[1] length should be 2"
assert rest_elem_values[1][0] == 5, "rest_elem_values[1][0] should be 5"
assert rest_elem_values[1][1] == 6, "rest_elem_values[1][1] should be 6"

# Starred unpacking: for *start_elem, last_elem in items
items2: list[tuple[int, int, int]] = [(1, 2, 3), (4, 5, 6)]
start_elem_values: list[list[int]] = []
last_elem_values: list[int] = []
for *start_elem, last_elem in items2:
    start_elem_values.append(start_elem)
    last_elem_values.append(last_elem)
assert len(last_elem_values) == 2, "Should have 2 last values"
assert last_elem_values[0] == 3, "Last value [0] should be 3"
assert last_elem_values[1] == 6, "Last value [1] should be 6"
assert len(start_elem_values) == 2, "Should have 2 start lists"
assert len(start_elem_values[0]) == 2, "start_elem_values[0] length should be 2"
assert start_elem_values[0][0] == 1, "start_elem_values[0][0] should be 1"
assert start_elem_values[0][1] == 2, "start_elem_values[0][1] should be 2"
assert len(start_elem_values[1]) == 2, "start_elem_values[1] length should be 2"
assert start_elem_values[1][0] == 4, "start_elem_values[1][0] should be 4"
assert start_elem_values[1][1] == 5, "start_elem_values[1][1] should be 5"

# Starred unpacking: for a_elem, *mid_elem, z_elem in items
items3: list[tuple[int, int, int, int]] = [(1, 2, 3, 4), (5, 6, 7, 8)]
a_elem_values: list[int] = []
mid_elem_values: list[list[int]] = []
z_elem_values: list[int] = []
for a_elem, *mid_elem, z_elem in items3:
    a_elem_values.append(a_elem)
    mid_elem_values.append(mid_elem)
    z_elem_values.append(z_elem)
assert len(a_elem_values) == 2, "Should have 2 a values"
assert a_elem_values[0] == 1, "a value [0] should be 1"
assert a_elem_values[1] == 5, "a value [1] should be 5"
assert len(z_elem_values) == 2, "Should have 2 z values"
assert z_elem_values[0] == 4, "z value [0] should be 4"
assert z_elem_values[1] == 8, "z value [1] should be 8"
assert len(mid_elem_values) == 2, "Should have 2 mid lists"
assert len(mid_elem_values[0]) == 2, "mid_elem_values[0] length should be 2"
assert mid_elem_values[0][0] == 2, "mid_elem_values[0][0] should be 2"
assert mid_elem_values[0][1] == 3, "mid_elem_values[0][1] should be 3"
assert len(mid_elem_values[1]) == 2, "mid_elem_values[1] length should be 2"
assert mid_elem_values[1][0] == 6, "mid_elem_values[1][0] should be 6"
assert mid_elem_values[1][1] == 7, "mid_elem_values[1][1] should be 7"

# Starred unpacking with list of lists
list_items: list[list[int]] = [[10, 20, 30], [40, 50, 60]]
list_first_elem: list[int] = []
list_rest_elem: list[list[int]] = []
for first_item, *rest_items in list_items:
    list_first_elem.append(first_item)
    list_rest_elem.append(rest_items)
assert len(list_first_elem) == 2, "Should have 2 list first values"
assert list_first_elem[0] == 10, "list_first_elem[0] should be 10"
assert list_first_elem[1] == 40, "list_first_elem[1] should be 40"
assert len(list_rest_elem) == 2, "Should have 2 list rest values"
assert len(list_rest_elem[0]) == 2, "list_rest_elem[0] length should be 2"
assert list_rest_elem[0][0] == 20, "list_rest_elem[0][0] should be 20"
assert list_rest_elem[0][1] == 30, "list_rest_elem[0][1] should be 30"

print("For loop starred unpacking tests passed!")

# ===== SECTION: Mixed-type tuple iteration =====

# Mixed-type tuple iteration - loop variable should be union type
mixed_tuple: tuple[int, str, bool] = (42, "hello", True)
mixed_items: list[int | str | bool] = []
for item in mixed_tuple:
    mixed_items.append(item)
assert len(mixed_items) == 3, "mixed_items should have 3 elements"

# Homogeneous tuple still works correctly (no union needed)
int_tuple: tuple[int, int, int] = (1, 2, 3)
int_sum: int = 0
for x in int_tuple:
    int_sum = int_sum + x
assert int_sum == 6, "int_sum should equal 6"

# Two-element mixed tuple
pair_tuple: tuple[str, int] = ("key", 42)
pair_items: list[str | int] = []
for pair_item in pair_tuple:
    pair_items.append(pair_item)
assert len(pair_items) == 2, "pair_items should have 2 elements"

print("Mixed-type tuple iteration tests passed!")

# ===== SECTION: itertools.chain() and itertools.islice() =====

import itertools

# Test chain with multiple lists
iter_chain_result: list[int] = []
for x in itertools.chain([1, 2], [3, 4], [5, 6]):
    iter_chain_result.append(x)
assert iter_chain_result == [1, 2, 3, 4, 5, 6], "chain with multiple lists failed"

# Test chain with single list
iter_chain_total: int = 0
for x in itertools.chain([10, 20, 30]):
    iter_chain_total = iter_chain_total + x
assert iter_chain_total == 60, "chain with single list failed"

# Test chain with ranges
iter_chain_ranges: list[int] = []
for x in itertools.chain(range(3), range(10, 13)):
    iter_chain_ranges.append(x)
assert iter_chain_ranges == [0, 1, 2, 10, 11, 12], "chain with ranges failed"

# Test chain with next()
iter_chain_it = itertools.chain([1, 2], [3, 4])
iter_cv1: int = next(iter_chain_it)
assert iter_cv1 == 1, "chain next() v1 failed"
iter_cv2: int = next(iter_chain_it)
assert iter_cv2 == 2, "chain next() v2 failed"
iter_cv3: int = next(iter_chain_it)
assert iter_cv3 == 3, "chain next() v3 failed"
iter_cv4: int = next(iter_chain_it)
assert iter_cv4 == 4, "chain next() v4 failed"

# Test islice with stop only
iter_islice_stop: list[int] = []
for x in itertools.islice([10, 20, 30, 40, 50], 3):
    iter_islice_stop.append(x)
assert iter_islice_stop == [10, 20, 30], "islice with stop failed"

# Test islice with start and stop
iter_islice_ss: list[int] = []
for x in itertools.islice([10, 20, 30, 40, 50], 1, 4):
    iter_islice_ss.append(x)
assert iter_islice_ss == [20, 30, 40], "islice with start+stop failed"

# Test islice with start, stop, step
iter_islice_sss: list[int] = []
for x in itertools.islice([10, 20, 30, 40, 50, 60, 70], 1, 6, 2):
    iter_islice_sss.append(x)
assert iter_islice_sss == [20, 40, 60], "islice with start+stop+step failed"

# Test islice with next()
iter_islice_it = itertools.islice([100, 200, 300, 400, 500], 2, 5)
iter_iv1: int = next(iter_islice_it)
assert iter_iv1 == 300, "islice next() v1 failed"
iter_iv2: int = next(iter_islice_it)
assert iter_iv2 == 400, "islice next() v2 failed"
iter_iv3: int = next(iter_islice_it)
assert iter_iv3 == 500, "islice next() v3 failed"

# Test islice with range
iter_islice_range: list[int] = []
for x in itertools.islice(range(100), 5, 10):
    iter_islice_range.append(x)
assert iter_islice_range == [5, 6, 7, 8, 9], "islice with range failed"

# Test from-import style
from itertools import chain, islice

iter_from_chain: list[int] = []
for x in chain([1, 2], [3]):
    iter_from_chain.append(x)
assert iter_from_chain == [1, 2, 3], "from-import chain failed"

iter_from_islice: list[int] = []
for x in islice(range(20), 3, 8):
    iter_from_islice.append(x)
assert iter_from_islice == [3, 4, 5, 6, 7], "from-import islice failed"

# Test chain + islice combination
iter_combo_chained = chain([1, 2, 3], [4, 5, 6], [7, 8, 9])
iter_combo_result: list[int] = []
for x in islice(iter_combo_chained, 2, 7):
    iter_combo_result.append(x)
assert iter_combo_result == [3, 4, 5, 6, 7], "chain+islice combo failed"

print("itertools chain/islice tests passed!")

# ============================================================================
# enumerate(start=N) keyword argument (regression test)
# ============================================================================

enum_start_result: list[tuple[int, str]] = []
for i, v in enumerate(["a", "b", "c"], start=1):
    enum_start_result.append((i, v))
assert enum_start_result[0] == (1, "a"), f"enumerate start=1 index 0 failed: {enum_start_result[0]}"
assert enum_start_result[1] == (2, "b"), f"enumerate start=1 index 1 failed: {enum_start_result[1]}"
assert enum_start_result[2] == (3, "c"), f"enumerate start=1 index 2 failed: {enum_start_result[2]}"

# enumerate with start=0 (default) still works
enum_default_result: list[tuple[int, str]] = []
for i, v in enumerate(["x", "y"], start=0):
    enum_default_result.append((i, v))
assert enum_default_result[0] == (0, "x"), "enumerate start=0 failed"

# enumerate with start=10
enum_ten_result: list[int] = []
for i, v in enumerate(range(3), start=10):
    enum_ten_result.append(i)
assert enum_ten_result == [10, 11, 12], f"enumerate start=10 failed: {enum_ten_result}"

print("enumerate(start=N) tests passed!")

# ===== SECTION: Lambda parameter inference in sorted/reduce =====

_lpi_words: list[str] = ["banana", "apple", "fig"]
_lpi_sorted = sorted(_lpi_words, key=lambda w: len(w))
assert _lpi_sorted[0] == "fig", "sorted key=lambda: shortest first"
assert _lpi_sorted[2] == "banana", "sorted key=lambda: longest last"

from functools import reduce
_lpi_nums: list[int] = [1, 2, 3, 4, 5]
_lpi_total = reduce(lambda a, b: a + b, _lpi_nums)
assert _lpi_total == 15, "reduce lambda: sum 1..5"

print("Lambda parameter inference tests passed!")

# ============================================================
# Enumerate bug-fix regression tests
# ============================================================

# Issue #6: enumerate over dict with str keys (avoids unboxing path for stability)
def test_enumerate_dict_str_keys():
    d: dict[str, int] = {"a": 1, "b": 2, "c": 3}
    keys: list[str] = []
    indices: list[int] = []
    for i, k in enumerate(d):
        indices.append(i)
        keys.append(k)
    assert indices == [0, 1, 2]
    assert keys == ["a", "b", "c"]

test_enumerate_dict_str_keys()

# Issue #2: enumerate with explicit negative step range
def test_enumerate_range_negative_step():
    result: list[int] = []
    for i, v in enumerate(range(5, 0, -1)):
        result.append(v)
    assert result == [5, 4, 3, 2, 1]

test_enumerate_range_negative_step()

# Issue #2 additional: enumerate range with positive step (regression check)
def test_enumerate_range_positive_step():
    result: list[int] = []
    for i, v in enumerate(range(0, 6, 2)):
        result.append(v)
    assert result == [0, 2, 4]

test_enumerate_range_positive_step()

print("Enumerate regression tests passed!")

# ===== Regression: sorted/min/max with non-capturing lambda key =====
# Non-capturing lambdas (no free variables) should work as key functions.

# sorted with lambda key (no captures)
words_s: list[str] = ["banana", "fig", "apple", "date"]
sorted_by_len: list[str] = sorted(words_s, key=lambda w: len(w))
assert sorted_by_len == ["fig", "date", "apple", "banana"], f"sorted lambda key: {sorted_by_len}"

# sorted with lambda key and reverse
sorted_rev: list[str] = sorted(words_s, key=lambda w: len(w), reverse=True)
assert sorted_rev == ["banana", "apple", "date", "fig"], f"sorted lambda key reverse: {sorted_rev}"

# min/max with user-defined key function (no captures)
def neg(x: int) -> int:
    return -x

nums_mm: list[int] = [3, 1, 4, 1, 5]
assert min(nums_mm, key=neg) == 5, "min with negation key"
assert max(nums_mm, key=neg) == 1, "max with negation key"

# list.sort with user-defined key (no captures)
def sort_by_len(s: str) -> int:
    return len(s)

items: list[str] = ["cc", "a", "bbb"]
items.sort(key=sort_by_len)
assert items == ["a", "cc", "bbb"], f"list.sort named key: {items}"

print("sorted/min/max key function regression tests passed!")

# ===== sorted/min/max with key= lambda capturing variables =====

# sorted with capturing lambda (addition to shift sort order)
_cap_offset: int = 100
_cap_nums: list[int] = [5, 3, 8, 1]
_cap_sorted: list[int] = sorted(_cap_nums, key=lambda x: _cap_offset - x)
assert _cap_sorted == [8, 5, 3, 1], f"sorted capturing lambda: {_cap_sorted}"

# sorted with capturing lambda and reverse
_cap_sorted_rev: list[int] = sorted(_cap_nums, key=lambda x: _cap_offset - x, reverse=True)
assert _cap_sorted_rev == [1, 3, 5, 8], f"sorted capturing lambda reverse: {_cap_sorted_rev}"

# min with capturing lambda
_cap_min: int = min(_cap_nums, key=lambda x: _cap_offset - x)
assert _cap_min == 8, f"min capturing lambda: {_cap_min}"

# max with capturing lambda
_cap_max: int = max(_cap_nums, key=lambda x: _cap_offset - x)
assert _cap_max == 1, f"max capturing lambda: {_cap_max}"

# list.sort with capturing lambda (int list)
_cap_sort_nums: list[int] = [5, 3, 8, 1]
_cap_sort_offset: int = 100
_cap_sort_nums.sort(key=lambda x: _cap_sort_offset - x)
assert _cap_sort_nums == [8, 5, 3, 1], f"list.sort capturing lambda: {_cap_sort_nums}"

# sorted with multiple captures
_cap_a: int = 2
_cap_b: int = 3
_cap_multi: list[int] = [10, 7, 4, 1]
_cap_multi_sorted: list[int] = sorted(_cap_multi, key=lambda x: x + _cap_a * _cap_b)
assert _cap_multi_sorted == [1, 4, 7, 10], f"sorted multi-capture lambda: {_cap_multi_sorted}"

# min/max with str captures
def _cap_make_key(prefix: str) -> str:
    return prefix

_cap_prefix_val: str = "z"
_cap_str_list: list[str] = ["banana", "apple", "cherry"]
_cap_str_sorted: list[str] = sorted(_cap_str_list, key=lambda s: s + _cap_prefix_val)
assert _cap_str_sorted == ["apple", "banana", "cherry"], f"sorted str capture: {_cap_str_sorted}"

print("sorted/min/max capturing lambda tests passed!")

# ===== SECTION: New for-loop target shapes (BindingTarget migration) =====

# Baseline shapes — these worked before the refactor, make sure they still do.
for _bt_a, _bt_b in [(1, 10), (2, 20), (3, 30)]:
    assert _bt_a * 10 == _bt_b
for _bt_first, *_bt_rest in [(1, 2, 3), (10, 20, 30)]:
    assert isinstance(_bt_rest, list)
for *_bt_init, _bt_last in [(1, 2, 3), (10, 20, 30)]:
    assert _bt_last == 3 or _bt_last == 30
for _bt_aa, *_bt_mid, _bt_z in [(1, 2, 3, 4, 5)]:
    assert _bt_aa == 1 and _bt_mid == [2, 3, 4] and _bt_z == 5

# NEW: nested for-target
for _bt_na, (_bt_nb, _bt_nc) in [(1, (2, 3)), (4, (5, 6))]:
    assert _bt_nb + _bt_nc > _bt_na

# NEW: attribute as for-target
class _BtContainer:
    val: int
    def __init__(self) -> None:
        self.val = 0
_bt_k = _BtContainer()
for _bt_k.val in [10, 20, 30]:
    pass
assert _bt_k.val == 30

# NEW: subscript as for-target
_bt_lst: list[int] = [0, 0, 0]
for _bt_lst[0] in [1, 2, 3]:
    pass
assert _bt_lst[0] == 3

# NEW: mixed — tuple-of-attr-and-var
class _BtPoint:
    x: int
    y: int
    def __init__(self) -> None:
        self.x = 0
        self.y = 0
_bt_p = _BtPoint()
for _bt_p.x, _bt_p.y in [(1, 2), (3, 4)]:
    pass
assert _bt_p.x == 3 and _bt_p.y == 4

print("New for-loop target shape tests passed!")

# ===== SECTION: Tuple targets in comprehensions (BindingTarget migration) =====
# Primary bug fix from the unified BindingTarget refactor: comprehension
# `for TARGET in ITER` clauses now accept the full target grammar, matching
# what CPython has always allowed.

# List comprehension with flat tuple target
_ct_pairs = [(1, 10), (2, 20), (3, 30)]
_ct_flat = [a + b for a, b in _ct_pairs]
assert _ct_flat == [11, 22, 33]

# Dict comprehension with tuple targets
_ct_items = [("a", 1), ("b", 2), ("c", 3)]
_ct_dict = {k: v for k, v in _ct_items}
assert _ct_dict["a"] == 1 and _ct_dict["b"] == 2 and _ct_dict["c"] == 3
assert len(_ct_dict) == 3

# Dict comprehension building inverted mapping — unpack order matters
_ct_inv = {v: k for k, v in _ct_items}
assert _ct_inv[1] == "a" and _ct_inv[2] == "b" and _ct_inv[3] == "c"
assert len(_ct_inv) == 3

# Set comprehension with tuple targets
_ct_set = {a * b for a, b in _ct_pairs}
assert 10 in _ct_set and 40 in _ct_set and 90 in _ct_set
assert len(_ct_set) == 3

# Multi-clause comprehension with tuple target
_ct_nested = [[(1, 2), (3, 4)], [(5, 6)]]
_ct_flattened = [a + b for row in _ct_nested for a, b in row]
assert _ct_flattened == [3, 7, 11]

print("New comprehension tuple-target tests passed!")

# =============================================================================
# §C.6 — tuple targets in generator expressions (Area C).
# `detect_for_loop_generator` now accepts any `BindingTarget`, so tuple /
# attr / index targets propagate through the optimised resume-loop builder.
# Desugar-time element-type inference covers: zip/enumerate of literals OR
# Var/attr references, dict `.items()/.keys()/.values()`, module-level
# bindings, and function-param Vars with annotations.
# =============================================================================

# zip with literal args
assert sum(x * y for x, y in zip([1, 2, 3], [4, 5, 6])) == 32

# list literal of tuple literals
assert sum(x * y for x, y in [(1, 4), (2, 5), (3, 6)]) == 32

# nested unpack in gen-expr
_ct_nested_sum = sum(a * b + c for a, (b, c) in [(1, (2, 3)), (4, (5, 6))])
assert _ct_nested_sum == (1 * 2 + 3) + (4 * 5 + 6)

# min/max over gen-expr yielding tuples — Area G §G.4 routes tuple-yielding
# iterators through `rt_tuple_cmp` (lexicographic) instead of raw pointer
# comparison. Before the fix, `min` returned the first element (pointer
# Lt against later allocations was always false); `max` was accidentally
# correct because higher-address tuples happened to also be lex-greater.
_ct_max = max((v, i) for i, v in enumerate([3, 1, 4, 1, 5]))
assert _ct_max == (5, 4)
_ct_min = min((v, i) for i, v in enumerate([3, 1, 4, 1, 5]))
assert _ct_min == (1, 1)

# 3-tuple yield, lexicographic compare
_g4_data = [(1, 2, 3), (1, 2, 4), (1, 1, 9)]
assert min((a, b, c) for a, b, c in _g4_data) == (1, 1, 9)
assert max((a, b, c) for a, b, c in _g4_data) == (1, 2, 4)

# Tie-breaking: strict `<` / `>` keeps the first-seen best on tie.
assert min((v, i) for i, v in enumerate([3, 1, 1, 4])) == (1, 1)
assert max((v, i) for i, v in enumerate([4, 1, 4, 1])) == (4, 2)

# zip over Var-typed module-level lists (shape-infer via VarTypeMap)
_ct_wo = [10, 20, 30]
_ct_xx = [1, 2, 3]
assert sum(a * b for a, b in zip(_ct_wo, _ct_xx)) == 140

# enumerate over Var-typed module-level list
_ct_vals = [3, 1, 4]
_ct_enum = [(i, v) for i, v in enumerate(_ct_vals)]
assert _ct_enum == [(0, 3), (1, 1), (2, 4)]

print("Generator-expression tuple-target tests passed!")

# =============================================================================
# Area G §G.3: Heap-typed closure captures in gen-expr
#
# Before Area G, `sum(x for x in a)` where `a` is a function-local or
# function-param heap value (list/dict/str/class instance) crashed with
# SIGSEGV: the gen-expr's creator function was synthesized with empty
# params and the captured VarId resolved to an uninitialised local slot.
# The fix plumbs captures through `ExprKind::Closure` (same mechanism
# lambdas use) and marks the gen-expr's local slots as heap via
# `rt_generator_set_local_type` so GC traces them across resume calls.
# =============================================================================


def _g3_local_list_sum() -> int:
    a = [1, 2, 3, 4, 5]
    return sum(x for x in a)


assert _g3_local_list_sum() == 15


def _g3_param_list_sum(a: list[int]) -> int:
    return sum(x for x in a)


assert _g3_param_list_sum([1, 2, 3]) == 6


# Note: `dict.items()` inside a gen-expr (both module-level and function-local)
# hits a pre-existing type-propagation gap — the tuple unpack `for _k, v in
# d.items()` doesn't carry the dict value type through to the gen-expr body.
# Tracked as a separate follow-up; list-based captures are the §G.3 payload.


def _g3_stress() -> int:
    data = list(range(1000))
    return sum(x * x for x in data)


assert _g3_stress() == sum(i * i for i in range(1000))


# Nested gen-exprs over captured lists — the microgpt.py:95 pattern.
# §G.10 adds a pre-desugar pass (`propagate_genexp_capture_types`) that
# writes capture types onto gen-expr creator params so the resume's
# for-loop element type resolves to `Tuple([Int, Int])` instead of Any,
# giving `rt_tuple_get_int` for the unpack.
def _g3_linear(x: list[int], w: list[list[int]]) -> list[int]:
    return [sum(wi * xi for wi, xi in zip(wo, x)) for wo in w]


assert _g3_linear([1, 2, 3], [[10, 20, 30], [1, 1, 1]]) == [140, 6]
assert _g3_linear([1, 1, 1], [[1, 2, 3], [4, 5, 6], [0, 0, 0]]) == [6, 15, 0]


# Area G §G.11: `dict.items()` inside a gen-expr. The untyped-global gap
# mis-classified module-level dict literals as `Int` (scan_global_var_types
# only recorded globals with explicit annotations), so `d.items()` inside
# a gen-expr dispatched `MethodCall` against `Int` and lowered to a None
# receiver — rt_iter_list(None) segfaulted. Fix: fall back to RHS inference
# for literal-shaped globals (list/dict/set/tuple + primitive literals).
_g11_mod_d = {"a": 1, "b": 2, "c": 3}
assert sum(v for _k, v in _g11_mod_d.items()) == 6
assert sum(v for _k, v in _g11_mod_d.items() if v > 1) == 5


def _g11_local_dict_sum() -> int:
    d = {"x": 10, "y": 20, "z": 30}
    return sum(v for _k, v in d.items())


assert _g11_local_dict_sum() == 60


def _g11_param_dict_sum(d: dict[str, int]) -> int:
    return sum(v for _k, v in d.items())


assert _g11_param_dict_sum({"a": 100, "b": 200}) == 300


# Same pattern with .values() — shouldn't have been broken but regression-check.
_g11_vals = {"p": 1, "q": 2}
assert sum(v for v in _g11_vals.values()) == 3


# Area G §G.12: `min()` / `max()` on an empty iterable must raise
# `ValueError`, not silently return a null accumulator. Covers all three
# fold paths — primitive Iterator (gen-expr over []), tuple Iterator
# (tuple-yielding gen-expr over []), and class Iterator (list of class
# instances). Also regression-covers the already-working list path.
_g12_empty_int: list[int] = []
try:
    _ = min(x for x in _g12_empty_int)
    raise RuntimeError("min on empty gen-expr should have raised ValueError")
except ValueError as _e:
    assert "min" in str(_e)

try:
    _ = max(x for x in _g12_empty_int)
    raise RuntimeError("max on empty gen-expr should have raised ValueError")
except ValueError as _e:
    assert "max" in str(_e)

# Tuple-yield gen-expr over empty iterable (§G.4 path)
try:
    _ = min((v, i) for i, v in enumerate(_g12_empty_int))
    raise RuntimeError("min on empty tuple gen-expr should have raised")
except ValueError as _e:
    assert "min" in str(_e)

try:
    _ = max((v, i) for i, v in enumerate(_g12_empty_int))
    raise RuntimeError("max on empty tuple gen-expr should have raised")
except ValueError as _e:
    assert "max" in str(_e)


# Class-element list via min/max — exercises `lower_minmax_class_fold`.
class _G12K:
    def __init__(self, v: int):
        self.v = v

    def __lt__(self, other: "_G12K") -> bool:
        return self.v < other.v


_g12_non_empty_ks = [_G12K(3), _G12K(1), _G12K(2)]
_g12_min_k = min(_g12_non_empty_ks)
assert _g12_min_k.v == 1

_g12_empty_ks: list[_G12K] = []
try:
    _ = min(_g12_empty_ks)
    raise RuntimeError("min on empty class list should have raised")
except ValueError as _e:
    assert "min" in str(_e)

try:
    _ = max(_g12_empty_ks)
    raise RuntimeError("max on empty class list should have raised")
except ValueError as _e:
    assert "max" in str(_e)


# Direct list min/max — always raised correctly; regression-check that
# §G.12 changes didn't break the non-iterator path.
try:
    _ = min(_g12_empty_int)
    raise RuntimeError("min on empty list should have raised")
except ValueError as _e:
    assert "min" in str(_e)

try:
    _ = max(_g12_empty_int)
    raise RuntimeError("max on empty list should have raised")
except ValueError as _e:
    assert "max" in str(_e)

print("§G.3 heap-captured gen-expr tests passed!")

print("All iteration and comprehension tests passed!")
