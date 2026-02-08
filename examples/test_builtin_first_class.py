# Test built-in functions as first-class values
#
# NOTE: map/filter with builtins currently works only when the source elements
# are already boxed (strings, tuples, dicts, etc.). For list[int] elements,
# a user-defined lambda should be used instead until element boxing is implemented.
#
# These tests focus on the supported use cases:
# - sorted/list.sort/min/max with key= works for all types (including list[int])
# - map/filter with builtins works for collections of collections

# =============================================================================
# sorted() with key= builtin functions (works for all element types)
# =============================================================================

# sorted with key=len
words_sorted_1 = ["aaa", "b", "cc"]
result_sorted_len = sorted(words_sorted_1, key=len)
assert result_sorted_len == ["b", "cc", "aaa"], "sorted with key=len"

# sorted with key=len, reverse=True
result_sorted_len_rev = sorted(words_sorted_1, key=len, reverse=True)
assert result_sorted_len_rev == ["aaa", "cc", "b"], "sorted with key=len reverse"

# sorted with key=abs on list[int] - raw integers are boxed before calling key function
nums_sorted_abs = [-3, 1, -2, 4]
result_sorted_abs = sorted(nums_sorted_abs, key=abs)
assert result_sorted_abs == [1, -2, -3, 4], "sorted with key=abs on list[int]"

# sorted with key=str on list[int] - lexicographic string ordering
nums_sorted_str = [10, 2, 1, 20]
result_sorted_str = sorted(nums_sorted_str, key=str)
assert result_sorted_str == [1, 10, 2, 20], "sorted with key=str on list[int]"

# sorted lists by length
data_sorted = [[1], [1, 2, 3], [1, 2]]
result_sorted_lists = sorted(data_sorted, key=len)
# Verify sorted by checking lengths are in ascending order
assert len(result_sorted_lists[0]) == 1, "sorted lists with key=len - first"
assert len(result_sorted_lists[1]) == 2, "sorted lists with key=len - second"
assert len(result_sorted_lists[2]) == 3, "sorted lists with key=len - third"

# =============================================================================
# list.sort() with key= builtin functions
# =============================================================================

# list.sort with key=len (boxed elements)
words_sort_1 = ["aaa", "b", "cc"]
words_sort_1.sort(key=len)
assert words_sort_1 == ["b", "cc", "aaa"], "list.sort with key=len"

# list.sort with key=len, reverse=True (boxed elements)
words_sort_2 = ["aaa", "b", "cc"]
words_sort_2.sort(key=len, reverse=True)
assert words_sort_2 == ["aaa", "cc", "b"], "list.sort with key=len reverse"

# list.sort with key=abs on list[int]
nums_sort_abs = [-5, 2, -3]
nums_sort_abs.sort(key=abs)
assert nums_sort_abs == [2, -3, -5], "list.sort with key=abs on list[int]"

# =============================================================================
# min()/max() with key= builtin functions
# =============================================================================

# min with key=len (boxed elements)
words_minmax = ["aaa", "b", "cc"]
result_min_len = min(words_minmax, key=len)
assert result_min_len == "b", "min with key=len"

# max with key=len (boxed elements)
result_max_len = max(words_minmax, key=len)
assert result_max_len == "aaa", "max with key=len"

# min/max with key=abs on list[int]
nums_minmax_abs = [-5, 2, -3, 1]
result_min_abs = min(nums_minmax_abs, key=abs)
assert result_min_abs == 1, "min with key=abs on list[int]"
result_max_abs = max(nums_minmax_abs, key=abs)
assert result_max_abs == -5, "max with key=abs on list[int]"

# =============================================================================
# map() / filter() with builtin functions - KNOWN LIMITATIONS
# =============================================================================
#
# NOTE: map() and filter() with builtin function references have limitations:
#
# 1. map() with builtins (like `map(len, words)`) creates lists with boxed results.
#    When indexing such lists, you get raw pointers instead of values.
#    Printing the list works correctly due to type-dispatch in print.
#
# 2. filter() expects predicates that return bool (i8), but builtin wrappers
#    return boxed objects (*mut Obj). This causes type mismatch issues.
#
# For now, use lambdas instead for map/filter:
#   - Instead of `map(len, words)`, use `map(lambda x: len(x), words)`
#   - Instead of `filter(len, items)`, use `filter(lambda x: len(x) > 0, items)`
#
# The key= parameter for sorted/min/max works correctly because those functions
# properly handle the boxed return values from builtin wrappers.

print("All builtin first-class tests passed!")
