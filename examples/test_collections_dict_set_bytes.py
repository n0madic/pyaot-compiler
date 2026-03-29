# Consolidated test file for dict, set, and bytes collections

# ===== SECTION: Dict creation, indexing, assignment =====

# Dict creation with string keys
d: dict[str, int] = {"a": 1, "b": 2, "c": 3}
assert len(d) == 3, "len(d) should equal 3"

# Dict indexing
assert d["a"] == 1, "d[\"a\"] should equal 1"
assert d["b"] == 2, "d[\"b\"] should equal 2"
assert d["c"] == 3, "d[\"c\"] should equal 3"

# Dict set (indexed assignment)
d["d"] = 4
assert d["d"] == 4, "d[\"d\"] should equal 4"
assert len(d) == 4, "len(d) should equal 4"

# Update existing key
d["a"] = 10
assert d["a"] == 10, "d[\"a\"] should equal 10"

# ===== SECTION: Dict 'in' operator =====

assert "a" in d, "\"a\" should be in d"
assert "b" in d, "\"b\" should be in d"
assert "z" not in d, "\"z\" not should be in d"
assert "missing" not in d, "\"missing\" not should be in d"

# Dict with integer keys
nums: dict[int, str] = {1: "one", 2: "two", 3: "three"}
assert len(nums) == 3, "len(nums) should equal 3"
assert nums[1] == "one", "nums[1] should equal \"one\""
assert nums[2] == "two", "nums[2] should equal \"two\""
assert nums[3] == "three", "nums[3] should equal \"three\""

# Integer key lookup
assert 1 in nums, "1 should be in nums"
assert 2 in nums, "2 should be in nums"
assert 99 not in nums, "99 not should be in nums"

# Empty dict
empty_dict: dict[str, int] = {}
assert len(empty_dict) == 0, "len(empty_dict) should equal 0"
assert "key" not in empty_dict, "\"key\" not should be in empty_dict"

# ===== SECTION: Dict methods (get, keys, values, items, pop, clear, copy) =====

# Dict methods: .get()
x = d.get("a")
assert x == 10, "x should equal 10"

# Dict methods: .clear()
copy_d: dict[str, int] = {"x": 1, "y": 2}
assert len(copy_d) == 2, "len(copy_d) should equal 2"
copy_d.clear()
assert len(copy_d) == 0, "len(copy_d) should equal 0"

# Dict methods: .copy()
original_dict: dict[str, int] = {"m": 100, "n": 200}
copied_dict: dict[str, int] = original_dict.copy()
assert len(copied_dict) == 2, "len(copied_dict) should equal 2"
assert copied_dict["m"] == 100, "copied_dict[\"m\"] should equal 100"
assert copied_dict["n"] == 200, "copied_dict[\"n\"] should equal 200"

# Modifying copy doesn't affect original
copied_dict["m"] = 999
assert original_dict["m"] == 100, "original_dict[\"m\"] should equal 100"

# Dict methods: .pop()
pop_test: dict[str, int] = {"a": 1, "b": 2, "c": 3}
val: int = pop_test.pop("b")
assert val == 2, "val should equal 2"
assert len(pop_test) == 2, "len(pop_test) should equal 2"
assert "b" not in pop_test, "\"b\" not should be in pop_test"

# Multiple operations
data: dict[str, int] = {}
data["first"] = 1
data["second"] = 2
data["third"] = 3
assert len(data) == 3, "len(data) should equal 3"
assert data["first"] == 1, "data[\"first\"] should equal 1"
data["first"] = 100
assert data["first"] == 100, "data[\"first\"] should equal 100"

# dict.update()
d1: dict[str, int] = {"a": 1, "b": 2}
d2: dict[str, int] = {"b": 20, "c": 3}
d1.update(d2)
assert d1["a"] == 1, "d1[\"a\"] should equal 1"
assert d1["b"] == 20, "d1[\"b\"] should equal 20"  # overwritten
assert d1["c"] == 3, "d1[\"c\"] should equal 3"   # added
assert len(d1) == 3, "len(d1) should equal 3"
print("dict.update() passed")

# ===== SECTION: Set literals and set() constructor =====

# Basic set literal
s: set[int] = {1, 2, 3}
assert len(s) == 3, "len(s) should equal 3"
assert 1 in s, "1 should be in s"
assert 2 in s, "2 should be in s"
assert 3 in s, "3 should be in s"
assert 4 not in s, "4 not should be in s"

# Empty set via set()
empty_set: set[int] = set()
assert len(empty_set) == 0, "len(empty_set) should equal 0"

# Set from list
from_list: set[int] = set([1, 2, 2, 3])
assert len(from_list) == 3, "len(from_list) should equal 3"

# String set
chars: set[str] = {"a", "b", "c"}
assert len(chars) == 3, "len(chars) should equal 3"
assert "a" in chars, "\"a\" should be in chars"
assert "b" in chars, "\"b\" should be in chars"
assert "c" in chars, "\"c\" should be in chars"

# ===== SECTION: Set methods (add, remove, discard, clear, copy) =====

# set.add()
s2: set[int] = set()
s2.add(1)
s2.add(2)
s2.add(1)  # Duplicate - no effect
assert len(s2) == 2, "len(s2) should equal 2"
assert 1 in s2, "1 should be in s2"
assert 2 in s2, "2 should be in s2"

# set.remove()
s3: set[int] = {1, 2, 3}
s3.remove(2)
assert len(s3) == 2, "len(s3) should equal 2"
assert 2 not in s3, "2 not should be in s3"

# set.discard() - no error if missing
s4: set[int] = {1, 2}
s4.discard(2)
s4.discard(99)  # No error
assert len(s4) == 1, "len(s4) should equal 1"
assert 2 not in s4, "2 not should be in s4"

# set.clear()
s5: set[int] = {1, 2, 3}
s5.clear()
assert len(s5) == 0, "len(s5) should equal 0"

# set.copy()
original_set: set[int] = {1, 2, 3}
copied_set: set[int] = original_set.copy()
assert len(copied_set) == 3, "len(copied_set) should equal 3"
assert 1 in copied_set, "1 should be in copied_set"
assert 2 in copied_set, "2 should be in copied_set"
assert 3 in copied_set, "3 should be in copied_set"

# ===== SECTION: Set iteration =====

total: int = 0
for set_x in {1, 2, 3}:
    total = total + set_x
assert total == 6, "total should equal 6"

# ===== SECTION: Set comprehensions =====

# Set comprehension
squares: set[int] = {set_x * set_x for set_x in range(5)}
assert len(squares) == 5, "len(squares) should equal 5"
assert 0 in squares, "0 should be in squares"
assert 1 in squares, "1 should be in squares"
assert 4 in squares, "4 should be in squares"
assert 9 in squares, "9 should be in squares"
assert 16 in squares, "16 should be in squares"

# Set comprehension with filter
evens_set: set[int] = {set_x for set_x in range(10) if set_x % 2 == 0}
assert len(evens_set) == 5, "len(evens_set) should equal 5"
assert 0 in evens_set, "0 should be in evens_set"
assert 2 in evens_set, "2 should be in evens_set"
assert 4 in evens_set, "4 should be in evens_set"
assert 6 in evens_set, "6 should be in evens_set"
assert 8 in evens_set, "8 should be in evens_set"

# ===== SECTION: Bytes literals and operations =====

def test_bytes_literal():
    b1: bytes = b'hello'
    assert len(b1) == 5, "len(b1) should equal 5"

def test_bytes_empty():
    empty_bytes: bytes = bytes()
    assert len(empty_bytes) == 0, "len(empty_bytes) should equal 0"
    assert empty_bytes == b'', "empty_bytes should equal b''"

def test_bytes_zero():
    zeros: bytes = bytes(5)
    assert len(zeros) == 5, "len(zeros) should equal 5"
    for i in range(5):
        assert zeros[i] == 0, "zeros[i] should equal 0"

def test_bytes_from_list():
    bytes_data: bytes = bytes([65, 66, 67])
    assert bytes_data == b'ABC', "bytes_data should equal b'ABC'"
    assert len(bytes_data) == 3, "len(bytes_data) should equal 3"

def test_bytes_from_str():
    bytes_data: bytes = bytes("hello", "utf-8")
    assert bytes_data == b'hello', "bytes_data should equal b'hello'"

def test_bytes_indexing():
    bytes_data: bytes = b'ABC'
    assert bytes_data[0] == 65, "bytes_data[0] should equal 65"
    assert bytes_data[-1] == 67, "bytes_data[-1] should equal 67"

def test_bytes_iteration():
    result: list[int] = []
    for b in b'hello':
        result.append(b)
    # Verify iteration yields correct byte values
    assert len(result) == 5, "len(result) should equal 5"
    assert result[0] == 104, "result[0] should equal 104"  # 'h'
    assert result[1] == 101, "result[1] should equal 101"  # 'e'
    assert result[2] == 108, "result[2] should equal 108"  # 'l'
    assert result[3] == 108, "result[3] should equal 108"  # 'l'
    assert result[4] == 111, "result[4] should equal 111"  # 'o'

def test_bytes_slicing():
    bytes_data: bytes = b'hello'
    assert bytes_data[1:4] == b'ell', "bytes_data[1:4] should equal b'ell'"

def test_bytes_return():
    def get_bytes() -> bytes:
        return b"test"
    assert get_bytes() == b"test", "get_bytes() should equal b\"test\""

# Run all bytes tests
test_bytes_literal()
test_bytes_empty()
test_bytes_zero()
test_bytes_from_list()
test_bytes_from_str()
test_bytes_indexing()
test_bytes_iteration()
test_bytes_slicing()
test_bytes_return()

# ===== SECTION: Container Printing =====

# Dict printing - str keys, int values
print_dict: dict[str, int] = {"a": 1, "b": 2}
print(print_dict)

# Dict printing - str keys, str values
print_dict_str: dict[str, str] = {"name": "Alice", "city": "NYC"}
print(print_dict_str)

# Empty dict
print_empty_dict: dict[str, int] = {}
print(print_empty_dict)

# Set printing
print_set: set[str] = {"x", "y"}
print(print_set)

# ===== SECTION: del statement =====

# del dict[key]
del_dict: dict[str, int] = {"a": 1, "b": 2, "c": 3}
del del_dict["b"]
assert len(del_dict) == 2, "del dict[key] len failed"
assert "b" not in del_dict, "del dict[key] should remove key"
assert "a" in del_dict, "del dict[key] should keep other keys"
assert "c" in del_dict, "del dict[key] should keep other keys"

# del list[index]
del_list: list[int] = [10, 20, 30, 40]
del del_list[1]
assert len(del_list) == 3, "del list[idx] len failed"
assert del_list[0] == 10, "del list[idx] first elem failed"
assert del_list[1] == 30, "del list[idx] shifted elem failed"
assert del_list[2] == 40, "del list[idx] last elem failed"

print("del statement tests passed!")

# ===== SECTION: dict.setdefault() and dict.popitem() =====

# dict.setdefault() - existing key
setdefault_dict: dict[str, int] = {"a": 1, "b": 2}
result_a: int = setdefault_dict.setdefault("a", 999)
assert result_a == 1, "setdefault should return existing value"
assert setdefault_dict["a"] == 1, "setdefault should not modify existing key"

# dict.setdefault() - new key
result_c: int = setdefault_dict.setdefault("c", 3)
assert result_c == 3, "setdefault should return default for new key"
assert setdefault_dict["c"] == 3, "setdefault should set new key"
assert len(setdefault_dict) == 3, "setdefault should increase dict size"

# dict.setdefault() - with None default
setdefault_none: dict[str, int] = {"x": 10}
result_y: int = setdefault_none.setdefault("y", 0)
assert result_y == 0, "setdefault with 0 default should work"
assert setdefault_none["y"] == 0, "setdefault with 0 should set value"

# dict.popitem() - basic usage
popitem_dict: dict[str, int] = {"a": 1, "b": 2, "c": 3}
last_item: tuple[str, int] = popitem_dict.popitem()
assert len(last_item) == 2, "popitem should return 2-tuple"
assert len(popitem_dict) == 2, "popitem should reduce dict size"
# Verify the removed key is no longer present
removed_key: str = last_item[0]
assert removed_key not in popitem_dict, "popitem should remove the key"

# dict.popitem() - until empty
popitem_dict2: dict[str, int] = {"x": 100, "y": 200}
item1: tuple[str, int] = popitem_dict2.popitem()
assert len(popitem_dict2) == 1, "first popitem should leave 1 item"
item2: tuple[str, int] = popitem_dict2.popitem()
assert len(popitem_dict2) == 0, "second popitem should empty dict"

print("dict.setdefault() and dict.popitem() tests passed!")

# ===== SECTION: Set operations (|, &, -, ^) and methods =====

set_a: set[int] = {1, 2, 3}
set_b: set[int] = {2, 3, 4}

# Set union operator
set_union_result: set[int] = set_a | set_b
assert 1 in set_union_result, "union should contain 1"
assert 2 in set_union_result, "union should contain 2"
assert 3 in set_union_result, "union should contain 3"
assert 4 in set_union_result, "union should contain 4"
assert len(set_union_result) == 4, "union should have 4 elements"

# Set intersection operator
set_inter_result: set[int] = set_a & set_b
assert 2 in set_inter_result, "intersection should contain 2"
assert 3 in set_inter_result, "intersection should contain 3"
assert 1 not in set_inter_result, "intersection should not contain 1"
assert len(set_inter_result) == 2, "intersection should have 2 elements"

# Set difference operator
set_diff_result: set[int] = set_a - set_b
assert 1 in set_diff_result, "difference should contain 1"
assert 2 not in set_diff_result, "difference should not contain 2"
assert len(set_diff_result) == 1, "difference should have 1 element"

# Set symmetric difference operator
set_symdiff_result: set[int] = set_a ^ set_b
assert 1 in set_symdiff_result, "sym_diff should contain 1"
assert 4 in set_symdiff_result, "sym_diff should contain 4"
assert 2 not in set_symdiff_result, "sym_diff should not contain 2"
assert len(set_symdiff_result) == 2, "sym_diff should have 2 elements"

# Set methods
set_union_m: set[int] = set_a.union(set_b)
assert len(set_union_m) == 4, "union method should have 4 elements"

set_inter_m: set[int] = set_a.intersection(set_b)
assert len(set_inter_m) == 2, "intersection method should have 2 elements"

set_diff_m: set[int] = set_a.difference(set_b)
assert len(set_diff_m) == 1, "difference method should have 1 element"

set_symdiff_m: set[int] = set_a.symmetric_difference(set_b)
assert len(set_symdiff_m) == 2, "symmetric_difference method should have 2 elements"

# Boolean set methods
assert set_a.issubset({1, 2, 3, 4}) == True, "issubset should be True"
assert set_a.issubset({1, 2}) == False, "issubset should be False"
assert set_a.issuperset({1, 2}) == True, "issuperset should be True"
assert set_a.issuperset({1, 2, 3, 4}) == False, "issuperset should be False"
assert set_a.isdisjoint({5, 6}) == True, "isdisjoint should be True"
assert set_a.isdisjoint({2, 5}) == False, "isdisjoint should be False"

print("Set operations and methods tests passed!")

# ===== SECTION: set.update() =====
set_upd: set[int] = {1, 2, 3}
set_upd.update({4, 5})
assert 4 in set_upd, "update should add 4"
assert 5 in set_upd, "update should add 5"
assert len(set_upd) == 5, "updated set should have 5 elements"

print("set.update() tests passed")

# ===== SECTION: set.intersection_update() =====
set_iu: set[int] = {1, 2, 3, 4}
set_iu.intersection_update({2, 3, 5})
assert 2 in set_iu, "intersection_update should keep 2"
assert 3 in set_iu, "intersection_update should keep 3"
assert 1 not in set_iu, "intersection_update should remove 1"
assert len(set_iu) == 2, "intersection_update result should have 2 elements"

print("set.intersection_update() tests passed")

# ===== SECTION: set.difference_update() =====
set_du: set[int] = {1, 2, 3, 4}
set_du.difference_update({2, 3, 5})
assert 1 in set_du, "difference_update should keep 1"
assert 4 in set_du, "difference_update should keep 4"
assert 2 not in set_du, "difference_update should remove 2"
assert len(set_du) == 2, "difference_update result should have 2 elements"

print("set.difference_update() tests passed")

# ===== SECTION: set.symmetric_difference_update() =====
set_sdu: set[int] = {1, 2, 3}
set_sdu.symmetric_difference_update({2, 3, 4})
assert 1 in set_sdu, "symmetric_difference_update should keep 1"
assert 4 in set_sdu, "symmetric_difference_update should add 4"
assert 2 not in set_sdu, "symmetric_difference_update should remove 2"
assert len(set_sdu) == 2, "symmetric_difference_update result should have 2 elements"

print("set.symmetric_difference_update() tests passed")

# TODO: tuple.index() and tuple.count() need raw int handling in rt_obj_eq

# ===== SECTION: dict.fromkeys() =====
dk_base: dict[str, int] = {}
dk_keys: list[str] = ["a", "b", "c"]
dk_result: dict[str, int] = dk_base.fromkeys(dk_keys, 0)
assert dk_result["a"] == 0, "fromkeys a should be 0"
assert dk_result["b"] == 0, "fromkeys b should be 0"
assert dk_result["c"] == 0, "fromkeys c should be 0"
assert len(dk_result) == 3, "fromkeys should have 3 entries"

print("dict.fromkeys() tests passed")

# ===== SECTION: dict | operator =====
dm_left: dict[str, int] = {"a": 1, "b": 2}
dm_right: dict[str, int] = {"b": 3, "c": 4}
dm_merged: dict[str, int] = dm_left | dm_right
assert dm_merged["a"] == 1, "merged dict a should be 1"
assert dm_merged["b"] == 3, "merged dict b should be 3 (right wins)"
assert dm_merged["c"] == 4, "merged dict c should be 4"
assert len(dm_merged) == 3, "merged dict should have 3 entries"

print("dict | operator tests passed")

# ===== SECTION: dict |= operator =====
dm_aug: dict[str, int] = {"a": 1, "b": 2}
dm_aug |= {"b": 3, "c": 4}
assert dm_aug["a"] == 1, "augmented dict a should be 1"
assert dm_aug["b"] == 3, "augmented dict b should be 3"
assert dm_aug["c"] == 4, "augmented dict c should be 4"
assert len(dm_aug) == 3, "augmented dict should have 3 entries"

# In-place alias semantics: |= must modify the dict, not create a new one
dm_alias_orig: dict[str, int] = {"x": 10}
dm_alias_ref: dict[str, int] = dm_alias_orig
dm_alias_orig |= {"y": 20}
assert len(dm_alias_ref) == 2, "alias should see in-place update"
assert dm_alias_ref["y"] == 20, "alias should have new key"

# Empty cases
dm_empty_rhs: dict[str, int] = {"a": 1}
dm_empty_rhs |= {}
assert len(dm_empty_rhs) == 1, "empty rhs should be no-op"

dm_empty_lhs: dict[str, int] = {}
dm_empty_lhs |= {"a": 1}
assert len(dm_empty_lhs) == 1, "empty lhs should get new entries"

print("dict |= operator tests passed")

# ===== SECTION: bytes.decode() =====
bd_test1: str = b"hello".decode()
assert bd_test1 == "hello", f"bytes decode should produce string, got {bd_test1}"

bd_test2: str = b"".decode()
assert bd_test2 == "", "empty bytes decode should produce empty string"

bd_test3: str = b"hello world".decode("utf-8")
assert bd_test3 == "hello world", "bytes decode utf-8 should work"

print("bytes.decode() tests passed")

# ===== SECTION: bytes.startswith() and bytes.endswith() =====
bse_data: bytes = b"hello world"
assert bse_data.startswith(b"hello") == True, "startswith hello should be True"
assert bse_data.startswith(b"world") == False, "startswith world should be False"
assert bse_data.endswith(b"world") == True, "endswith world should be True"
assert bse_data.endswith(b"hello") == False, "endswith hello should be False"
assert b"".startswith(b"") == True, "empty startswith empty should be True"
assert b"".endswith(b"") == True, "empty endswith empty should be True"

print("bytes.startswith/endswith() tests passed")

# ===== SECTION: bytes.find() and bytes.rfind() =====
bf_data: bytes = b"hello world hello"
bf_test1: int = bf_data.find(b"hello")
assert bf_test1 == 0, f"bytes.find should find first, got {bf_test1}"

bf_test2: int = bf_data.find(b"xyz")
assert bf_test2 == -1, "bytes.find missing should return -1"

bf_test3: int = bf_data.rfind(b"hello")
assert bf_test3 == 12, f"bytes.rfind should find last, got {bf_test3}"

bf_test4: int = bf_data.rfind(b"xyz")
assert bf_test4 == -1, "bytes.rfind missing should return -1"

print("bytes.find/rfind() tests passed")

# ===== SECTION: bytes.count() =====
bc_data: bytes = b"abcabcabc"
bc_test1: int = bc_data.count(b"abc")
assert bc_test1 == 3, f"bytes.count should be 3, got {bc_test1}"

bc_test2: int = bc_data.count(b"xyz")
assert bc_test2 == 0, "bytes.count missing should be 0"

print("bytes.count() tests passed")

# ===== SECTION: bytes.replace() =====
br_data: bytes = b"hello world"
br_test1: bytes = br_data.replace(b"world", b"python")
assert br_test1 == b"hello python", "bytes.replace should work"

br_test2: bytes = b"aaa".replace(b"a", b"bb")
assert br_test2 == b"bbbbbb", "bytes.replace all should work"

print("bytes.replace() tests passed")

# ===== SECTION: bytes.split() and bytes.rsplit() =====
bs_data: bytes = b"a,b,c"
bs_test1: list[bytes] = bs_data.split(b",")
assert len(bs_test1) == 3, f"bytes.split should produce 3 parts, got {len(bs_test1)}"
assert bs_test1[0] == b"a", "bytes.split first part"
assert bs_test1[1] == b"b", "bytes.split second part"
assert bs_test1[2] == b"c", "bytes.split third part"

bs_test2: list[bytes] = b"a,b,c,d".rsplit(b",", 2)
assert len(bs_test2) == 3, f"bytes.rsplit maxsplit should produce 3 parts, got {len(bs_test2)}"
assert bs_test2[0] == b"a,b", "bytes.rsplit first part"
assert bs_test2[1] == b"c", "bytes.rsplit second part"
assert bs_test2[2] == b"d", "bytes.rsplit third part"

print("bytes.split/rsplit() tests passed")

# ===== SECTION: bytes.strip/lstrip/rstrip() =====
bst_data: bytes = b"  hello  "
bst_test1: bytes = bst_data.strip()
assert bst_test1 == b"hello", "bytes.strip should remove whitespace"

bst_test2: bytes = bst_data.lstrip()
assert bst_test2 == b"hello  ", "bytes.lstrip should remove left whitespace"

bst_test3: bytes = bst_data.rstrip()
assert bst_test3 == b"  hello", "bytes.rstrip should remove right whitespace"

print("bytes.strip/lstrip/rstrip() tests passed")

# ===== SECTION: bytes.upper() and bytes.lower() =====
bul_data: bytes = b"Hello World"
bul_test1: bytes = bul_data.upper()
assert bul_test1 == b"HELLO WORLD", "bytes.upper should work"

bul_test2: bytes = bul_data.lower()
assert bul_test2 == b"hello world", "bytes.lower should work"

print("bytes.upper/lower() tests passed")

# ===== SECTION: bytes.join() =====
bj_sep: bytes = b","
bj_parts: list[bytes] = [b"a", b"b", b"c"]
bj_result: bytes = bj_sep.join(bj_parts)
assert bj_result == b"a,b,c", "bytes.join should work"

bj_empty: bytes = b"".join([b"x", b"y"])
assert bj_empty == b"xy", "bytes.join with empty sep should concat"

print("bytes.join() tests passed")

# ===== SECTION: bytes concatenation and repetition =====
bcat_a: bytes = b"hello"
bcat_b: bytes = b" world"
bcat_result: bytes = bcat_a + bcat_b
assert bcat_result == b"hello world", "bytes concat should work"

brep_data: bytes = b"ab"
brep_result: bytes = brep_data * 3
assert brep_result == b"ababab", "bytes repeat should work"

print("bytes concat/repeat tests passed")

# ===== SECTION: Dict with float keys =====
float_dict: dict[float, str] = {1.5: "a", 2.5: "b", 3.5: "c"}
assert len(float_dict) == 3, "float dict length should be 3"
assert float_dict[1.5] == "a", "float dict lookup 1.5"
assert float_dict[2.5] == "b", "float dict lookup 2.5"
assert float_dict[3.5] == "c", "float dict lookup 3.5"

# Update existing float key
float_dict[1.5] = "updated"
assert float_dict[1.5] == "updated", "float dict update"
assert len(float_dict) == 3, "float dict length after update should still be 3"

# Add new float key
float_dict[4.5] = "d"
assert len(float_dict) == 4, "float dict length after add"

# in operator
assert 1.5 in float_dict, "1.5 should be in float_dict"
assert 2.5 in float_dict, "2.5 should be in float_dict"
assert 9.9 not in float_dict, "9.9 should not be in float_dict"

# Iterate over float dict keys
float_key_sum: float = 0.0
for fk in float_dict:
    float_key_sum = float_key_sum + fk
assert float_key_sum == 12.0, "float dict key sum should be 12.0"

print("dict with float keys tests passed")

# ===== SECTION: Dict with None key =====
none_dict: dict[None, str] = {None: "null_value"}
assert len(none_dict) == 1, "none dict length should be 1"
assert none_dict[None] == "null_value", "none dict lookup None"
assert None in none_dict, "None should be in none_dict"

# Update None key
none_dict[None] = "updated_null"
assert none_dict[None] == "updated_null", "none dict update"
assert len(none_dict) == 1, "none dict length after update should still be 1"

print("dict with None key tests passed")

# ===== SECTION: Dict with tuple keys =====
tuple_dict: dict[tuple[int, int], str] = {(1, 2): "pair_a", (3, 4): "pair_b"}
assert len(tuple_dict) == 2, "tuple dict length should be 2"
assert tuple_dict[(1, 2)] == "pair_a", "tuple dict lookup (1,2)"
assert tuple_dict[(3, 4)] == "pair_b", "tuple dict lookup (3,4)"

# in operator
assert (1, 2) in tuple_dict, "(1,2) should be in tuple_dict"
assert (3, 4) in tuple_dict, "(3,4) should be in tuple_dict"
assert (5, 6) not in tuple_dict, "(5,6) should not be in tuple_dict"

# Update existing tuple key
tuple_dict[(1, 2)] = "updated_pair"
assert tuple_dict[(1, 2)] == "updated_pair", "tuple dict update"
assert len(tuple_dict) == 2, "tuple dict length after update should still be 2"

# Add new tuple key
tuple_dict[(5, 6)] = "pair_c"
assert len(tuple_dict) == 3, "tuple dict length after add"

print("dict with tuple keys tests passed")

# ===== SECTION: Set with float elements =====
float_set: set[float] = {1.1, 2.2, 3.3}
assert len(float_set) == 3, "float set length should be 3"
assert 1.1 in float_set, "1.1 should be in float_set"
assert 2.2 in float_set, "2.2 should be in float_set"
assert 3.3 in float_set, "3.3 should be in float_set"
assert 4.4 not in float_set, "4.4 should not be in float_set"

# add
float_set.add(4.4)
assert 4.4 in float_set, "4.4 should be in float_set after add"
assert len(float_set) == 4, "float set length after add"

# discard
float_set.discard(2.2)
assert 2.2 not in float_set, "2.2 should not be in float_set after discard"
assert len(float_set) == 3, "float set length after discard"

# Iterate over float set
float_elem_count: int = 0
for fe in float_set:
    float_elem_count = float_elem_count + 1
assert float_elem_count == 3, "float set iteration count"

print("set with float elements tests passed")

# ===== SECTION: Set with None element =====
none_set: set[None] = {None}
assert len(none_set) == 1, "none set length should be 1"
assert None in none_set, "None should be in none_set"

# Adding None again should not increase length
none_set.add(None)
assert len(none_set) == 1, "none set length after duplicate add"

# discard
none_set.discard(None)
assert None not in none_set, "None should not be in none_set after discard"
assert len(none_set) == 0, "none set length after discard"

print("set with None element tests passed")

# ===== SECTION: Set with tuple elements =====
tuple_set: set[tuple[int, int]] = {(1, 2), (3, 4)}
assert len(tuple_set) == 2, "tuple set length should be 2"
assert (1, 2) in tuple_set, "(1,2) should be in tuple_set"
assert (3, 4) in tuple_set, "(3,4) should be in tuple_set"
assert (5, 6) not in tuple_set, "(5,6) should not be in tuple_set"

# add
tuple_set.add((5, 6))
assert (5, 6) in tuple_set, "(5,6) should be in tuple_set after add"
assert len(tuple_set) == 3, "tuple set length after add"

# Adding duplicate should not increase length
tuple_set.add((1, 2))
assert len(tuple_set) == 3, "tuple set length after duplicate add"

print("set with tuple elements tests passed")

# ===== SECTION: Dict insertion order preservation =====

# Dicts preserve insertion order (Python 3.7+ guarantee)
order_dict: dict[str, int] = {}
order_dict["c"] = 3
order_dict["a"] = 1
order_dict["b"] = 2
order_keys: list[str] = list(order_dict.keys())
assert order_keys[0] == "c", "first key should be c"
assert order_keys[1] == "a", "second key should be a"
assert order_keys[2] == "b", "third key should be b"

order_vals: list[int] = list(order_dict.values())
assert order_vals[0] == 3, "first value should be 3"
assert order_vals[1] == 1, "second value should be 1"
assert order_vals[2] == 2, "third value should be 2"

# Updating existing key preserves its position
order_dict["a"] = 99
order_keys2: list[str] = list(order_dict.keys())
assert order_keys2[0] == "c", "after update, first key still c"
assert order_keys2[1] == "a", "after update, second key still a"
assert order_keys2[2] == "b", "after update, third key still b"
order_vals2: list[int] = list(order_dict.values())
assert order_vals2[1] == 99, "updated value should be 99"

# Deleting and re-inserting moves key to end
del order_dict["a"]
order_dict["a"] = 50
order_keys3: list[str] = list(order_dict.keys())
assert order_keys3[0] == "c", "after re-insert, first key is c"
assert order_keys3[1] == "b", "after re-insert, second key is b"
assert order_keys3[2] == "a", "after re-insert, third key is a (moved to end)"

# Integer keys also preserve order
int_order_dict: dict[int, str] = {}
int_order_dict[5] = "five"
int_order_dict[1] = "one"
int_order_dict[3] = "three"
int_order_vals: list[str] = list(int_order_dict.values())
assert int_order_vals[0] == "five", "int dict first value"
assert int_order_vals[1] == "one", "int dict second value"
assert int_order_vals[2] == "three", "int dict third value"

print("dict insertion order tests passed")

# ===== SECTION: sorted(set(...)) =====

# sorted() on a set returns a sorted list
sorted_set_result: list[int] = sorted(set([3, 1, 4, 1, 5, 9, 2, 6]))
assert sorted_set_result[0] == 1, "sorted set first element"
assert sorted_set_result[1] == 2, "sorted set second element"
assert sorted_set_result[2] == 3, "sorted set third element"
assert sorted_set_result[3] == 4, "sorted set fourth element"
assert sorted_set_result[4] == 5, "sorted set fifth element"
assert sorted_set_result[5] == 6, "sorted set sixth element"
assert sorted_set_result[6] == 9, "sorted set seventh element"
assert len(sorted_set_result) == 7, "sorted set length (duplicates removed)"

# sorted() on set of strings
str_set: set[str] = set(["banana", "apple", "cherry"])
sorted_str_set: list[str] = sorted(str_set)
assert sorted_str_set[0] == "apple", "sorted string set first"
assert sorted_str_set[1] == "banana", "sorted string set second"
assert sorted_str_set[2] == "cherry", "sorted string set third"

print("sorted(set()) tests passed")

print("All dict, set, and bytes tests passed!")
