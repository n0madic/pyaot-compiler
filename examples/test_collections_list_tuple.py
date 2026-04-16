# Consolidated test file for list and tuple collections

# ===== SECTION: List creation, indexing, len =====

# List creation and indexing
nums: list[int] = [1, 2, 3, 4, 5]
assert nums[0] == 1, "nums[0] should equal 1"
assert nums[2] == 3, "nums[2] should equal 3"
assert nums[4] == 5, "nums[4] should equal 5"
assert nums[-1] == 5, "nums[-1] should equal 5"  # negative indexing
assert nums[-2] == 4, "nums[-2] should equal 4"
assert len(nums) == 5, "len(nums) should equal 5"

# Empty list
empty_list: list[int] = []
assert len(empty_list) == 0, "len(empty_list) should equal 0"

# Single element list
single_list: list[int] = [42]
assert single_list[0] == 42, "single_list[0] should equal 42"
assert len(single_list) == 1, "len(single_list) should equal 1"

# Larger list
big_list: list[int] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
assert big_list[0] == 10, "big_list[0] should equal 10"
assert big_list[5] == 60, "big_list[5] should equal 60"
assert big_list[9] == 100, "big_list[9] should equal 100"
assert big_list[-1] == 100, "big_list[-1] should equal 100"
assert len(big_list) == 10, "len(big_list) should equal 10"

# ===== SECTION: List slicing (positive, negative, step) =====

# Basic slicing
slice_nums: list[int] = [0, 1, 2, 3, 4, 5]
slice1: list[int] = slice_nums[1:4]
assert len(slice1) == 3, "len(slice1) should equal 3"
assert slice1[0] == 1, "slice1[0] should equal 1"
assert slice1[1] == 2, "slice1[1] should equal 2"
assert slice1[2] == 3, "slice1[2] should equal 3"
print("Basic slice [1:4] passed")

# Slice from start
slice2: list[int] = slice_nums[:3]
assert len(slice2) == 3, "len(slice2) should equal 3"
assert slice2[0] == 0, "slice2[0] should equal 0"
assert slice2[1] == 1, "slice2[1] should equal 1"
assert slice2[2] == 2, "slice2[2] should equal 2"
print("Slice from start [:3] passed")

# Slice to end
slice3: list[int] = slice_nums[3:]
assert len(slice3) == 3, "len(slice3) should equal 3"
assert slice3[0] == 3, "slice3[0] should equal 3"
assert slice3[1] == 4, "slice3[1] should equal 4"
assert slice3[2] == 5, "slice3[2] should equal 5"
print("Slice to end [3:] passed")

# Slice with step
evens: list[int] = slice_nums[::2]
assert len(evens) == 3, "len(evens) should equal 3"
assert evens[0] == 0, "evens[0] should equal 0"
assert evens[1] == 2, "evens[1] should equal 2"
assert evens[2] == 4, "evens[2] should equal 4"
print("Slice with step [::2] passed")

# Slice with step=3
every_third: list[int] = slice_nums[::3]
assert len(every_third) == 2, "len(every_third) should equal 2"
assert every_third[0] == 0, "every_third[0] should equal 0"
assert every_third[1] == 3, "every_third[1] should equal 3"
print("Slice with step [::3] passed")

# Negative indices
last_two: list[int] = slice_nums[-2:]
assert len(last_two) == 2, "len(last_two) should equal 2"
assert last_two[0] == 4, "last_two[0] should equal 4"
assert last_two[1] == 5, "last_two[1] should equal 5"
print("Negative slice [-2:] passed")

# Negative end index (exclude last element)
without_last: list[int] = slice_nums[:-1]
assert len(without_last) == 5, "len(without_last) should equal 5"
assert without_last == [0, 1, 2, 3, 4], "slice [:-1] failed"
print("Negative end slice [:-1] passed")

# Slice with negative start and end
middle_neg: list[int] = slice_nums[-4:-1]
assert middle_neg == [2, 3, 4], "slice [-4:-1] failed"
print("Negative range slice [-4:-1] passed")

# Empty slice
empty_slice: list[int] = slice_nums[3:3]
assert len(empty_slice) == 0, "len(empty_slice) should equal 0"
print("Empty slice [3:3] passed")

# ===== SECTION: List methods (append, extend, pop, insert, clear, copy, reverse, index, count) =====

# List append
items: list[int] = [1, 2]
items.append(3)
assert len(items) == 3, "len(items) should equal 3"
assert items[2] == 3, "items[2] should equal 3"
items.append(4)
assert len(items) == 4, "len(items) should equal 4"
assert items[3] == 4, "items[3] should equal 4"
print("append() passed")

# List extend
extend_list: list[int] = [1, 2, 3]
extend_list.extend([4, 5, 6])
assert len(extend_list) == 6, "len(extend_list) should equal 6"
assert extend_list == [1, 2, 3, 4, 5, 6], "extend() failed"
print("extend() passed")

# List pop
values: list[int] = [10, 20, 30, 40]
last: int = values.pop()
assert last == 40, "last should equal 40"
assert len(values) == 3, "len(values) should equal 3"

first: int = values.pop(0)
assert first == 10, "first should equal 10"
assert len(values) == 2, "len(values) should equal 2"
assert values[0] == 20, "values[0] should equal 20"
print("pop() passed")

# List insert
data: list[int] = [1, 3, 4]
data.insert(1, 2)
assert len(data) == 4, "len(data) should equal 4"
assert data[0] == 1, "data[0] should equal 1"
assert data[1] == 2, "data[1] should equal 2"
assert data[2] == 3, "data[2] should equal 3"
assert data[3] == 4, "data[3] should equal 4"
print("insert() passed")

# List clear
to_clear: list[int] = [1, 2, 3, 4, 5]
to_clear.clear()
assert len(to_clear) == 0, "len(to_clear) should equal 0"
print("clear() passed")

# List copy
original: list[int] = [1, 2, 3]
copied: list[int] = original.copy()
assert len(copied) == 3, "len(copied) should equal 3"
assert copied[0] == 1, "copied[0] should equal 1"
assert copied[1] == 2, "copied[1] should equal 2"
assert copied[2] == 3, "copied[2] should equal 3"

# Modify copy - original should be unchanged
copied.append(4)
assert len(copied) == 4, "len(copied) should equal 4"
assert len(original) == 3, "len(original) should equal 3"
print("copy() passed")

# List reverse
nums2: list[int] = [1, 2, 3, 4, 5]
nums2.reverse()
assert len(nums2) == 5, "len(nums2) should equal 5"
assert nums2[0] == 5, "nums2[0] should equal 5"
assert nums2[1] == 4, "nums2[1] should equal 4"
assert nums2[2] == 3, "nums2[2] should equal 3"
assert nums2[3] == 2, "nums2[3] should equal 2"
assert nums2[4] == 1, "nums2[4] should equal 1"
print("reverse() passed")

# Single element reverse
single_rev: list[int] = [42]
single_rev.reverse()
assert len(single_rev) == 1, "len(single_rev) should equal 1"
assert single_rev[0] == 42, "single_rev[0] should equal 42"
print("reverse() single element passed")

# Combined operations: build list incrementally
build: list[int] = []
build.append(1)
build.append(2)
build.append(3)
assert len(build) == 3, "len(build) should equal 3"

# Slice and copy
subset: list[int] = build[1:].copy()
assert len(subset) == 2, "len(subset) should equal 2"
assert subset[0] == 2, "subset[0] should equal 2"
assert subset[1] == 3, "subset[1] should equal 3"

# Reverse the subset
subset.reverse()
assert subset[0] == 3, "subset[0] should equal 3"
assert subset[1] == 2, "subset[1] should equal 2"
print("Combined operations passed")

# list.index()
idx_list: list[int] = [10, 20, 30, 20, 40]
assert idx_list.index(10) == 0, "idx_list.index(10) should equal 0"
assert idx_list.index(20) == 1, "idx_list.index(20) should equal 1"  # first occurrence
assert idx_list.index(40) == 4, "idx_list.index(40) should equal 4"
print("list.index() passed")

# list.count()
count_list: list[int] = [1, 2, 2, 3, 2, 4]
assert count_list.count(2) == 3, "count_list.count(2) should equal 3"
assert count_list.count(1) == 1, "count_list.count(1) should equal 1"
assert count_list.count(99) == 0, "count_list.count(99) should equal 0"
print("list.count() passed")

# list.index() with string values (value equality, not pointer equality)
str_idx_list: list[str] = ["hello", "world", "foo", "bar"]
assert str_idx_list.index("hello") == 0, "str list.index(hello) should be 0"
assert str_idx_list.index("world") == 1, "str list.index(world) should be 1"
assert str_idx_list.index("foo") == 2, "str list.index(foo) should be 2"
assert str_idx_list.index("bar") == 3, "str list.index(bar) should be 3"

# list.index() on sorted(set()) result — dynamically created strings
joined_str: str = "zyxwvu"
sorted_chars: list[str] = sorted(set(joined_str))
assert sorted_chars.index("u") == 0, "sorted chars index of u"
assert sorted_chars.index("v") == 1, "sorted chars index of v"
assert sorted_chars.index("w") == 2, "sorted chars index of w"
assert sorted_chars.index("x") == 3, "sorted chars index of x"
assert sorted_chars.index("y") == 4, "sorted chars index of y"
assert sorted_chars.index("z") == 5, "sorted chars index of z"

# list.count() with string values
str_count_list: list[str] = ["a", "b", "a", "c", "a"]
assert str_count_list.count("a") == 3, "str list.count(a) should be 3"
assert str_count_list.count("b") == 1, "str list.count(b) should be 1"
assert str_count_list.count("x") == 0, "str list.count(x) should be 0"

print("list.index()/count() with strings passed")

# ===== SECTION: List equality and min/max =====

list_a: list[int] = [1, 2, 3]
list_b: list[int] = [1, 2, 3]
list_c: list[int] = [1, 2, 4]
assert list_a == list_b, "equal lists should be equal"
assert list_a != list_c, "different lists should not be equal"

# Empty list equality
empty_a: list[int] = []
empty_b: list[int] = []
assert empty_a == empty_b, "empty lists should be equal"

# String list equality
str_list_a: list[str] = ["hello", "world"]
str_list_b: list[str] = ["hello", "world"]
assert str_list_a == str_list_b, "string lists should be equal"

# ===== SECTION: Cross-elem-tag list equality =====
# Same-tag regression guard — starred-rest on homogeneous source produces
# List(Int) / ELEM_RAW_INT, matches the literal's storage.
cx_a, *cx_rest = (1, 2, 3, 4)
assert cx_rest == [2, 3, 4], "same-tag cross-rest equality"
assert [2, 3, 4] == cx_rest, "same-tag cross-rest equality (symmetric)"

# Cross-tag — heterogeneous tuple source produces List(Any) / ELEM_HEAP_OBJ
# starred slice (boxed pointers). Literal [2, 3] is ELEM_RAW_INT.
# rt_list_eq must dispatch per element.
(cx_b, *cx_rest2, cx_last) = (1, 2, 3, (4, 5))
assert cx_rest2 == [2, 3], "cross-tag list eq: heap-boxed vs raw-int"
assert [2, 3] == cx_rest2, "cross-tag list eq (symmetric)"

# Heterogeneous source with float — exercises the float branch of list_elem_eq.
(cx_c, *cx_rest3, cx_last3) = (1, 2.5, 3, (4, 5))
assert cx_rest3 == [2.5, 3], "cross-tag list eq with float"

# Bool in heterogeneous source — exercises bool branch.
(cx_d, *cx_rest4, cx_last4) = (1, True, False, (4, 5))
assert cx_rest4 == [True, False], "cross-tag list eq with bool"

# ===== SECTION: List ordering comparisons (lexicographic) =====

# Basic list[int] ordering
assert [1, 2, 3] < [1, 2, 4], "[1,2,3] < [1,2,4] failed"
assert [1, 2, 4] > [1, 2, 3], "[1,2,4] > [1,2,3] failed"
assert [1, 2, 3] <= [1, 2, 4], "[1,2,3] <= [1,2,4] failed"
assert [1, 2, 4] >= [1, 2, 3], "[1,2,4] >= [1,2,3] failed"

# Equal lists with <= and >=
ord_eq_a: list[int] = [1, 2, 3]
ord_eq_b: list[int] = [1, 2, 3]
assert ord_eq_a <= ord_eq_b, "equal lists should be <="
assert ord_eq_a >= ord_eq_b, "equal lists should be >="
assert not (ord_eq_a < ord_eq_b), "equal lists should not be <"
assert not (ord_eq_a > ord_eq_b), "equal lists should not be >"

# Different lengths (prefix is less)
assert [1, 2] < [1, 2, 3], "shorter prefix should be less"
assert [1, 2, 3] > [1, 2], "longer list should be greater than prefix"
assert [1, 2] <= [1, 2, 3], "shorter prefix should be <="
assert [1, 2, 3] >= [1, 2], "longer list should be >= prefix"

# Empty lists
ord_empty_a: list[int] = []
ord_empty_b: list[int] = []
assert ord_empty_a <= ord_empty_b, "empty lists should be <="
assert ord_empty_a >= ord_empty_b, "empty lists should be >="
assert not (ord_empty_a < ord_empty_b), "empty lists should not be <"
assert not (ord_empty_a > ord_empty_b), "empty lists should not be >"
ord_nonempty: list[int] = [1]
assert ord_empty_a < ord_nonempty, "empty list should be < non-empty"
assert ord_nonempty > ord_empty_a, "non-empty list should be > empty"

# list[float] ordering
assert [1.0, 2.0] < [1.0, 3.0], "float list ordering failed"
assert [1.5, 2.5] > [1.5, 2.0], "float list ordering failed (gt)"
assert [1.0, 2.0] <= [1.0, 2.0], "float list <= failed"
assert [1.0, 2.0] >= [1.0, 2.0], "float list >= failed"

# list[str] ordering
assert ["a", "b"] < ["a", "c"], "str list ordering failed"
assert ["b", "a"] > ["a", "z"], "str list ordering failed (gt)"
assert ["hello"] < ["hello", "world"], "str list prefix ordering failed"
assert ["a", "b"] <= ["a", "b"], "str list <= failed"
assert ["a", "b"] >= ["a", "b"], "str list >= failed"

print("list ordering comparisons passed")

# min() and max() with list argument
numbers_for_minmax: list[int] = [10, 20, 5, 40, 15]
assert min(numbers_for_minmax) == 5, "min(list) failed"
assert max(numbers_for_minmax) == 40, "max(list) failed"

# Float arrays
floats: list[float] = [1.5, 2.7, 3.14, 4.0, 5.5]
assert floats == [1.5, 2.7, 3.14, 4.0, 5.5], "float array failed"
assert min(floats) == 1.5, "float min failed"
assert max(floats) == 5.5, "float max failed"
assert len(floats) == 5, "len(floats) should equal 5"

# List modification (element assignment)
fruits: list[str] = ["apple", "banana", "cherry"]
assert fruits == ["apple", "banana", "cherry"], "initial fruits failed"
fruits[1] = "blueberry"
assert fruits == ["apple", "blueberry", "cherry"], "modified fruits failed"

numbers_mod: list[int] = [1, 2, 3, 4, 5]
assert numbers_mod == [1, 2, 3, 4, 5], "initial numbers failed"
numbers_mod[0] = 10
assert numbers_mod == [10, 2, 3, 4, 5], "modified numbers failed"

# ===== SECTION: Tuple creation, indexing, unpacking =====

# Tuple creation and indexing
point: tuple[int, int] = (10, 20)
assert point[0] == 10, "point[0] should equal 10"
assert point[1] == 20, "point[1] should equal 20"
assert point[-1] == 20, "point[-1] should equal 20"  # negative indexing
assert point[-2] == 10, "point[-2] should equal 10"
assert len(point) == 2, "len(point) should equal 2"

# Single element tuple
single_tuple: tuple[int] = (99,)
assert single_tuple[0] == 99, "single_tuple[0] should equal 99"
assert len(single_tuple) == 1, "len(single_tuple) should equal 1"

# Multi-element tuple
triple: tuple[int, int, int] = (1, 2, 3)
assert triple[0] == 1, "triple[0] should equal 1"
assert triple[1] == 2, "triple[1] should equal 2"
assert triple[2] == 3, "triple[2] should equal 3"
assert len(triple) == 3, "len(triple) should equal 3"

# ===== SECTION: Tuple slicing =====

# Create a tuple
tuple_nums: tuple[int, int, int, int, int, int] = (0, 1, 2, 3, 4, 5)

# Basic slicing
tuple_slice1: tuple[int, int, int, int, int, int] = tuple_nums[1:4]
assert len(tuple_slice1) == 3, "len(tuple_slice1) should equal 3"
assert tuple_slice1[0] == 1, "tuple_slice1[0] should equal 1"
assert tuple_slice1[1] == 2, "tuple_slice1[1] should equal 2"
assert tuple_slice1[2] == 3, "tuple_slice1[2] should equal 3"
print("Tuple basic slice [1:4] passed")

# Slice from start
tuple_slice2: tuple[int, int, int, int, int, int] = tuple_nums[:3]
assert len(tuple_slice2) == 3, "len(tuple_slice2) should equal 3"
assert tuple_slice2[0] == 0, "tuple_slice2[0] should equal 0"
assert tuple_slice2[1] == 1, "tuple_slice2[1] should equal 1"
assert tuple_slice2[2] == 2, "tuple_slice2[2] should equal 2"
print("Tuple slice from start [:3] passed")

# Slice to end
tuple_slice3: tuple[int, int, int, int, int, int] = tuple_nums[3:]
assert len(tuple_slice3) == 3, "len(tuple_slice3) should equal 3"
assert tuple_slice3[0] == 3, "tuple_slice3[0] should equal 3"
assert tuple_slice3[1] == 4, "tuple_slice3[1] should equal 4"
assert tuple_slice3[2] == 5, "tuple_slice3[2] should equal 5"
print("Tuple slice to end [3:] passed")

# Full slice (copy)
full: tuple[int, int, int, int, int, int] = tuple_nums[:]
assert len(full) == 6, "len(full) should equal 6"
assert full[0] == 0, "full[0] should equal 0"
assert full[5] == 5, "full[5] should equal 5"
print("Tuple full slice [:] passed")

# Slice with step
tuple_evens: tuple[int, int, int, int, int, int] = tuple_nums[::2]
assert len(tuple_evens) == 3, "len(tuple_evens) should equal 3"
assert tuple_evens[0] == 0, "tuple_evens[0] should equal 0"
assert tuple_evens[1] == 2, "tuple_evens[1] should equal 2"
assert tuple_evens[2] == 4, "tuple_evens[2] should equal 4"
print("Tuple slice with step [::2] passed")

# Every third element
tuple_every_third: tuple[int, int, int, int, int, int] = tuple_nums[::3]
assert len(tuple_every_third) == 2, "len(tuple_every_third) should equal 2"
assert tuple_every_third[0] == 0, "tuple_every_third[0] should equal 0"
assert tuple_every_third[1] == 3, "tuple_every_third[1] should equal 3"
print("Tuple slice with step [::3] passed")

# Step with start and end
stepped: tuple[int, int, int, int, int, int] = tuple_nums[1:5:2]
assert len(stepped) == 2, "len(stepped) should equal 2"
assert stepped[0] == 1, "stepped[0] should equal 1"
assert stepped[1] == 3, "stepped[1] should equal 3"
print("Tuple slice with start, end, step [1:5:2] passed")

# Negative indices
tuple_last_two: tuple[int, int, int, int, int, int] = tuple_nums[-2:]
assert len(tuple_last_two) == 2, "len(tuple_last_two) should equal 2"
assert tuple_last_two[0] == 4, "tuple_last_two[0] should equal 4"
assert tuple_last_two[1] == 5, "tuple_last_two[1] should equal 5"
print("Tuple negative slice [-2:] passed")

# All but last element
all_but_last: tuple[int, int, int, int, int, int] = tuple_nums[:-1]
assert len(all_but_last) == 5, "len(all_but_last) should equal 5"
assert all_but_last[0] == 0, "all_but_last[0] should equal 0"
assert all_but_last[4] == 4, "all_but_last[4] should equal 4"
print("Tuple negative slice [:-1] passed")

# Reverse entire tuple
reversed_tuple: tuple[int, int, int, int, int, int] = tuple_nums[::-1]
assert len(reversed_tuple) == 6, "len(reversed_tuple) should equal 6"
assert reversed_tuple[0] == 5, "reversed_tuple[0] should equal 5"
assert reversed_tuple[1] == 4, "reversed_tuple[1] should equal 4"
assert reversed_tuple[2] == 3, "reversed_tuple[2] should equal 3"
assert reversed_tuple[3] == 2, "reversed_tuple[3] should equal 2"
assert reversed_tuple[4] == 1, "reversed_tuple[4] should equal 1"
assert reversed_tuple[5] == 0, "reversed_tuple[5] should equal 0"
print("Tuple reverse slice [::-1] passed")

# Reverse with step -2
reverse_evens: tuple[int, int, int, int, int, int] = tuple_nums[::-2]
assert len(reverse_evens) == 3, "len(reverse_evens) should equal 3"
assert reverse_evens[0] == 5, "reverse_evens[0] should equal 5"
assert reverse_evens[1] == 3, "reverse_evens[1] should equal 3"
assert reverse_evens[2] == 1, "reverse_evens[2] should equal 1"
print("Tuple reverse slice [::-2] passed")

# Edge cases: empty slice
tuple_empty_slice: tuple[int, int, int, int, int, int] = tuple_nums[3:3]
assert len(tuple_empty_slice) == 0, "len(tuple_empty_slice) should equal 0"
print("Tuple empty slice [3:3] passed")

# Empty range (start > end with positive step)
empty_range: tuple[int, int, int, int, int, int] = tuple_nums[4:2]
assert len(empty_range) == 0, "len(empty_range) should equal 0"
print("Tuple empty range [4:2] passed")

# Single element tuple slicing
single_t: tuple[int] = (42,)
single_t_slice: tuple[int] = single_t[:]
assert len(single_t_slice) == 1, "len(single_t_slice) should equal 1"
assert single_t_slice[0] == 42, "single_t_slice[0] should equal 42"
print("Tuple single element slice passed")

# Out of bounds: end beyond length
beyond_end: tuple[int, int, int, int, int, int] = tuple_nums[3:100]
assert len(beyond_end) == 3, "len(beyond_end) should equal 3"
assert beyond_end[0] == 3, "beyond_end[0] should equal 3"
assert beyond_end[1] == 4, "beyond_end[1] should equal 4"
assert beyond_end[2] == 5, "beyond_end[2] should equal 5"
print("Tuple beyond end [3:100] passed")

# Start beyond length
start_beyond: tuple[int, int, int, int, int, int] = tuple_nums[100:]
assert len(start_beyond) == 0, "len(start_beyond) should equal 0"
print("Tuple start beyond [100:] passed")

# ===== SECTION: List sort method =====

# Basic sort
sort_nums: list[int] = [3, 1, 4, 1, 5, 9, 2, 6]
sort_nums.sort()
assert sort_nums == [1, 1, 2, 3, 4, 5, 6, 9], "list.sort() basic failed"

# Sort already sorted
sorted_nums: list[int] = [1, 2, 3, 4, 5]
sorted_nums.sort()
assert sorted_nums == [1, 2, 3, 4, 5], "list.sort() already sorted failed"

# Sort reverse sorted
reverse_nums: list[int] = [5, 4, 3, 2, 1]
reverse_nums.sort()
assert reverse_nums == [1, 2, 3, 4, 5], "list.sort() reverse sorted failed"

# Sort with reverse=True keyword argument (CPython-compatible)
rev_sort_nums: list[int] = [3, 1, 4, 1, 5]
rev_sort_nums.sort(reverse=True)
assert rev_sort_nums == [5, 4, 3, 1, 1], "list.sort(reverse=True) failed"

# Sort single element
single_sort: list[int] = [42]
single_sort.sort()
assert single_sort == [42], "list.sort() single element failed"

# Sort empty list
empty_sort: list[int] = []
empty_sort.sort()
assert empty_sort == [], "list.sort() empty failed"

# Sort with negative numbers
neg_nums: list[int] = [-3, 1, -4, 1, 5]
neg_nums.sort()
assert neg_nums == [-4, -3, 1, 1, 5], "list.sort() with negatives failed"

# Sort strings
sort_strs: list[str] = ["banana", "apple", "cherry"]
sort_strs.sort()
assert sort_strs == ["apple", "banana", "cherry"], "list.sort() strings failed"

# Sort with key function (sort by length)
def str_len(s: str) -> int:
    return len(s)

strs_by_len: list[str] = ["apple", "fig", "banana", "kiwi"]
strs_by_len.sort(key=str_len)
assert strs_by_len == ["fig", "kiwi", "apple", "banana"], "list.sort(key=) failed"

# Sort with key and reverse (longest first)
strs_by_len2: list[str] = ["apple", "fig", "banana", "kiwi"]
strs_by_len2.sort(key=str_len, reverse=True)
assert strs_by_len2 == ["banana", "apple", "kiwi", "fig"], "list.sort(key=, reverse=True) failed"

# Sort with key=None (default behavior)
nums_key_none: list[int] = [3, 1, 4, 1, 5]
nums_key_none.sort(key=None)
assert nums_key_none == [1, 1, 3, 4, 5], "list.sort(key=None) failed"

# Sort with lambda key
nums_abs: list[int] = [-5, 2, -3, 1, -4]
def abs_val(x: int) -> int:
    if x < 0:
        return -x
    return x
nums_abs.sort(key=abs_val)
assert nums_abs == [1, 2, -3, -4, -5], "list.sort(key=abs_val) failed"

# Sort with key and reverse=False (explicit)
sort_explicit: list[str] = ["cherry", "apple", "banana"]
sort_explicit.sort(key=str_len, reverse=False)
assert sort_explicit == ["apple", "cherry", "banana"], "list.sort(key=, reverse=False) failed"

print("list.sort() tests passed")

# ===== SECTION: Container Printing =====

# List printing - integers
print_int_list: list[int] = [1, 2, 3]
print(print_int_list)

# List printing - strings
print_str_list: list[str] = ["a", "b"]
print(print_str_list)

# Empty list
print_empty_list: list[int] = []
print(print_empty_list)

# Nested lists (list of lists of ints)
print_nested_list: list[list[int]] = [[1, 2], [3, 4]]
print(print_nested_list)

# Mixed-type list (int and nested list)
print_mixed_list = [1, 2, ["string"]]
print(print_mixed_list)

# Tuple printing
print_tuple: tuple[int, int, int] = (1, 2, 3)
print(print_tuple)

# Single-element tuple
print_single_tuple: tuple[int] = (42,)
print(print_single_tuple)

# String tuple
print_str_tuple: tuple[str, str] = ("hello", "world")
print(print_str_tuple)

# ===== SECTION: Tuple equality comparison =====

# Basic tuple equality (integer tuples)
tuple_eq_a: tuple[int, int, int] = (1, 2, 3)
tuple_eq_b: tuple[int, int, int] = (1, 2, 3)
tuple_eq_c: tuple[int, int, int] = (1, 2, 4)
assert tuple_eq_a == tuple_eq_b, "equal int tuples should be equal"
assert tuple_eq_a != tuple_eq_c, "different int tuples should not be equal"
print("Basic tuple equality passed")

# Empty tuple equality
empty_tuple_a: tuple[()] = ()
empty_tuple_b: tuple[()] = ()
assert empty_tuple_a == empty_tuple_b, "empty tuples should be equal"
print("Empty tuple equality passed")

# String tuple equality
str_tuple_a: tuple[str, str, str] = ("a", "b", "c")
str_tuple_b: tuple[str, str, str] = ("a", "b", "c")
str_tuple_c: tuple[str, str, str] = ("a", "b", "d")
assert str_tuple_a == str_tuple_b, "equal string tuples should be equal"
assert str_tuple_a != str_tuple_c, "different string tuples should not be equal"
print("String tuple equality passed")

# Mixed type tuple equality
mixed_tuple_a: tuple[int, str, int] = (1, "hello", 3)
mixed_tuple_b: tuple[int, str, int] = (1, "hello", 3)
mixed_tuple_c: tuple[int, str, int] = (1, "world", 3)
assert mixed_tuple_a == mixed_tuple_b, "equal mixed tuples should be equal"
assert mixed_tuple_a != mixed_tuple_c, "different mixed tuples should not be equal"
print("Mixed tuple equality passed")

# Nested tuple equality
nested_tuple_a: tuple[tuple[int, int], tuple[int, int]] = ((1, 2), (3, 4))
nested_tuple_b: tuple[tuple[int, int], tuple[int, int]] = ((1, 2), (3, 4))
nested_tuple_c: tuple[tuple[int, int], tuple[int, int]] = ((1, 2), (3, 5))
assert nested_tuple_a == nested_tuple_b, "equal nested tuples should be equal"
assert nested_tuple_a != nested_tuple_c, "different nested tuples should not be equal"
print("Nested tuple equality passed")

# Different length tuples should not be equal
len_tuple_a: tuple[int, int, int] = (1, 2, 3)
len_tuple_b: tuple[int, int] = (1, 2)
assert len_tuple_a != len_tuple_b, "different length tuples should not be equal"
print("Different length tuple inequality passed")

# Single element tuple equality
single_tuple_eq_a: tuple[int] = (42,)
single_tuple_eq_b: tuple[int] = (42,)
single_tuple_eq_c: tuple[int] = (99,)
assert single_tuple_eq_a == single_tuple_eq_b, "equal single element tuples should be equal"
assert single_tuple_eq_a != single_tuple_eq_c, "different single element tuples should not be equal"
print("Single element tuple equality passed")

# ===== SECTION: Tuple ordering comparison =====

# Basic ordering - same length tuples
tuple_ord_1: tuple[int, int, int] = (1, 2, 3)
tuple_ord_2: tuple[int, int, int] = (1, 2, 4)
tuple_ord_3: tuple[int, int, int] = (1, 3, 0)
tuple_ord_4: tuple[int, int, int] = (1, 2, 3)

assert tuple_ord_1 < tuple_ord_2, "(1,2,3) < (1,2,4) should be True"
assert not tuple_ord_2 < tuple_ord_1, "(1,2,4) < (1,2,3) should be False"
assert tuple_ord_1 < tuple_ord_3, "(1,2,3) < (1,3,0) should be True"
assert not tuple_ord_3 < tuple_ord_1, "(1,3,0) < (1,2,3) should be False"
assert not tuple_ord_1 < tuple_ord_4, "(1,2,3) < (1,2,3) should be False (equal)"
print("Basic tuple less-than ordering passed")

assert tuple_ord_1 <= tuple_ord_2, "(1,2,3) <= (1,2,4) should be True"
assert not tuple_ord_2 <= tuple_ord_1, "(1,2,4) <= (1,2,3) should be False"
assert tuple_ord_1 <= tuple_ord_4, "(1,2,3) <= (1,2,3) should be True (equal)"
print("Basic tuple less-than-or-equal ordering passed")

assert tuple_ord_2 > tuple_ord_1, "(1,2,4) > (1,2,3) should be True"
assert not tuple_ord_1 > tuple_ord_2, "(1,2,3) > (1,2,4) should be False"
assert tuple_ord_3 > tuple_ord_1, "(1,3,0) > (1,2,3) should be True"
assert not tuple_ord_1 > tuple_ord_3, "(1,2,3) > (1,3,0) should be False"
assert not tuple_ord_1 > tuple_ord_4, "(1,2,3) > (1,2,3) should be False (equal)"
print("Basic tuple greater-than ordering passed")

assert tuple_ord_2 >= tuple_ord_1, "(1,2,4) >= (1,2,3) should be True"
assert not tuple_ord_1 >= tuple_ord_2, "(1,2,3) >= (1,2,4) should be False"
assert tuple_ord_1 >= tuple_ord_4, "(1,2,3) >= (1,2,3) should be True (equal)"
print("Basic tuple greater-than-or-equal ordering passed")

# Different length tuples - lexicographic semantics
tuple_short: tuple[int, int] = (1, 2)
tuple_long: tuple[int, int, int] = (1, 2, 3)
tuple_short_larger: tuple[int, int] = (1, 3)

assert tuple_short < tuple_long, "(1,2) < (1,2,3) should be True (prefix)"
assert not tuple_long < tuple_short, "(1,2,3) < (1,2) should be False"
assert tuple_short_larger > tuple_long, "(1,3) > (1,2,3) should be True (second element)"
assert not tuple_long > tuple_short_larger, "(1,2,3) > (1,3) should be False"
print("Different length tuple ordering passed")

assert tuple_short <= tuple_long, "(1,2) <= (1,2,3) should be True"
assert not tuple_long <= tuple_short, "(1,2,3) <= (1,2) should be False"
assert tuple_short_larger >= tuple_long, "(1,3) >= (1,2,3) should be True"
assert not tuple_long >= tuple_short_larger, "(1,2,3) >= (1,3) should be False"
print("Different length tuple ordering with equality passed")

# Empty tuple comparisons
tuple_empty: tuple[()] = ()
tuple_nonempty: tuple[int] = (1,)

assert tuple_empty < tuple_nonempty, "empty < non-empty should be True"
assert not tuple_nonempty < tuple_empty, "non-empty < empty should be False"
assert tuple_empty <= tuple_nonempty, "empty <= non-empty should be True"
assert not tuple_nonempty <= tuple_empty, "non-empty <= empty should be False"
assert tuple_nonempty > tuple_empty, "non-empty > empty should be True"
assert not tuple_empty > tuple_nonempty, "empty > non-empty should be False"
assert tuple_nonempty >= tuple_empty, "non-empty >= empty should be True"
assert not tuple_empty >= tuple_nonempty, "empty >= non-empty should be False"
print("Empty tuple ordering passed")

# Equal empty tuples
tuple_empty2: tuple[()] = ()
assert not tuple_empty < tuple_empty2, "empty < empty should be False (equal)"
assert tuple_empty <= tuple_empty2, "empty <= empty should be True (equal)"
assert not tuple_empty > tuple_empty2, "empty > empty should be False (equal)"
assert tuple_empty >= tuple_empty2, "empty >= empty should be True (equal)"
print("Equal empty tuple ordering passed")

# String tuples
tuple_str_1: tuple[str, str] = ("a", "b")
tuple_str_2: tuple[str, str] = ("a", "c")
tuple_str_3: tuple[str, str] = ("b", "a")

assert tuple_str_1 < tuple_str_2, "('a','b') < ('a','c') should be True"
assert not tuple_str_2 < tuple_str_1, "('a','c') < ('a','b') should be False"
assert tuple_str_1 < tuple_str_3, "('a','b') < ('b','a') should be True"
assert not tuple_str_3 < tuple_str_1, "('b','a') < ('a','b') should be False"
print("String tuple ordering passed")

# Nested tuples
tuple_nested_1: tuple[int, tuple[int, int]] = (1, (2, 3))
tuple_nested_2: tuple[int, tuple[int, int]] = (1, (2, 4))
tuple_nested_3: tuple[int, tuple[int, int]] = (1, (3, 0))

assert tuple_nested_1 < tuple_nested_2, "(1,(2,3)) < (1,(2,4)) should be True"
assert not tuple_nested_2 < tuple_nested_1, "(1,(2,4)) < (1,(2,3)) should be False"
assert tuple_nested_1 < tuple_nested_3, "(1,(2,3)) < (1,(3,0)) should be True"
assert not tuple_nested_3 < tuple_nested_1, "(1,(3,0)) < (1,(2,3)) should be False"
print("Nested tuple ordering passed")

# Single element tuples
tuple_single_1: tuple[int] = (5,)
tuple_single_2: tuple[int] = (10,)
tuple_single_3: tuple[int] = (5,)

assert tuple_single_1 < tuple_single_2, "(5,) < (10,) should be True"
assert not tuple_single_2 < tuple_single_1, "(10,) < (5,) should be False"
assert not tuple_single_1 < tuple_single_3, "(5,) < (5,) should be False (equal)"
assert tuple_single_1 <= tuple_single_3, "(5,) <= (5,) should be True (equal)"
assert tuple_single_2 > tuple_single_1, "(10,) > (5,) should be True"
assert tuple_single_2 >= tuple_single_1, "(10,) >= (5,) should be True"
print("Single element tuple ordering passed")

# Float tuples
tuple_float_1: tuple[float, float] = (1.5, 2.5)
tuple_float_2: tuple[float, float] = (1.5, 3.0)
tuple_float_3: tuple[float, float] = (2.0, 1.0)

assert tuple_float_1 < tuple_float_2, "(1.5,2.5) < (1.5,3.0) should be True"
assert not tuple_float_2 < tuple_float_1, "(1.5,3.0) < (1.5,2.5) should be False"
assert tuple_float_1 < tuple_float_3, "(1.5,2.5) < (2.0,1.0) should be True"
assert not tuple_float_3 < tuple_float_1, "(2.0,1.0) < (1.5,2.5) should be False"
print("Float tuple ordering passed")

# Bool tuples
tuple_bool_1: tuple[bool, bool] = (False, False)
tuple_bool_2: tuple[bool, bool] = (False, True)
tuple_bool_3: tuple[bool, bool] = (True, False)

assert tuple_bool_1 < tuple_bool_2, "(False,False) < (False,True) should be True"
assert not tuple_bool_2 < tuple_bool_1, "(False,True) < (False,False) should be False"
assert tuple_bool_1 < tuple_bool_3, "(False,False) < (True,False) should be True"
assert not tuple_bool_3 < tuple_bool_1, "(True,False) < (False,False) should be False"
print("Bool tuple ordering passed")

# Mixed types (same types at same positions)
tuple_mixed_1: tuple[int, str, float] = (1, "a", 2.5)
tuple_mixed_2: tuple[int, str, float] = (1, "a", 3.0)
tuple_mixed_3: tuple[int, str, float] = (1, "b", 1.0)

assert tuple_mixed_1 < tuple_mixed_2, "(1,'a',2.5) < (1,'a',3.0) should be True"
assert not tuple_mixed_2 < tuple_mixed_1, "(1,'a',3.0) < (1,'a',2.5) should be False"
assert tuple_mixed_1 < tuple_mixed_3, "(1,'a',2.5) < (1,'b',1.0) should be True"
assert not tuple_mixed_3 < tuple_mixed_1, "(1,'b',1.0) < (1,'a',2.5) should be False"
print("Mixed type tuple ordering passed")

# ===== SECTION: Nested unpacking =====

# Basic nested unpacking
nested1: tuple[int, tuple[int, int]] = (1, (2, 3))
a1, (b1, c1) = nested1
assert a1 == 1, "nested unpacking: a1 should be 1"
assert b1 == 2, "nested unpacking: b1 should be 2"
assert c1 == 3, "nested unpacking: c1 should be 3"
print("Basic nested unpacking passed")

# Deeper nesting (3 levels)
nested2: tuple[int, tuple[int, tuple[int, int]]] = (10, (20, (30, 40)))
x, (y, (z, w)) = nested2
assert x == 10, "deep nested: x should be 10"
assert y == 20, "deep nested: y should be 20"
assert z == 30, "deep nested: z should be 30"
assert w == 40, "deep nested: w should be 40"
print("Deep nested unpacking passed")

# Mixed tuple/list nested unpacking
mixed_nested: tuple[int, list[int]] = (1, [2, 3])
g, [h, i] = mixed_nested
assert g == 1, "mixed nested: g should be 1"
assert h == 2, "mixed nested: h should be 2"
assert i == 3, "mixed nested: i should be 3"
print("Mixed tuple/list nested unpacking passed")

# Multiple nested groups
multi_nested: tuple[tuple[int, int], tuple[int, int]] = ((1, 2), (3, 4))
(m1, m2), (m3, m4) = multi_nested
assert m1 == 1 and m2 == 2 and m3 == 3 and m4 == 4, "multi nested groups failed"
print("Multiple nested groups unpacking passed")

# ===== SECTION: Mixed-type tuple indexing =====

# Mixed-type tuple: str and int
mixed_tuple1: tuple[str, int] = ("hello", 42)
assert mixed_tuple1[0] == "hello", "mixed_tuple1[0] should be 'hello'"
assert mixed_tuple1[1] == 42, "mixed_tuple1[1] should be 42"

# Mixed-type tuple: int, str, bool
mixed_tuple2: tuple[int, str, bool] = (100, "world", True)
assert mixed_tuple2[0] == 100, "mixed_tuple2[0] should be 100"
assert mixed_tuple2[1] == "world", "mixed_tuple2[1] should be 'world'"
assert mixed_tuple2[2] == True, "mixed_tuple2[2] should be True"

# Mixed-type tuple: with float
mixed_tuple3: tuple[str, float, int] = ("pi", 3.14, 7)
assert mixed_tuple3[0] == "pi", "mixed_tuple3[0] should be 'pi'"
assert mixed_tuple3[1] == 3.14, "mixed_tuple3[1] should be 3.14"
assert mixed_tuple3[2] == 7, "mixed_tuple3[2] should be 7"

# Negative indexing on mixed-type tuple
assert mixed_tuple2[-1] == True, "mixed_tuple2[-1] should be True"
assert mixed_tuple2[-2] == "world", "mixed_tuple2[-2] should be 'world'"
assert mixed_tuple2[-3] == 100, "mixed_tuple2[-3] should be 100"

print("Mixed-type tuple indexing passed")

# ===== SECTION: Variable-length tuples (tuple[T, ...]) =====

# PEP 585 form — compiles and iterates over variable-length homogeneous tuple
def tv_sum_all(t: tuple[int, ...]) -> int:
    total: int = 0
    for x in t:
        total += x
    return total
assert tv_sum_all(()) == 0, "tv_sum_all(empty)"
assert tv_sum_all((1,)) == 1, "tv_sum_all((1,))"
assert tv_sum_all((1, 2, 3, 4)) == 10, "tv_sum_all((1,2,3,4))"

# Another TupleVar function — first element
def tv_first(t: tuple[int, ...]) -> int:
    if len(t) > 0:
        return t[0]
    return -1
assert tv_first(()) == -1, "tv_first(empty)"
assert tv_first((5,)) == 5, "tv_first((5,))"
assert tv_first((10, 20, 30)) == 10, "tv_first((10,20,30))"

# len() over TupleVar — widens fixed tuples at call site
def tv_describe(nums: tuple[int, ...]) -> int:
    return len(nums)
assert tv_describe(()) == 0, "tv_describe(empty)"
assert tv_describe((1,)) == 1, "tv_describe((1,))"
assert tv_describe((1, 2, 3)) == 3, "tv_describe((1,2,3))"

# zip over two TupleVars
def tv_sum_pairs(a: tuple[int, ...], b: tuple[int, ...]) -> int:
    total: int = 0
    for x, y in zip(a, b):
        total += x * y
    return total
assert tv_sum_pairs((1, 2, 3), (10, 20, 30)) == 10 + 40 + 90, "zip over TupleVars"

# enumerate over TupleVar
def tv_find(t: tuple[int, ...], v: int) -> int:
    for i, x in enumerate(t):
        if x == v:
            return i
    return -1
assert tv_find((10, 20, 30, 40), 30) == 2, "enumerate over TupleVar"
assert tv_find((10, 20, 30, 40), 99) == -1, "enumerate over TupleVar (miss)"

# Slice preserves element type and compares equal to a fixed tuple
def tv_take_two(t: tuple[int, ...]) -> tuple[int, ...]:
    return t[:2]
assert tv_take_two((1, 2, 3, 4, 5)) == (1, 2), "slice TupleVar == fixed"

# Contains operator
def tv_contains(t: tuple[int, ...], v: int) -> bool:
    return v in t
assert tv_contains((1, 2, 3), 2) is True
assert tv_contains((1, 2, 3), 99) is False
assert tv_contains((), 1) is False

print("Variable-length tuple tests passed")

# ============================================================================
# Tuple 'in' operator (regression test)
# ============================================================================

tuple_in_test: tuple[int, int, int, int, int] = (10, 20, 30, 40, 50)
assert 10 in tuple_in_test, "10 in (10,20,30,40,50) should be True"
assert 30 in tuple_in_test, "30 in (10,20,30,40,50) should be True"
assert 50 in tuple_in_test, "50 in (10,20,30,40,50) should be True"
assert 99 not in tuple_in_test, "99 not in (10,20,30,40,50) should be True"
assert 0 not in tuple_in_test, "0 not in (10,20,30,40,50) should be True"

tuple_str_in: tuple[str, str, str] = ("a", "b", "c")
assert "a" in tuple_str_in, "'a' in ('a','b','c') should be True"
assert "d" not in tuple_str_in, "'d' not in ('a','b','c') should be True"

print("Tuple 'in' operator tests passed!")

# ============================================================================
# x**2 in list comprehension (regression test)
# ============================================================================

pow_comp_result: list[int] = [x ** 2 for x in range(6)]
assert pow_comp_result == [0, 1, 4, 9, 16, 25], f"[x**2 for x in range(6)] failed: {pow_comp_result}"

pow_comp_cube: list[int] = [x ** 3 for x in range(5)]
assert pow_comp_cube == [0, 1, 8, 27, 64], f"[x**3 for x in range(5)] failed: {pow_comp_cube}"

# Mixed: multiplication and power in same comprehension
pow_comp_mixed: list[int] = [x * x + x ** 2 for x in range(4)]
assert pow_comp_mixed == [0, 2, 8, 18], f"[x*x + x**2] failed: {pow_comp_mixed}"

print("x**2 in list comprehension tests passed!")

# ============================================================================
# Empty list (no type annotation) + append + remove/insert
# Regression: empty list got elem_tag=ELEM_HEAP_OBJ but append stored raw ints,
# causing segfault when remove tried to dereference raw ints as pointers.
# ============================================================================

# Minimal case: empty list + append + remove
li_empty = []
li_empty.append(1)
li_empty.remove(1)
assert li_empty == [], "empty list append+remove should give []"

# Multiple appends then remove
li_build = []
li_build.append(10)
li_build.append(20)
li_build.append(30)
assert li_build == [10, 20, 30], "empty list multi-append"
li_build.remove(20)
assert li_build == [10, 30], "empty list remove middle element"

# Empty list + append + insert + remove
li_ops = []
li_ops.append(1)
li_ops.append(3)
li_ops.insert(1, 2)
assert li_ops == [1, 2, 3], "empty list append+insert"
li_ops.remove(2)
assert li_ops == [1, 3], "empty list insert then remove"

# Empty list + append + slicing + del + remove (original crash scenario)
def test_empty_list_complex():
    li = []
    li.append(1)
    li.append(2)
    li.append(4)
    li.append(3)
    assert li == [1, 2, 4, 3], "empty list append 4 elements"

    assert li[1:3] == [2, 4], "slice [1:3]"
    assert li[::2] == [1, 4], "slice [::2]"
    assert li[::-1] == [3, 4, 2, 1], "reverse slice"

    li2 = li[:]
    assert li2 == [1, 2, 4, 3], "full slice copy"

    del li[2]
    assert li == [1, 2, 3], "del element"

    li.remove(2)
    assert li == [1, 3], "remove after del"

    li.insert(1, 2)
    assert li == [1, 2, 3], "insert after remove"

test_empty_list_complex()

# Empty list + append in function scope with many allocations
def test_empty_list_gc_pressure():
    a = []
    a.append(1)
    a.append(2)
    a.append(3)
    # Create temporary lists to increase GC pressure
    t1 = [10, 20, 30]
    t2 = [40, 50, 60]
    t3 = a[:]
    assert a == [1, 2, 3], "list eq under gc pressure"
    a.remove(2)
    assert a == [1, 3], "remove under gc pressure"

test_empty_list_gc_pressure()

print("Empty list append/remove tests passed!")

# ============================================================================
# Empty container type inference from usage (no annotation)
# Regression: `li = []` got List(Any) / elem_tag=ELEM_HEAP_OBJ even when only
# ints are appended. The type planner now infers the element type from subsequent
# append/insert/add calls and creates the list with the correct elem_tag.
# ============================================================================

# String element inference: li = [] + append(str)
str_list = []
str_list.append("hello")
str_list.append("world")
assert str_list == ["hello", "world"], "empty list + append str"
assert len(str_list) == 2, "empty list + append str len"
str_list.remove("hello")
assert str_list == ["world"], "empty list + remove str"

# Int element inference inside if block
def test_empty_list_in_branch():
    flag: bool = True
    if flag:
        nums = []
        nums.append(42)
        nums.append(99)
        assert nums == [42, 99], "empty list in if branch"
        nums.remove(42)
        assert nums == [99], "remove in if branch"

test_empty_list_in_branch()

# Int element inference inside for loop body
def test_empty_list_in_loop():
    result = []
    for i in range(5):
        result.append(i)
    assert result == [0, 1, 2, 3, 4], "empty list in loop body"
    result.remove(2)
    assert result == [0, 1, 3, 4], "remove after loop append"

test_empty_list_in_loop()

# Empty list + insert infers element type
insert_list = []
insert_list.insert(0, 100)
insert_list.insert(0, 200)
assert insert_list == [200, 100], "empty list + insert int"
insert_list.remove(200)
assert insert_list == [100], "remove after insert"

print("Empty container type inference tests passed!")

# ===== SECTION: New unpacking shapes (BindingTarget migration) =====

# Flat — baseline
bt_a, bt_b, bt_c = (1, 2, 3)
assert (bt_a, bt_b, bt_c) == (1, 2, 3)

# Starred — baseline (homogeneous source so rest comparison is safe)
bt_a2, *bt_rest2 = [1, 2, 3, 4]
assert bt_a2 == 1 and bt_rest2 == [2, 3, 4]
*bt_rest3, bt_z3 = [1, 2, 3, 4]
assert bt_rest3 == [1, 2, 3] and bt_z3 == 4
bt_a4, *bt_mid4, bt_z4 = [1, 2, 3, 4, 5]
assert bt_a4 == 1 and bt_mid4 == [2, 3, 4] and bt_z4 == 5

# Nested — baseline
(bt_na, (bt_nb, bt_nc)) = (1, (2, 3))
assert (bt_na, bt_nb, bt_nc) == (1, 2, 3)

# NEW: nested + starred (element checks — comparison of starred rest from mixed
# tuple currently hits a pre-existing type-tag limitation; see INSIGHTS.md)
(bt_sna, *bt_snm, (bt_snb, bt_snc)) = (1, 2, 3, (4, 5))
assert bt_sna == 1 and bt_snb == 4 and bt_snc == 5
assert len(bt_snm) == 2 and bt_snm[0] == 2 and bt_snm[1] == 3

# NEW: attribute leaves
class BtC:
    x: int
    y: int

bt_obj = BtC()
bt_obj.x, bt_obj.y = 10, 20
assert (bt_obj.x, bt_obj.y) == (10, 20)

# NEW: subscript leaves
bt_lst = [0, 0]
bt_lst[0], bt_lst[1] = 111, 222
assert bt_lst == [111, 222]

# NEW: mixed leaves (Var, Attr, Index) — use a function to avoid module-level
# global-reassignment limitation (pre-existing, not caused by this commit)
class BtMix:
    x: int

def _test_mixed_leaves() -> None:
    bt_mix_obj = BtMix()
    bt_mix_lst = [0, 0]
    bt_mix_a, bt_mix_obj.x, bt_mix_lst[0] = 100, 200, 300
    assert bt_mix_a == 100 and bt_mix_obj.x == 200 and bt_mix_lst[0] == 300

_test_mixed_leaves()

# NEW: list-form target behaves like tuple-form
[bt_p, bt_q] = (7, 8)
assert (bt_p, bt_q) == (7, 8)

# NEW: deeply nested with both starred and attr leaf
class BtD:
    field: int

bt_d = BtD()
(bt_xa, *bt_xm, (bt_xb, bt_d.field)) = (1, 2, 3, (4, 5))
assert bt_xa == 1 and bt_xb == 4 and bt_d.field == 5
assert len(bt_xm) == 2 and bt_xm[0] == 2 and bt_xm[1] == 3

print("New unpacking shapes (BindingTarget migration) tests passed!")

print("All list and tuple collection tests passed!")
