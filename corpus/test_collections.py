# Consolidated collections corpus: defaultdict, Counter, deque, OrderedDict,
# list, tuple, dict, set, bytes — plus container methods and augmented ops.
#
# Folded from (now redundant): test_collections_list_tuple.py,
# test_collections_dict_set_bytes.py, p4_methods.py, p21_container_methods.py,
# p35_counter.py, p51_container_aug_ops.py. Each folded source body lives in a
# `_fold_<source>()` function called at the end (isolation); module-level
# classes that cannot be nested inside a function are hoisted here with a prefix.
from collections import defaultdict, Counter, deque, OrderedDict


# ===== Hoisted module-level classes (pyaot: classes cannot nest in a function) =====
# From test_collections_list_tuple.py (BindingTarget unpacking with attr leaves).
class _LtBtC:
    x: int
    y: int


class _LtBtMix:
    x: int


class _LtBtD:
    field: int


# From p51_container_aug_ops.py (attribute aug-op target).
class _AugBox:
    def __init__(self):
        self.s = {1, 2, 3, 4}

# =============================================================================
# defaultdict
# =============================================================================

# ===== SECTION: defaultdict(int) =====
dd_int = defaultdict(int)
dd_int["a"] += 1
dd_int["b"] += 2
dd_int["a"] += 10
assert dd_int["a"] == 11, "defaultdict(int) augmented assign"
assert dd_int["b"] == 2, "defaultdict(int) single increment"
assert dd_int["c"] == 0, "defaultdict(int) missing key returns 0"
assert len(dd_int) == 3, "defaultdict(int) len after 3 key accesses"

# ===== SECTION: defaultdict(float) =====
dd_float = defaultdict(float)
dd_float["x"] += 1.5
assert dd_float["x"] == 1.5, "defaultdict(float) augmented assign"
assert dd_float["y"] == 0.0, "defaultdict(float) missing key returns 0.0"

# ===== SECTION: defaultdict(str) =====
dd_str = defaultdict(str)
dd_str["x"] += "hello"
assert dd_str["x"] == "hello", "defaultdict(str) concat"
assert dd_str["y"] == "", "defaultdict(str) missing key returns empty string"

# ===== SECTION: defaultdict(bool) =====
dd_bool = defaultdict(bool)
assert dd_bool["x"] == False, "defaultdict(bool) missing key returns False"

# ===== SECTION: defaultdict(list) =====
dd_list = defaultdict(list)
dd_list["fruits"].append("apple")
dd_list["fruits"].append("banana")
dd_list["vegs"].append("carrot")
assert len(dd_list["fruits"]) == 2, "defaultdict(list) append multiple"
assert len(dd_list["vegs"]) == 1, "defaultdict(list) append single"
assert len(dd_list["empty"]) == 0, "defaultdict(list) missing key returns empty list"

# ===== SECTION: defaultdict(dict) =====
dd_dict = defaultdict(dict)
assert len(dd_dict["x"]) == 0, "defaultdict(dict) missing key returns empty dict"

# ===== SECTION: defaultdict(set) =====
dd_set = defaultdict(set)
assert len(dd_set["x"]) == 0, "defaultdict(set) missing key returns empty set"

# ===== SECTION: defaultdict dict operations =====
dd_ops = defaultdict(int)
dd_ops["a"] = 10
dd_ops["b"] = 20
dd_ops["c"] = 30
assert dd_ops["a"] == 10, "defaultdict direct assignment"
assert "a" in dd_ops, "defaultdict 'in' present key"
assert "z" not in dd_ops, "defaultdict 'not in' absent key"
assert len(dd_ops) == 3, "defaultdict len"

# ===== SECTION: defaultdict .get() =====
dd_get = defaultdict(int)
dd_get["a"] = 5
assert dd_get.get("a") == 5, "defaultdict .get() existing key"
r = dd_get.get("missing")
assert r is None, "defaultdict .get() missing key returns None"
assert len(dd_get) == 1, "defaultdict .get() did not insert missing key"

# ===== SECTION: defaultdict del =====
dd_del = defaultdict(int)
dd_del["a"] = 1
dd_del["b"] = 2
del dd_del["a"]
assert "a" not in dd_del, "defaultdict del removes key"
assert len(dd_del) == 1, "defaultdict del decreases len"

# ===== SECTION: defaultdict .keys()/.values() =====
dd_kv = defaultdict(int)
dd_kv["x"] = 10
dd_kv["y"] = 20
k = dd_kv.keys()
assert len(k) == 2, "defaultdict .keys() length"
v = dd_kv.values()
assert len(v) == 2, "defaultdict .values() length"

# ===== SECTION: defaultdict() without factory =====
dd_none = defaultdict()
dd_none["a"] = 42
assert dd_none["a"] == 42, "defaultdict() no factory, direct access"

# =============================================================================
# Counter
# =============================================================================

# ===== SECTION: Counter from string =====
ctr_str = Counter("abracadabra")
assert ctr_str.total() == 11, "Counter.total() for 'abracadabra'"
mc3 = ctr_str.most_common(3)
assert len(mc3) == 3, "Counter.most_common(3) returns 3 items"

# ===== SECTION: Counter double indexing (print + compare via HeapAny) =====
first_pair = mc3[0]
assert first_pair[0] == "a", "Counter most_common first element is 'a'"
assert first_pair[1] == 5, "Counter most_common first count is 5"
assert mc3[1][1] == 2, "Counter most_common second count is 2"
assert mc3[0][1] != 3, "Counter most_common first count is not 3"

# ===== SECTION: Counter from list =====
ctr_list = Counter([1, 2, 1, 3, 2, 1])
assert ctr_list.total() == 6, "Counter.total() for int list"
mc2 = ctr_list.most_common(2)
assert len(mc2) == 2, "Counter.most_common(2) returns 2 items"

# ===== SECTION: Counter empty =====
ctr_empty = Counter()
assert ctr_empty.total() == 0, "empty Counter.total() is 0"
assert len(ctr_empty.most_common()) == 0, "empty Counter.most_common() is empty"

# ===== SECTION: Counter len =====
ctr_len = Counter("hello")
assert len(ctr_len) == 4, "Counter len (unique elements in 'hello': h,e,l,o)"
assert ctr_len.total() == 5, "Counter total (all chars in 'hello')"

# ===== SECTION: Counter most_common all =====
ctr_all = Counter("aabbc")
mc_all = ctr_all.most_common()
assert len(mc_all) == 3, "Counter.most_common() without arg returns all"

# =============================================================================
# deque
# =============================================================================

# ===== SECTION: deque empty =====
dq_empty = deque()
assert len(dq_empty) == 0, "empty deque len"

# ===== SECTION: deque append and len =====
dq_app = deque()
dq_app.append("a")
dq_app.append("b")
dq_app.append("c")
assert len(dq_app) == 3, "deque len after 3 appends"

# ===== SECTION: deque appendleft =====
dq_al = deque()
dq_al.append("b")
dq_al.append("c")
dq_al.appendleft("a")
dq_al.appendleft("z")
assert len(dq_al) == 4, "deque len after append + appendleft"

# ===== SECTION: deque maxlen =====
dq_max = deque(maxlen=3)
dq_max.append("a")
dq_max.append("b")
dq_max.append("c")
assert len(dq_max) == 3, "deque maxlen=3 at capacity"
dq_max.append("d")
assert len(dq_max) == 3, "deque maxlen=3 after overflow append"

# ===== SECTION: deque maxlen with appendleft =====
dq_max2 = deque(maxlen=2)
dq_max2.append("a")
dq_max2.append("b")
dq_max2.appendleft("z")
assert len(dq_max2) == 2, "deque maxlen=2 appendleft overflow"

# ===== SECTION: deque pop/popleft =====
dq_pop = deque()
dq_pop.append("a")
dq_pop.append("b")
dq_pop.append("c")
dq_pop.pop()
assert len(dq_pop) == 2, "deque pop decreases len"
dq_pop.popleft()
assert len(dq_pop) == 1, "deque popleft decreases len"
dq_pop.pop()
assert len(dq_pop) == 0, "deque pop to empty"

# ===== SECTION: deque extend =====
dq_ext = deque()
dq_ext.append("start")
dq_ext.extend(["a", "b", "c"])
assert len(dq_ext) == 4, "deque extend with string list"

# ===== SECTION: deque extendleft =====
dq_extl = deque()
dq_extl.append("end")
dq_extl.extendleft(["c", "b", "a"])
assert len(dq_extl) == 4, "deque extendleft with string list"

# ===== SECTION: deque rotate =====
dq_rot = deque()
dq_rot.append("a")
dq_rot.append("b")
dq_rot.append("c")
dq_rot.append("d")
dq_rot.rotate(1)
assert len(dq_rot) == 4, "deque rotate(1) preserves length"
dq_rot.rotate(-2)
assert len(dq_rot) == 4, "deque rotate(-2) preserves length"

# ===== SECTION: deque reverse =====
dq_rev = deque()
dq_rev.append("x")
dq_rev.append("y")
dq_rev.append("z")
dq_rev.reverse()
assert len(dq_rev) == 3, "deque reverse preserves length"

# ===== SECTION: deque clear =====
dq_clr = deque()
dq_clr.append("a")
dq_clr.append("b")
dq_clr.clear()
assert len(dq_clr) == 0, "deque clear empties deque"
dq_clr.append("x")
assert len(dq_clr) == 1, "deque usable after clear"

# ===== SECTION: deque copy =====
dq_orig = deque()
dq_orig.append("a")
dq_orig.append("b")
dq_orig.append("c")
dq_copy = dq_orig.copy()
assert len(dq_copy) == 3, "deque copy preserves length"
dq_orig.pop()
assert len(dq_orig) == 2, "original deque modified after copy"
assert len(dq_copy) == 3, "copy not affected by original modification"

# ===== SECTION: deque count =====
dq_cnt = deque()
dq_cnt.append("a")
dq_cnt.append("b")
dq_cnt.append("a")
dq_cnt.append("c")
dq_cnt.append("a")
assert dq_cnt.count("a") == 3, "deque count matching element"
assert dq_cnt.count("b") == 1, "deque count single occurrence"
assert dq_cnt.count("z") == 0, "deque count absent element"

# ===== SECTION: deque capacity growth =====
dq_grow = deque()
for i in range(20):
    dq_grow.append("item")
assert len(dq_grow) == 20, "deque grows beyond initial capacity"

# ===== SECTION: deque maxlen with extend =====
dq_mext = deque(maxlen=5)
dq_mext.extend(["a", "b", "c", "d", "e", "f", "g"])
assert len(dq_mext) == 5, "deque maxlen=5 after extending with 7 items"

# ===== SECTION: list(deque) =====
# A deque is not an iterator object; list(deque) must convert via
# rt_list_from_deque (walking the ring buffer left-to-right), not feed the
# DequeObj to rt_list_from_iter (which would misread its header as an
# iterator kind and yield garbage / an empty list).
dq_tl = deque()
dq_tl.append("b")
dq_tl.append("c")
dq_tl.appendleft("a")
assert list(dq_tl) == ["a", "b", "c"], "list(deque) preserves left-to-right order"
assert len(list(dq_tl)) == 3, "list(deque) length"
# Int elements (tagged immediates) survive the conversion.
dq_ti = deque()
dq_ti.append(1)
dq_ti.append(2)
dq_ti.append(3)
assert list(dq_ti) == [1, 2, 3], "list(deque) of ints"
# Constructed from an iterable, then converted back.
assert list(deque([10, 20, 30])) == [10, 20, 30], "list(deque(iterable))"
# Empty deque converts to an empty list.
assert list(deque()) == [], "list(empty deque)"

# ===== SECTION: deque iteration (typed element) =====
# A deque[T] is iterable; the loop variable is typed `T` (raw for primitives),
# so arithmetic and methods on it work exactly as for list[T].
dq_it = deque([1, 2, 3])
dq_it_total = 0
for dq_it_x in dq_it:
    dq_it_total += dq_it_x
assert dq_it_total == 6, "deque iteration sums elements"
# Iteration over a constructed deque with arithmetic in the body.
dq_it_doubled = []
for dq_it_y in deque([10, 20, 30]):
    dq_it_doubled.append(dq_it_y * 2)
assert dq_it_doubled == [20, 40, 60], "deque iteration with element arithmetic"

# ===== SECTION: deque iteration after empty-bootstrap appends =====
# An empty `deque()` narrowed to `deque[int]` purely through observed appends
# must iterate with a raw-int loop variable (the solver disambiguation path).
dq_boot = deque()
dq_boot.append(1)
dq_boot.append(2)
dq_boot.append(3)
dq_boot_total = 0
for dq_boot_x in dq_boot:
    dq_boot_total += dq_boot_x * 10
assert dq_boot_total == 60, "empty-bootstrap deque iterates as deque[int]"
# appendleft-only bootstrap also marks the var as a deque.
dq_bootl = deque()
dq_bootl.appendleft(5)
dq_bootl.appendleft(7)
dq_bootl_sum = 0
for dq_bootl_x in dq_bootl:
    dq_bootl_sum += dq_bootl_x
assert dq_bootl_sum == 12, "appendleft-bootstrap deque iterates as deque[int]"

# ===== SECTION: deque reductions (sum / sorted / min / max) =====
assert sum(deque([1, 2, 3, 4])) == 10, "sum(deque)"
assert sorted(deque([3, 1, 2])) == [1, 2, 3], "sorted(deque)"
assert sorted(deque([3, 1, 2]), reverse=True) == [3, 2, 1], "sorted(deque, reverse)"
assert min(deque([3, 1, 2])) == 1, "min(deque)"
assert max(deque([3, 1, 2])) == 3, "max(deque)"
assert sum(deque([1.5, 2.5])) == 4.0, "sum(deque[float])"

# ===== SECTION: deque subscript dq[i] =====
dq_sub = deque([10, 20, 30])
assert dq_sub[0] == 10, "deque subscript first"
assert dq_sub[2] == 30, "deque subscript last (positive)"
assert dq_sub[-1] == 30, "deque subscript negative index"
assert dq_sub[-3] == 10, "deque subscript negative to first"

# ===== SECTION: deque membership x in dq =====
dq_mem = deque([10, 20, 30])
assert 20 in dq_mem, "deque membership present"
assert 99 not in dq_mem, "deque membership absent"
assert 10 in dq_mem, "deque membership first element"

# ===== SECTION: deque repr (str == repr) =====
assert str(deque([1, 2, 3])) == "deque([1, 2, 3])", "deque repr"
assert str(deque([1, 2, 3], 5)) == "deque([1, 2, 3], maxlen=5)", "deque repr with maxlen"
assert str(deque()) == "deque([])", "empty deque repr"

# ===== SECTION: deque as iterable into other builtins =====
# A deque has no rt_iter_* factory of its own; every iterable-consuming
# builtin snapshots it to a list (or routes through IterSourceKind::Deque).
assert tuple(deque([1, 2, 3])) == (1, 2, 3), "tuple(deque)"
assert sorted(set(deque([1, 2, 2, 3]))) == [1, 2, 3], "set(deque)"
dq_iter_src = iter(deque([10, 20, 30]))
assert next(dq_iter_src) == 10, "iter(deque) first"
assert next(dq_iter_src) == 20, "iter(deque) second"
dq_rev_acc = []
for dq_rev_x in reversed(deque([1, 2, 3])):
    dq_rev_acc.append(dq_rev_x)
assert dq_rev_acc == [3, 2, 1], "reversed(deque)"
dq_enum_acc = []
for dq_enum_i, dq_enum_v in enumerate(deque([5, 6, 7])):
    dq_enum_acc.append(dq_enum_i * 100 + dq_enum_v)
assert dq_enum_acc == [5, 106, 207], "enumerate(deque)"
dq_zip_acc = []
for dq_zip_a, dq_zip_b in zip(deque([1, 2, 3]), deque([4, 5, 6])):
    dq_zip_acc.append(dq_zip_a * 10 + dq_zip_b)
assert dq_zip_acc == [14, 25, 36], "zip(deque, deque)"
assert list(map(lambda dq_m: dq_m * 2, deque([1, 2, 3]))) == [2, 4, 6], "map over deque"
dq_cnt_b = Counter(deque([1, 1, 2, 3, 3, 3]))
assert dq_cnt_b[1] == 2 and dq_cnt_b[2] == 1 and dq_cnt_b[3] == 3, "Counter(deque)"
assert ",".join(deque(["a", "b", "c"])) == "a,b,c", "str.join(deque)"

# ===== SECTION: all / any over deque =====
assert all(deque([1, 2, 3])) is True, "all(deque) truthy"
assert all(deque([1, 0, 3])) is False, "all(deque) with falsy"
assert any(deque([0, 0, 0])) is False, "any(deque) all falsy"
assert any(deque([0, 1, 0])) is True, "any(deque) one truthy"

# ===== SECTION: deque pop / popleft unbox primitive elements =====
dq_pop_u = deque([5, 6, 7])
assert dq_pop_u.pop() == 7, "deque.pop() unboxes int"
assert dq_pop_u.popleft() == 5, "deque.popleft() unboxes int"
assert list(dq_pop_u) == [6], "deque after pop/popleft"

# ===== SECTION: deque item assignment dq[i] = v =====
dq_set = deque([1, 2, 3])
dq_set[1] = 99
assert list(dq_set) == [1, 99, 3], "deque dq[i] = v positive index"
dq_set[-1] = 77
assert list(dq_set) == [1, 99, 77], "deque dq[i] = v negative index"

# ===== SECTION: del dq[i] =====
dq_del = deque([10, 20, 30, 40])
del dq_del[1]
assert list(dq_del) == [10, 30, 40], "del dq[i] middle"
del dq_del[-1]
assert list(dq_del) == [10, 30], "del dq[i] negative index"
del dq_del[0]
assert list(dq_del) == [30], "del dq[i] first"

# ===== SECTION: deque index / insert / remove =====
dq_idx = deque([10, 20, 30, 20])
assert dq_idx.index(20) == 1, "deque.index returns first match"
assert dq_idx.index(30) == 2, "deque.index middle"
dq_ins = deque([10, 20, 30])
dq_ins.insert(1, 15)
assert list(dq_ins) == [10, 15, 20, 30], "deque.insert middle"
dq_ins.insert(0, 5)
assert list(dq_ins) == [5, 10, 15, 20, 30], "deque.insert front"
dq_ins.insert(100, 99)
assert list(dq_ins) == [5, 10, 15, 20, 30, 99], "deque.insert clamps past end"
dq_rem = deque([10, 20, 30, 20])
dq_rem.remove(20)
assert list(dq_rem) == [10, 30, 20], "deque.remove first occurrence"

# ===== SECTION: deque insert at maxlen raises IndexError =====
dq_insmax = deque([1, 2, 3], 3)
try:
    dq_insmax.insert(1, 99)
    assert False, "deque.insert at maxlen should raise"
except IndexError:
    pass

# ===== SECTION: deque.index / remove of absent element raises ValueError =====
dq_absent = deque([1, 2, 3])
try:
    dq_absent.index(99)
    assert False, "deque.index absent should raise"
except ValueError:
    pass
try:
    dq_absent.remove(99)
    assert False, "deque.remove absent should raise"
except ValueError:
    pass


# ===== SECTION: f(*deque) argument unpacking =====
def _deque_sum3(a: int, b: int, c: int) -> int:
    return a + b + c


dq_star = deque([1, 2, 3])
assert _deque_sum3(*dq_star) == 6, "f(*deque) unpacking"

# =============================================================================
# OrderedDict
# =============================================================================

# ===== SECTION: OrderedDict basic =====
od = OrderedDict()
od["a"] = 1
od["b"] = 2
od["c"] = 3
assert len(od) == 3, "OrderedDict len"
assert "a" in od, "OrderedDict 'in' present"
assert "z" not in od, "OrderedDict 'not in' absent"
assert od["b"] == 2, "OrderedDict subscript access"

# ===== SECTION: OrderedDict move_to_end =====
od3 = OrderedDict()
od3["a"] = 1
od3["b"] = 2
od3["c"] = 3
od3.move_to_end("a")  # Move "a" to end: b, c, a
assert len(od3) == 3, "OrderedDict move_to_end preserves length"
t3 = od3.popitem()  # Pop from end: ("a", 1)
assert len(od3) == 2, "OrderedDict popitem reduces length"
assert "a" not in od3, "OrderedDict popitem removed moved key"

# ===== SECTION: OrderedDict move_to_end(last=False) =====
od4 = OrderedDict()
od4["a"] = 1
od4["b"] = 2
od4["c"] = 3
od4.move_to_end("c", False)  # Move "c" to beginning: c, a, b
t4 = od4.popitem(False)  # Pop from beginning: ("c", 3)
assert len(od4) == 2, "OrderedDict popitem(last=False) reduces length"
assert "c" not in od4, "OrderedDict popitem(False) removed correct key"

# ===== SECTION: OrderedDict popitem =====
od5 = OrderedDict()
od5["x"] = 10
od5["y"] = 20
od5["z"] = 30
od5.popitem()  # Remove last ("z", 30)
assert len(od5) == 2, "OrderedDict popitem() removes last"
assert "z" not in od5, "OrderedDict popitem() removed correct key"
od5.popitem(False)  # Remove first ("x", 10)
assert len(od5) == 1, "OrderedDict popitem(False) removes first"
assert "x" not in od5, "OrderedDict popitem(False) removed correct key"
assert "y" in od5, "OrderedDict remaining key after popitem"

# ===== SECTION: Destructuring a Tuple[Any] (regression) =====
# Before the fix, `a, b = ...` over a `Tuple[Any]` widened each binding to
# the ambiguous `Any` type (printed as a raw i64 pointer), while index access
# `t[0]` correctly promoted to HeapAny. Both paths must now agree.
od6 = OrderedDict()
od6["alpha"] = 1
od6["beta"] = 2
key_last, val_last = od6.popitem()
assert key_last == "beta", (
    f"destructured key from popitem() must behave as str; got {key_last}"
)
assert val_last == 2, f"destructured value from popitem() must equal 2; got {val_last}"
# Also test `str(key)` round-trip which relies on HeapAny dispatch.
assert str(key_last) == "beta", (
    "str() of destructured Tuple[Any] element must dispatch on actual type tag"
)

# ===== SECTION: structural == for Counter and defaultdict =====
# Counter and defaultdict share the DictObj layout; `==` must compare by
# content (same keys + equal counts/values), not by pointer identity —
# both for concrete-typed operands and through the generic `rt_obj_eq`
# path when an operand is a dynamic `Any`.
ctr_eq_a = Counter("aabbc")
ctr_eq_b = Counter("bcaab")
ctr_eq_c = Counter("aabbd")
assert ctr_eq_a == ctr_eq_b, "Counter == is structural and order-independent"
assert ctr_eq_a != ctr_eq_c, "Counter != detects differing counts"

dd_eq_a = defaultdict(int)
dd_eq_a["x"] = 1
dd_eq_a["y"] = 2
dd_eq_b = defaultdict(int)
dd_eq_b["y"] = 2
dd_eq_b["x"] = 1
dd_eq_c = defaultdict(int)
dd_eq_c["x"] = 9
assert dd_eq_a == dd_eq_b, "defaultdict == is structural and order-independent"
assert dd_eq_a != dd_eq_c, "defaultdict != detects differing value"

# Dynamic operand (Any) routed through the generic object-eq path.
ctr_eq_dyn = [Counter("xy"), 0][0]
assert ctr_eq_dyn == Counter("yx"), "Any == Counter (structural)"
dd_eq_dyn = [dd_eq_a, 0][0]
assert dd_eq_dyn == dd_eq_b, "Any == defaultdict (structural)"

print("Structural Counter/defaultdict equality tests passed")


# =============================================================================
# FOLDED: test_collections_list_tuple.py  (list / tuple surface)
# =============================================================================
def _fold_list_tuple():
    # ----- List creation, indexing, len -----
    nums: list[int] = [1, 2, 3, 4, 5]
    assert nums[0] == 1
    assert nums[2] == 3
    assert nums[4] == 5
    assert nums[-1] == 5
    assert nums[-2] == 4
    assert len(nums) == 5

    empty_list: list[int] = []
    assert len(empty_list) == 0

    single_list: list[int] = [42]
    assert single_list[0] == 42
    assert len(single_list) == 1

    big_list: list[int] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
    assert big_list[0] == 10
    assert big_list[5] == 60
    assert big_list[9] == 100
    assert big_list[-1] == 100
    assert len(big_list) == 10

    # ----- List slicing -----
    slice_nums: list[int] = [0, 1, 2, 3, 4, 5]
    assert slice_nums[1:4] == [1, 2, 3]
    assert slice_nums[:3] == [0, 1, 2]
    assert slice_nums[3:] == [3, 4, 5]
    assert slice_nums[::2] == [0, 2, 4]
    assert slice_nums[::3] == [0, 3]
    assert slice_nums[-2:] == [4, 5]
    assert slice_nums[:-1] == [0, 1, 2, 3, 4]
    assert slice_nums[-4:-1] == [2, 3, 4]
    assert slice_nums[3:3] == []

    # ----- List methods -----
    items: list[int] = [1, 2]
    items.append(3)
    assert items == [1, 2, 3]
    items.append(4)
    assert items == [1, 2, 3, 4]

    extend_list: list[int] = [1, 2, 3]
    extend_list.extend([4, 5, 6])
    assert extend_list == [1, 2, 3, 4, 5, 6]

    values: list[int] = [10, 20, 30, 40]
    assert values.pop() == 40
    assert len(values) == 3
    assert values.pop(0) == 10
    assert values == [20, 30]

    data: list[int] = [1, 3, 4]
    data.insert(1, 2)
    assert data == [1, 2, 3, 4]

    to_clear: list[int] = [1, 2, 3, 4, 5]
    to_clear.clear()
    assert to_clear == []

    original: list[int] = [1, 2, 3]
    copied: list[int] = original.copy()
    assert copied == [1, 2, 3]
    copied.append(4)
    assert len(copied) == 4
    assert len(original) == 3

    nums2: list[int] = [1, 2, 3, 4, 5]
    nums2.reverse()
    assert nums2 == [5, 4, 3, 2, 1]

    single_rev: list[int] = [42]
    single_rev.reverse()
    assert single_rev == [42]

    build: list[int] = []
    build.append(1)
    build.append(2)
    build.append(3)
    subset: list[int] = build[1:].copy()
    assert subset == [2, 3]
    subset.reverse()
    assert subset == [3, 2]

    # list.index() / list.count()
    idx_list: list[int] = [10, 20, 30, 20, 40]
    assert idx_list.index(10) == 0
    assert idx_list.index(20) == 1
    assert idx_list.index(40) == 4
    count_list: list[int] = [1, 2, 2, 3, 2, 4]
    assert count_list.count(2) == 3
    assert count_list.count(1) == 1
    assert count_list.count(99) == 0

    str_idx_list: list[str] = ["hello", "world", "foo", "bar"]
    assert str_idx_list.index("hello") == 0
    assert str_idx_list.index("world") == 1
    assert str_idx_list.index("foo") == 2
    assert str_idx_list.index("bar") == 3

    sorted_chars: list[str] = sorted(set("zyxwvu"))
    assert sorted_chars.index("u") == 0
    assert sorted_chars.index("v") == 1
    assert sorted_chars.index("w") == 2
    assert sorted_chars.index("x") == 3
    assert sorted_chars.index("y") == 4
    assert sorted_chars.index("z") == 5

    str_count_list: list[str] = ["a", "b", "a", "c", "a"]
    assert str_count_list.count("a") == 3
    assert str_count_list.count("b") == 1
    assert str_count_list.count("x") == 0

    # ----- List equality -----
    assert [1, 2, 3] == [1, 2, 3]
    assert [1, 2, 3] != [1, 2, 4]
    empty_a: list[int] = []
    empty_b: list[int] = []
    assert empty_a == empty_b
    assert ["hello", "world"] == ["hello", "world"]

    # Cross-elem-tag list equality (starred-rest production)
    cx_a, *cx_rest = (1, 2, 3, 4)
    assert cx_rest == [2, 3, 4]
    assert [2, 3, 4] == cx_rest
    (cx_b, *cx_rest2, cx_last) = (1, 2, 3, (4, 5))
    assert cx_rest2 == [2, 3]
    assert [2, 3] == cx_rest2
    (cx_c, *cx_rest3, cx_last3) = (1, 2.5, 3, (4, 5))
    assert cx_rest3 == [2.5, 3]
    (cx_d, *cx_rest4, cx_last4) = (1, True, False, (4, 5))
    assert cx_rest4 == [True, False]

    # ----- List ordering (lexicographic) -----
    assert [1, 2, 3] < [1, 2, 4]
    assert [1, 2, 4] > [1, 2, 3]
    assert [1, 2, 3] <= [1, 2, 4]
    assert [1, 2, 4] >= [1, 2, 3]
    assert [1, 2, 3] <= [1, 2, 3]
    assert [1, 2, 3] >= [1, 2, 3]
    assert not ([1, 2, 3] < [1, 2, 3])
    assert not ([1, 2, 3] > [1, 2, 3])
    assert [1, 2] < [1, 2, 3]
    assert [1, 2, 3] > [1, 2]
    assert [1, 2] <= [1, 2, 3]
    assert [1, 2, 3] >= [1, 2]
    assert [] <= []
    assert [] >= []
    assert not ([] < [])
    assert [] < [1]
    assert [1] > []
    assert [1.0, 2.0] < [1.0, 3.0]
    assert [1.5, 2.5] > [1.5, 2.0]
    assert [1.0, 2.0] <= [1.0, 2.0]
    assert [1.0, 2.0] >= [1.0, 2.0]
    assert ["a", "b"] < ["a", "c"]
    assert ["b", "a"] > ["a", "z"]
    assert ["hello"] < ["hello", "world"]
    assert ["a", "b"] <= ["a", "b"]
    assert ["a", "b"] >= ["a", "b"]

    # min/max over list
    assert min([10, 20, 5, 40, 15]) == 5
    assert max([10, 20, 5, 40, 15]) == 40
    floats: list[float] = [1.5, 2.7, 3.14, 4.0, 5.5]
    assert floats == [1.5, 2.7, 3.14, 4.0, 5.5]
    assert min(floats) == 1.5
    assert max(floats) == 5.5
    assert len(floats) == 5

    # element assignment
    fruits: list[str] = ["apple", "banana", "cherry"]
    fruits[1] = "blueberry"
    assert fruits == ["apple", "blueberry", "cherry"]
    numbers_mod: list[int] = [1, 2, 3, 4, 5]
    numbers_mod[0] = 10
    assert numbers_mod == [10, 2, 3, 4, 5]

    # ----- Tuple creation, indexing -----
    point: tuple[int, int] = (10, 20)
    assert point[0] == 10
    assert point[1] == 20
    assert point[-1] == 20
    assert point[-2] == 10
    assert len(point) == 2
    single_tuple: tuple[int] = (99,)
    assert single_tuple[0] == 99
    assert len(single_tuple) == 1
    triple: tuple[int, int, int] = (1, 2, 3)
    assert triple[0] == 1
    assert triple[1] == 2
    assert triple[2] == 3
    assert len(triple) == 3

    # ----- Tuple slicing -----
    tuple_nums: tuple[int, int, int, int, int, int] = (0, 1, 2, 3, 4, 5)
    assert tuple_nums[1:4] == (1, 2, 3)
    assert tuple_nums[:3] == (0, 1, 2)
    assert tuple_nums[3:] == (3, 4, 5)
    assert tuple_nums[:] == (0, 1, 2, 3, 4, 5)
    assert tuple_nums[::2] == (0, 2, 4)
    assert tuple_nums[::3] == (0, 3)
    assert tuple_nums[1:5:2] == (1, 3)
    assert tuple_nums[-2:] == (4, 5)
    assert tuple_nums[:-1] == (0, 1, 2, 3, 4)
    assert tuple_nums[::-1] == (5, 4, 3, 2, 1, 0)
    assert tuple_nums[::-2] == (5, 3, 1)
    assert tuple_nums[3:3] == ()
    assert tuple_nums[4:2] == ()
    single_t: tuple[int] = (42,)
    assert single_t[:] == (42,)
    assert tuple_nums[3:100] == (3, 4, 5)
    assert tuple_nums[100:] == ()

    # ----- Tuple index() / count() -----
    tuple_search: tuple[int, int, int, int] = (1, 2, 2, 3)
    assert tuple_search.index(2) == 1
    assert tuple_search.index(3) == 3
    assert tuple_search.count(2) == 2
    assert tuple_search.count(1) == 1
    assert tuple_search.count(99) == 0
    tuple_bools: tuple[bool, bool, bool] = (True, False, True)
    assert tuple_bools.index(False) == 1
    assert tuple_bools.count(True) == 2
    tuple_strs: tuple[str, str, str] = ("a", "b", "b")
    assert tuple_strs.index("b") == 1
    assert tuple_strs.count("b") == 2
    try:
        tuple_search.index(99)
        raise AssertionError("tuple.index of absent value should raise")
    except ValueError:
        pass

    # ----- list.sort() (incl. key/reverse) -----
    sort_nums: list[int] = [3, 1, 4, 1, 5, 9, 2, 6]
    sort_nums.sort()
    assert sort_nums == [1, 1, 2, 3, 4, 5, 6, 9]
    sorted_nums: list[int] = [1, 2, 3, 4, 5]
    sorted_nums.sort()
    assert sorted_nums == [1, 2, 3, 4, 5]
    reverse_nums: list[int] = [5, 4, 3, 2, 1]
    reverse_nums.sort()
    assert reverse_nums == [1, 2, 3, 4, 5]
    rev_sort_nums: list[int] = [3, 1, 4, 1, 5]
    rev_sort_nums.sort(reverse=True)
    assert rev_sort_nums == [5, 4, 3, 1, 1]
    single_sort: list[int] = [42]
    single_sort.sort()
    assert single_sort == [42]
    empty_sort: list[int] = []
    empty_sort.sort()
    assert empty_sort == []
    neg_nums: list[int] = [-3, 1, -4, 1, 5]
    neg_nums.sort()
    assert neg_nums == [-4, -3, 1, 1, 5]
    sort_strs: list[str] = ["banana", "apple", "cherry"]
    sort_strs.sort()
    assert sort_strs == ["apple", "banana", "cherry"]

    def _str_len(s: str) -> int:
        return len(s)

    strs_by_len: list[str] = ["apple", "fig", "banana", "kiwi"]
    strs_by_len.sort(key=_str_len)
    assert strs_by_len == ["fig", "kiwi", "apple", "banana"]
    strs_by_len2: list[str] = ["apple", "fig", "banana", "kiwi"]
    strs_by_len2.sort(key=_str_len, reverse=True)
    assert strs_by_len2 == ["banana", "apple", "kiwi", "fig"]
    nums_key_none: list[int] = [3, 1, 4, 1, 5]
    nums_key_none.sort(key=None)
    assert nums_key_none == [1, 1, 3, 4, 5]

    def _abs_val(x: int) -> int:
        if x < 0:
            return -x
        return x

    nums_abs: list[int] = [-5, 2, -3, 1, -4]
    nums_abs.sort(key=_abs_val)
    assert nums_abs == [1, 2, -3, -4, -5]
    sort_explicit: list[str] = ["cherry", "apple", "banana"]
    sort_explicit.sort(key=_str_len, reverse=False)
    assert sort_explicit == ["apple", "cherry", "banana"]

    # ----- Container printing (str() round-trip) -----
    assert str([1, 2, 3]) == "[1, 2, 3]"
    assert str(["a", "b"]) == "['a', 'b']"
    assert str([]) == "[]"
    assert str([[1, 2], [3, 4]]) == "[[1, 2], [3, 4]]"
    print_mixed_list = [1, 2, ["string"]]
    assert str(print_mixed_list) == "[1, 2, ['string']]"
    assert str((1, 2, 3)) == "(1, 2, 3)"
    assert str((42,)) == "(42,)"
    assert str(("hello", "world")) == "('hello', 'world')"

    # ----- Tuple equality -----
    assert (1, 2, 3) == (1, 2, 3)
    assert (1, 2, 3) != (1, 2, 4)
    empty_tuple_a: tuple[()] = ()
    empty_tuple_b: tuple[()] = ()
    assert empty_tuple_a == empty_tuple_b
    assert ("a", "b", "c") == ("a", "b", "c")
    assert ("a", "b", "c") != ("a", "b", "d")
    assert (1, "hello", 3) == (1, "hello", 3)
    assert (1, "hello", 3) != (1, "world", 3)
    assert ((1, 2), (3, 4)) == ((1, 2), (3, 4))
    assert ((1, 2), (3, 4)) != ((1, 2), (3, 5))
    len_tuple_a: tuple[int, int, int] = (1, 2, 3)
    len_tuple_b: tuple[int, int] = (1, 2)
    assert len_tuple_a != len_tuple_b
    assert (42,) == (42,)
    assert (42,) != (99,)

    # ----- Tuple ordering -----
    assert (1, 2, 3) < (1, 2, 4)
    assert not ((1, 2, 4) < (1, 2, 3))
    assert (1, 2, 3) < (1, 3, 0)
    assert not ((1, 3, 0) < (1, 2, 3))
    assert not ((1, 2, 3) < (1, 2, 3))
    assert (1, 2, 3) <= (1, 2, 4)
    assert not ((1, 2, 4) <= (1, 2, 3))
    assert (1, 2, 3) <= (1, 2, 3)
    assert (1, 2, 4) > (1, 2, 3)
    assert not ((1, 2, 3) > (1, 2, 4))
    assert (1, 3, 0) > (1, 2, 3)
    assert (1, 2, 4) >= (1, 2, 3)
    assert not ((1, 2, 3) >= (1, 2, 4))
    assert (1, 2, 3) >= (1, 2, 3)
    # different lengths
    assert (1, 2) < (1, 2, 3)
    assert not ((1, 2, 3) < (1, 2))
    assert (1, 3) > (1, 2, 3)
    assert not ((1, 2, 3) > (1, 3))
    assert (1, 2) <= (1, 2, 3)
    assert (1, 3) >= (1, 2, 3)
    # empty tuple comparisons
    assert () < (1,)
    assert not ((1,) < ())
    assert () <= (1,)
    assert (1,) > ()
    assert (1,) >= ()
    assert not (() < ())
    assert () <= ()
    assert () >= ()
    # string / nested / single / float / bool / mixed tuples
    assert ("a", "b") < ("a", "c")
    assert ("a", "b") < ("b", "a")
    assert (1, (2, 3)) < (1, (2, 4))
    assert (1, (2, 3)) < (1, (3, 0))
    assert (5,) < (10,)
    assert not ((5,) < (5,))
    assert (5,) <= (5,)
    assert (10,) > (5,)
    assert (1.5, 2.5) < (1.5, 3.0)
    assert (1.5, 2.5) < (2.0, 1.0)
    assert (False, False) < (False, True)
    assert (False, False) < (True, False)
    assert (1, "a", 2.5) < (1, "a", 3.0)
    assert (1, "a", 2.5) < (1, "b", 1.0)

    # ----- Nested unpacking -----
    a1, (b1, c1) = (1, (2, 3))
    assert a1 == 1 and b1 == 2 and c1 == 3
    x, (y, (z, w)) = (10, (20, (30, 40)))
    assert x == 10 and y == 20 and z == 30 and w == 40
    g, [h, i] = (1, [2, 3])
    assert g == 1 and h == 2 and i == 3
    (m1, m2), (m3, m4) = ((1, 2), (3, 4))
    assert m1 == 1 and m2 == 2 and m3 == 3 and m4 == 4

    # ----- Mixed-type tuple indexing -----
    mixed_tuple1: tuple[str, int] = ("hello", 42)
    assert mixed_tuple1[0] == "hello"
    assert mixed_tuple1[1] == 42
    mixed_tuple2: tuple[int, str, bool] = (100, "world", True)
    assert mixed_tuple2[0] == 100
    assert mixed_tuple2[1] == "world"
    assert mixed_tuple2[2] == True
    mixed_tuple3: tuple[str, float, int] = ("pi", 3.14, 7)
    assert mixed_tuple3[0] == "pi"
    assert mixed_tuple3[1] == 3.14
    assert mixed_tuple3[2] == 7
    assert mixed_tuple2[-1] == True
    assert mixed_tuple2[-2] == "world"
    assert mixed_tuple2[-3] == 100

    # ----- Variable-length tuples tuple[T, ...] -----
    def _tv_sum_all(t: tuple[int, ...]) -> int:
        total: int = 0
        for tv in t:
            total += tv
        return total

    assert _tv_sum_all(()) == 0
    assert _tv_sum_all((1,)) == 1
    assert _tv_sum_all((1, 2, 3, 4)) == 10

    def _tv_first(t: tuple[int, ...]) -> int:
        if len(t) > 0:
            return t[0]
        return -1

    assert _tv_first(()) == -1
    assert _tv_first((5,)) == 5
    assert _tv_first((10, 20, 30)) == 10

    def _tv_describe(n: tuple[int, ...]) -> int:
        return len(n)

    assert _tv_describe(()) == 0
    assert _tv_describe((1,)) == 1
    assert _tv_describe((1, 2, 3)) == 3

    def _tv_sum_pairs(a: tuple[int, ...], b: tuple[int, ...]) -> int:
        total: int = 0
        for px, py in zip(a, b):
            total += px * py
        return total

    assert _tv_sum_pairs((1, 2, 3), (10, 20, 30)) == 10 + 40 + 90

    def _tv_find(t: tuple[int, ...], v: int) -> int:
        for fi, fx in enumerate(t):
            if fx == v:
                return fi
        return -1

    assert _tv_find((10, 20, 30, 40), 30) == 2
    assert _tv_find((10, 20, 30, 40), 99) == -1

    def _tv_take_two(t: tuple[int, ...]) -> tuple[int, ...]:
        return t[:2]

    assert _tv_take_two((1, 2, 3, 4, 5)) == (1, 2)

    def _tv_contains(t: tuple[int, ...], v: int) -> bool:
        return v in t

    assert _tv_contains((1, 2, 3), 2) is True
    assert _tv_contains((1, 2, 3), 99) is False
    assert _tv_contains((), 1) is False

    # ----- Tuple 'in' operator -----
    tuple_in_test: tuple[int, int, int, int, int] = (10, 20, 30, 40, 50)
    assert 10 in tuple_in_test
    assert 30 in tuple_in_test
    assert 50 in tuple_in_test
    assert 99 not in tuple_in_test
    assert 0 not in tuple_in_test
    tuple_str_in: tuple[str, str, str] = ("a", "b", "c")
    assert "a" in tuple_str_in
    assert "d" not in tuple_str_in

    # ----- x**2 in list comprehension -----
    assert [x2 ** 2 for x2 in range(6)] == [0, 1, 4, 9, 16, 25]
    assert [x3 ** 3 for x3 in range(5)] == [0, 1, 8, 27, 64]
    assert [xm * xm + xm ** 2 for xm in range(4)] == [0, 2, 8, 18]

    # ----- Empty list (no annotation) + append + remove/insert -----
    li_empty = []
    li_empty.append(1)
    li_empty.remove(1)
    assert li_empty == []
    li_build = []
    li_build.append(10)
    li_build.append(20)
    li_build.append(30)
    assert li_build == [10, 20, 30]
    li_build.remove(20)
    assert li_build == [10, 30]
    li_ops = []
    li_ops.append(1)
    li_ops.append(3)
    li_ops.insert(1, 2)
    assert li_ops == [1, 2, 3]
    li_ops.remove(2)
    assert li_ops == [1, 3]

    def _empty_list_complex():
        li = []
        li.append(1)
        li.append(2)
        li.append(4)
        li.append(3)
        assert li == [1, 2, 4, 3]
        assert li[1:3] == [2, 4]
        assert li[::2] == [1, 4]
        assert li[::-1] == [3, 4, 2, 1]
        li2 = li[:]
        assert li2 == [1, 2, 4, 3]
        del li[2]
        assert li == [1, 2, 3]
        li.remove(2)
        assert li == [1, 3]
        li.insert(1, 2)
        assert li == [1, 2, 3]

    _empty_list_complex()

    def _empty_list_gc_pressure():
        a = []
        a.append(1)
        a.append(2)
        a.append(3)
        t1 = [10, 20, 30]
        t2 = [40, 50, 60]
        t3 = a[:]
        assert a == [1, 2, 3]
        a.remove(2)
        assert a == [1, 3]

    _empty_list_gc_pressure()

    # ----- Empty container type inference from usage -----
    str_list = []
    str_list.append("hello")
    str_list.append("world")
    assert str_list == ["hello", "world"]
    assert len(str_list) == 2
    str_list.remove("hello")
    assert str_list == ["world"]

    def _empty_list_in_branch():
        flag: bool = True
        if flag:
            nums_b = []
            nums_b.append(42)
            nums_b.append(99)
            assert nums_b == [42, 99]
            nums_b.remove(42)
            assert nums_b == [99]

    _empty_list_in_branch()

    def _empty_list_in_loop():
        result = []
        for li in range(5):
            result.append(li)
        assert result == [0, 1, 2, 3, 4]
        result.remove(2)
        assert result == [0, 1, 3, 4]

    _empty_list_in_loop()

    insert_list = []
    insert_list.insert(0, 100)
    insert_list.insert(0, 200)
    assert insert_list == [200, 100]
    insert_list.remove(200)
    assert insert_list == [100]

    # ----- Sequence repetition (list/bytes * int) -----
    rep_floats = [0.0] * 5
    assert len(rep_floats) == 5
    for rep_i in range(5):
        assert rep_floats[rep_i] == 0.0
    assert [1, 2, 3] * 2 == [1, 2, 3, 1, 2, 3]
    assert 3 * [7] == [7, 7, 7]
    rep_n = 4
    rep_dyn = [9.5] * rep_n
    assert len(rep_dyn) == 4
    assert rep_dyn[0] == 9.5 and rep_dyn[3] == 9.5
    assert [42] * True == [42]
    assert [42] * False == []
    assert [42] * 0 == []
    assert [99] * -3 == []
    rep_empty_src: list[int] = []
    assert rep_empty_src * 100 == []
    rep_str = ["abc"] * 3
    assert len(rep_str) == 3
    assert rep_str[0] == "abc" and rep_str[1] == "abc" and rep_str[2] == "abc"
    rep_src = [10, 20]
    rep_dst = rep_src * 2
    rep_src.append(30)
    assert rep_dst == [10, 20, 10, 20]
    assert rep_src == [10, 20, 30]
    assert b"ab" * 3 == b"ababab"
    assert b"ab" * 0 == b""
    assert "x" * 5 == "xxxxx"
    assert "z" * 0 == ""

    # ----- New unpacking shapes (BindingTarget migration) -----
    bt_a, bt_b, bt_c = (1, 2, 3)
    assert (bt_a, bt_b, bt_c) == (1, 2, 3)
    bt_a2, *bt_rest2 = [1, 2, 3, 4]
    assert bt_a2 == 1 and bt_rest2 == [2, 3, 4]
    *bt_rest3, bt_z3 = [1, 2, 3, 4]
    assert bt_rest3 == [1, 2, 3] and bt_z3 == 4
    bt_a4, *bt_mid4, bt_z4 = [1, 2, 3, 4, 5]
    assert bt_a4 == 1 and bt_mid4 == [2, 3, 4] and bt_z4 == 5
    (bt_na, (bt_nb, bt_nc)) = (1, (2, 3))
    assert (bt_na, bt_nb, bt_nc) == (1, 2, 3)
    (bt_sna, *bt_snm, (bt_snb, bt_snc)) = (1, 2, 3, (4, 5))
    assert bt_sna == 1 and bt_snb == 4 and bt_snc == 5
    assert len(bt_snm) == 2 and bt_snm[0] == 2 and bt_snm[1] == 3

    bt_obj = _LtBtC()
    bt_obj.x, bt_obj.y = 10, 20
    assert (bt_obj.x, bt_obj.y) == (10, 20)
    bt_lst = [0, 0]
    bt_lst[0], bt_lst[1] = 111, 222
    assert bt_lst == [111, 222]

    def _test_mixed_leaves() -> None:
        bt_mix_obj = _LtBtMix()
        bt_mix_lst = [0, 0]
        bt_mix_a, bt_mix_obj.x, bt_mix_lst[0] = 100, 200, 300
        assert bt_mix_a == 100 and bt_mix_obj.x == 200 and bt_mix_lst[0] == 300

    _test_mixed_leaves()
    [bt_p, bt_q] = (7, 8)
    assert (bt_p, bt_q) == (7, 8)
    bt_d = _LtBtD()
    (bt_xa, *bt_xm, (bt_xb, bt_d.field)) = (1, 2, 3, (4, 5))
    assert bt_xa == 1 and bt_xb == 4 and bt_d.field == 5
    assert len(bt_xm) == 2 and bt_xm[0] == 2 and bt_xm[1] == 3

    # ----- Structural == for a container behind a dynamic operand -----
    _dyn_eq_items = [(1.0,), 99, "x"]
    _dyn_eq_first = _dyn_eq_items[0]
    assert _dyn_eq_first == (1.0,)
    assert (1.0,) == _dyn_eq_first
    assert _dyn_eq_first != (2.0,)
    _dyn_eq_lists = [[1, 2, 3], 0]
    _dyn_eq_lst = _dyn_eq_lists[0]
    assert _dyn_eq_lst == [1, 2, 3]
    assert _dyn_eq_lst != [1, 2]


_fold_list_tuple()


# =============================================================================
# FOLDED: test_collections_dict_set_bytes.py  (dict / set / bytes surface)
# =============================================================================
def _fold_dict_set_bytes():
    # ----- Dict creation, indexing, assignment -----
    d: dict[str, int] = {"a": 1, "b": 2, "c": 3}
    assert len(d) == 3
    assert d["a"] == 1
    assert d["b"] == 2
    assert d["c"] == 3
    d["d"] = 4
    assert d["d"] == 4
    assert len(d) == 4
    d["a"] = 10
    assert d["a"] == 10

    # ----- Dict 'in' -----
    assert "a" in d
    assert "b" in d
    assert "z" not in d
    assert "missing" not in d
    nums: dict[int, str] = {1: "one", 2: "two", 3: "three"}
    assert len(nums) == 3
    assert nums[1] == "one"
    assert nums[2] == "two"
    assert nums[3] == "three"
    assert 1 in nums
    assert 2 in nums
    assert 99 not in nums
    empty_dict: dict[str, int] = {}
    assert len(empty_dict) == 0
    assert "key" not in empty_dict

    # ----- Dict methods -----
    assert d.get("a") == 10
    copy_d: dict[str, int] = {"x": 1, "y": 2}
    assert len(copy_d) == 2
    copy_d.clear()
    assert len(copy_d) == 0
    original_dict: dict[str, int] = {"m": 100, "n": 200}
    copied_dict: dict[str, int] = original_dict.copy()
    assert len(copied_dict) == 2
    assert copied_dict["m"] == 100
    assert copied_dict["n"] == 200
    copied_dict["m"] = 999
    assert original_dict["m"] == 100
    pop_test: dict[str, int] = {"a": 1, "b": 2, "c": 3}
    assert pop_test.pop("b") == 2
    assert len(pop_test) == 2
    assert "b" not in pop_test
    data: dict[str, int] = {}
    data["first"] = 1
    data["second"] = 2
    data["third"] = 3
    assert len(data) == 3
    assert data["first"] == 1
    data["first"] = 100
    assert data["first"] == 100
    d1: dict[str, int] = {"a": 1, "b": 2}
    d2: dict[str, int] = {"b": 20, "c": 3}
    d1.update(d2)
    assert d1["a"] == 1
    assert d1["b"] == 20
    assert d1["c"] == 3
    assert len(d1) == 3

    # ----- Set literals / set() -----
    s: set[int] = {1, 2, 3}
    assert len(s) == 3
    assert 1 in s and 2 in s and 3 in s
    assert 4 not in s
    empty_set: set[int] = set()
    assert len(empty_set) == 0
    from_list: set[int] = set([1, 2, 2, 3])
    assert len(from_list) == 3
    chars: set[str] = {"a", "b", "c"}
    assert len(chars) == 3
    assert "a" in chars and "b" in chars and "c" in chars

    # ----- Set methods -----
    s2: set[int] = set()
    s2.add(1)
    s2.add(2)
    s2.add(1)
    assert len(s2) == 2
    assert 1 in s2 and 2 in s2
    s3: set[int] = {1, 2, 3}
    s3.remove(2)
    assert len(s3) == 2
    assert 2 not in s3
    s4: set[int] = {1, 2}
    s4.discard(2)
    s4.discard(99)
    assert len(s4) == 1
    assert 2 not in s4
    s5: set[int] = {1, 2, 3}
    s5.clear()
    assert len(s5) == 0
    original_set: set[int] = {1, 2, 3}
    copied_set: set[int] = original_set.copy()
    assert sorted(copied_set) == [1, 2, 3]

    # ----- Set iteration / comprehensions -----
    total: int = 0
    for set_x in {1, 2, 3}:
        total = total + set_x
    assert total == 6
    squares: set[int] = {sx * sx for sx in range(5)}
    assert sorted(squares) == [0, 1, 4, 9, 16]
    evens_set: set[int] = {sx for sx in range(10) if sx % 2 == 0}
    assert sorted(evens_set) == [0, 2, 4, 6, 8]

    # ----- Container printing (deterministic) -----
    assert str({"a": 1, "b": 2}) == "{'a': 1, 'b': 2}"
    assert str({"name": "Alice", "city": "NYC"}) == "{'name': 'Alice', 'city': 'NYC'}"
    assert str({}) == "{}"
    assert sorted({"x", "y"}) == ["x", "y"]

    # ----- del statement -----
    del_dict: dict[str, int] = {"a": 1, "b": 2, "c": 3}
    del del_dict["b"]
    assert len(del_dict) == 2
    assert "b" not in del_dict
    assert "a" in del_dict and "c" in del_dict
    del_list: list[int] = [10, 20, 30, 40]
    del del_list[1]
    assert del_list == [10, 30, 40]

    # ----- dict.setdefault() / dict.popitem() -----
    setdefault_dict: dict[str, int] = {"a": 1, "b": 2}
    assert setdefault_dict.setdefault("a", 999) == 1
    assert setdefault_dict["a"] == 1
    assert setdefault_dict.setdefault("c", 3) == 3
    assert setdefault_dict["c"] == 3
    assert len(setdefault_dict) == 3
    setdefault_none: dict[str, int] = {"x": 10}
    assert setdefault_none.setdefault("y", 0) == 0
    assert setdefault_none["y"] == 0
    popitem_dict: dict[str, int] = {"a": 1, "b": 2, "c": 3}
    last_item: tuple[str, int] = popitem_dict.popitem()
    assert len(last_item) == 2
    assert len(popitem_dict) == 2
    assert last_item[0] not in popitem_dict
    popitem_dict2: dict[str, int] = {"x": 100, "y": 200}
    popitem_dict2.popitem()
    assert len(popitem_dict2) == 1
    popitem_dict2.popitem()
    assert len(popitem_dict2) == 0

    # ----- Set operators (|, &, -, ^) and methods -----
    set_a: set[int] = {1, 2, 3}
    set_b: set[int] = {2, 3, 4}
    assert sorted(set_a | set_b) == [1, 2, 3, 4]
    assert sorted(set_a & set_b) == [2, 3]
    assert sorted(set_a - set_b) == [1]
    assert sorted(set_a ^ set_b) == [1, 4]
    assert sorted(set_a.union(set_b)) == [1, 2, 3, 4]
    assert sorted(set_a.intersection(set_b)) == [2, 3]
    assert sorted(set_a.difference(set_b)) == [1]
    assert sorted(set_a.symmetric_difference(set_b)) == [1, 4]
    assert set_a.issubset({1, 2, 3, 4}) == True
    assert set_a.issubset({1, 2}) == False
    assert set_a.issuperset({1, 2}) == True
    assert set_a.issuperset({1, 2, 3, 4}) == False
    assert set_a.isdisjoint({5, 6}) == True
    assert set_a.isdisjoint({2, 5}) == False

    # ----- set.update / *_update -----
    set_upd: set[int] = {1, 2, 3}
    set_upd.update({4, 5})
    assert sorted(set_upd) == [1, 2, 3, 4, 5]
    set_iu: set[int] = {1, 2, 3, 4}
    set_iu.intersection_update({2, 3, 5})
    assert sorted(set_iu) == [2, 3]
    set_du: set[int] = {1, 2, 3, 4}
    set_du.difference_update({2, 3, 5})
    assert sorted(set_du) == [1, 4]
    set_sdu: set[int] = {1, 2, 3}
    set_sdu.symmetric_difference_update({2, 3, 4})
    assert sorted(set_sdu) == [1, 4]

    # ----- dict.fromkeys() (instance form) -----
    dk_base: dict[str, int] = {}
    dk_result: dict[str, int] = dk_base.fromkeys(["a", "b", "c"], 0)
    assert dk_result["a"] == 0 and dk_result["b"] == 0 and dk_result["c"] == 0
    assert len(dk_result) == 3

    # ----- dict | and |= -----
    dm_merged: dict[str, int] = {"a": 1, "b": 2} | {"b": 3, "c": 4}
    assert dm_merged["a"] == 1
    assert dm_merged["b"] == 3
    assert dm_merged["c"] == 4
    assert len(dm_merged) == 3
    dm_aug: dict[str, int] = {"a": 1, "b": 2}
    dm_aug |= {"b": 3, "c": 4}
    assert dm_aug["a"] == 1 and dm_aug["b"] == 3 and dm_aug["c"] == 4
    assert len(dm_aug) == 3
    dm_alias_orig: dict[str, int] = {"x": 10}
    dm_alias_ref: dict[str, int] = dm_alias_orig
    dm_alias_orig |= {"y": 20}
    assert len(dm_alias_ref) == 2
    assert dm_alias_ref["y"] == 20
    dm_empty_rhs: dict[str, int] = {"a": 1}
    dm_empty_rhs |= {}
    assert len(dm_empty_rhs) == 1
    dm_empty_lhs: dict[str, int] = {}
    dm_empty_lhs |= {"a": 1}
    assert len(dm_empty_lhs) == 1

    # ----- bytes literals / construction / indexing / iteration / slicing -----
    assert len(b"hello") == 5
    assert len(bytes()) == 0
    assert bytes() == b""
    zeros: bytes = bytes(5)
    assert len(zeros) == 5
    for zi in range(5):
        assert zeros[zi] == 0
    assert bytes([65, 66, 67]) == b"ABC"
    assert len(bytes([65, 66, 67])) == 3
    assert bytes("hello", "utf-8") == b"hello"
    bytes_data: bytes = b"ABC"
    assert bytes_data[0] == 65
    assert bytes_data[-1] == 67
    bytes_iter_result: list[int] = []
    for bv in b"hello":
        bytes_iter_result.append(bv)
    assert bytes_iter_result == [104, 101, 108, 108, 111]
    assert b"hello"[1:4] == b"ell"

    def _get_bytes() -> bytes:
        return b"test"

    assert _get_bytes() == b"test"

    # ----- bytes.decode() -----
    assert b"hello".decode() == "hello"
    assert b"".decode() == ""
    assert b"hello world".decode("utf-8") == "hello world"

    # ----- bytes.startswith / endswith -----
    bse_data: bytes = b"hello world"
    assert bse_data.startswith(b"hello") == True
    assert bse_data.startswith(b"world") == False
    assert bse_data.endswith(b"world") == True
    assert bse_data.endswith(b"hello") == False
    assert b"".startswith(b"") == True
    assert b"".endswith(b"") == True

    # ----- bytes.find / rfind -----
    bf_data: bytes = b"hello world hello"
    assert bf_data.find(b"hello") == 0
    assert bf_data.find(b"xyz") == -1
    assert bf_data.rfind(b"hello") == 12
    assert bf_data.rfind(b"xyz") == -1

    # ----- bytes.count / replace -----
    assert b"abcabcabc".count(b"abc") == 3
    assert b"abcabcabc".count(b"xyz") == 0
    assert b"hello world".replace(b"world", b"python") == b"hello python"
    assert b"aaa".replace(b"a", b"bb") == b"bbbbbb"

    # ----- bytes.split / rsplit (sep + maxsplit + whitespace) -----
    bs_test1: list[bytes] = b"a,b,c".split(b",")
    assert bs_test1 == [b"a", b"b", b"c"]
    bs_test2: list[bytes] = b"a,b,c,d".rsplit(b",", 2)
    assert bs_test2 == [b"a,b", b"c", b"d"]
    assert b"a b c".split(None, 1) == [b"a", b"b c"]
    assert b"a b c".rsplit(None, 1) == [b"a b", b"c"]
    assert b"x y z w".split(None, 2) == [b"x", b"y", b"z w"]
    assert b"x y z w".rsplit(None, 2) == [b"x y", b"z", b"w"]
    assert b"a-b-c-d".split(b"-", 1) == [b"a", b"b-c-d"]
    assert b"a-b-c-d".rsplit(b"-", 1) == [b"a-b-c", b"d"]

    # ----- bytes.strip / lstrip / rstrip -----
    bst_data: bytes = b"  hello  "
    assert bst_data.strip() == b"hello"
    assert bst_data.lstrip() == b"hello  "
    assert bst_data.rstrip() == b"  hello"
    assert b"  hello world  ".strip() == b"hello world"

    # ----- bytes.upper / lower -----
    assert b"Hello World".upper() == b"HELLO WORLD"
    assert b"Hello World".lower() == b"hello world"

    # ----- bytes method return type inference (no annotations) -----
    brti_data = b"  Hello World  "
    assert brti_data.upper() == b"  HELLO WORLD  "
    assert brti_data.lower() == b"  hello world  "
    assert brti_data.strip() == b"Hello World"
    assert brti_data.lstrip() == b"Hello World  "
    assert brti_data.rstrip() == b"  Hello World"
    assert brti_data.replace(b"Hello", b"Hi") == b"  Hi World  "
    assert brti_data.find(b"Hello") == 2
    assert brti_data.count(b" ") == 5
    assert brti_data.startswith(b"  H") == True
    assert brti_data.endswith(b"  ") == True
    assert brti_data.decode() == "  Hello World  "

    # ----- bytes.join -----
    assert b",".join([b"a", b"b", b"c"]) == b"a,b,c"
    assert b"".join([b"x", b"y"]) == b"xy"
    assert b",".join((b"a", b"b")) == b"a,b"
    assert sorted(b",".join({b"a", b"b", b"c"}).split(b",")) == [b"a", b"b", b"c"]
    assert b",".join([b"a", b"bb", b"ccc"]) == b"a,bb,ccc"

    # ----- bytes as comparable / hashable list & set elements -----
    assert [b"a", b"b", b"c"] == [b"a", b"b", b"c"]
    assert [b"a", b"b"] != [b"a", b"c"]
    assert (b"x", b"y") == (b"x", b"y")
    assert sorted([b"c", b"a", b"b"]) == [b"a", b"b", b"c"]
    assert sorted([b"c", b"a", b"b"], reverse=True) == [b"c", b"b", b"a"]
    assert b"a" in [b"a", b"b"]
    bk1: bytes = b"a" + b""
    bk2: bytes = b"a" + b""
    assert bk1 == bk2
    assert len({bk1, bk2}) == 1
    byte_keyed: dict[bytes, int] = {}
    byte_keyed[bk1] = 1
    byte_keyed[bk2] = 2
    assert len(byte_keyed) == 1
    assert byte_keyed[b"a"] == 2
    assert b"a" in {b"a", b"b"}

    # ----- bytes concat / repeat -----
    assert b"hello" + b" world" == b"hello world"
    assert b"ab" * 3 == b"ababab"
    assert b"ab" + b"cd" == b"abcd"

    # ----- bytes as an iterable of ints (list/sum/min/max/sorted) -----
    bs = b"hello"
    assert list(bs) == [104, 101, 108, 108, 111]
    assert sum(bs) == 532
    assert min(bs) == 101 and max(bs) == 111
    assert sorted(bs) == [101, 104, 108, 108, 111]
    assert sum(b"hi") == 209
    assert sorted(b"dcba") == [97, 98, 99, 100]
    assert sum(bs, 1000) == 1532

    # ----- Dict with float keys -----
    float_dict: dict[float, str] = {1.5: "a", 2.5: "b", 3.5: "c"}
    assert len(float_dict) == 3
    assert float_dict[1.5] == "a"
    assert float_dict[2.5] == "b"
    assert float_dict[3.5] == "c"
    float_dict[1.5] = "updated"
    assert float_dict[1.5] == "updated"
    assert len(float_dict) == 3
    float_dict[4.5] = "d"
    assert len(float_dict) == 4
    assert 1.5 in float_dict and 2.5 in float_dict
    assert 9.9 not in float_dict
    float_key_sum: float = 0.0
    for fk in float_dict:
        float_key_sum = float_key_sum + fk
    assert float_key_sum == 12.0

    # ----- Dict with None key -----
    none_dict: dict[None, str] = {None: "null_value"}
    assert len(none_dict) == 1
    assert none_dict[None] == "null_value"
    assert None in none_dict
    none_dict[None] = "updated_null"
    assert none_dict[None] == "updated_null"
    assert len(none_dict) == 1

    # ----- Dict with tuple keys -----
    tuple_dict: dict[tuple[int, int], str] = {(1, 2): "pair_a", (3, 4): "pair_b"}
    assert len(tuple_dict) == 2
    assert tuple_dict[(1, 2)] == "pair_a"
    assert tuple_dict[(3, 4)] == "pair_b"
    assert (1, 2) in tuple_dict
    assert (3, 4) in tuple_dict
    assert (5, 6) not in tuple_dict
    tuple_dict[(1, 2)] = "updated_pair"
    assert tuple_dict[(1, 2)] == "updated_pair"
    assert len(tuple_dict) == 2
    tuple_dict[(5, 6)] = "pair_c"
    assert len(tuple_dict) == 3

    # ----- Set with float elements -----
    float_set: set[float] = {1.1, 2.2, 3.3}
    assert len(float_set) == 3
    assert 1.1 in float_set and 2.2 in float_set and 3.3 in float_set
    assert 4.4 not in float_set
    float_set.add(4.4)
    assert 4.4 in float_set
    assert len(float_set) == 4
    float_set.discard(2.2)
    assert 2.2 not in float_set
    assert len(float_set) == 3
    float_elem_count: int = 0
    for fe in float_set:
        float_elem_count = float_elem_count + 1
    assert float_elem_count == 3

    # ----- Set with None element -----
    none_set: set[None] = {None}
    assert len(none_set) == 1
    assert None in none_set
    none_set.add(None)
    assert len(none_set) == 1
    none_set.discard(None)
    assert None not in none_set
    assert len(none_set) == 0

    # ----- Set with tuple elements -----
    tuple_set: set[tuple[int, int]] = {(1, 2), (3, 4)}
    assert len(tuple_set) == 2
    assert (1, 2) in tuple_set
    assert (3, 4) in tuple_set
    assert (5, 6) not in tuple_set
    tuple_set.add((5, 6))
    assert (5, 6) in tuple_set
    assert len(tuple_set) == 3
    tuple_set.add((1, 2))
    assert len(tuple_set) == 3

    # ----- Dict insertion order -----
    order_dict: dict[str, int] = {}
    order_dict["c"] = 3
    order_dict["a"] = 1
    order_dict["b"] = 2
    assert list(order_dict.keys()) == ["c", "a", "b"]
    assert list(order_dict.values()) == [3, 1, 2]
    order_dict["a"] = 99
    assert list(order_dict.keys()) == ["c", "a", "b"]
    assert list(order_dict.values())[1] == 99
    del order_dict["a"]
    order_dict["a"] = 50
    assert list(order_dict.keys()) == ["c", "b", "a"]
    int_order_dict: dict[int, str] = {}
    int_order_dict[5] = "five"
    int_order_dict[1] = "one"
    int_order_dict[3] = "three"
    assert list(int_order_dict.values()) == ["five", "one", "three"]

    # ----- sorted(set(...)) -----
    assert sorted(set([3, 1, 4, 1, 5, 9, 2, 6])) == [1, 2, 3, 4, 5, 6, 9]
    assert sorted(set(["banana", "apple", "cherry"])) == ["apple", "banana", "cherry"]

    # ----- Structural == / != for dict and set -----
    assert {"a": 1, "b": 2} == {"b": 2, "a": 1}
    assert {"a": 1, "b": 2} != {"a": 1, "b": 9}
    assert {"k": [1, 2]} == {"k": [1, 2]}
    assert {"k": (1, 2)} != {"k": (1, 3)}
    assert {} == {}
    assert {"a": 1, "b": 2} != {"a": 1}
    assert {1, 2, 3} == {3, 2, 1}
    assert {1, 2, 3} != {1, 2, 9}
    assert set() == set()
    assert {1, 2, 3} != {1, 2}
    assert {(1, 2), (3, 4)} == {(3, 4), (1, 2)}
    _de_dyn = [{1: 2}, 0][0]
    assert _de_dyn == {1: 2}
    _se_dyn = [{7, 8}, 0][0]
    assert _se_dyn == {8, 7}

    # ----- Bytes ordering comparisons (lexicographic) -----
    assert (b"abc" < b"abd") is True
    assert (b"abc" > b"abd") is False
    assert (b"abc" <= b"abc") is True
    assert (b"abd" >= b"abc") is True

    # ----- min()/max() over bytes-element iterables -----
    _mmb_list: list[bytes] = [b"c", b"a", b"b"]
    assert min(_mmb_list) == b"a"
    assert max(_mmb_list) == b"c"
    _mmb_tuple: tuple[bytes, bytes, bytes] = (b"c", b"a", b"b")
    assert min(_mmb_tuple) == b"a"
    assert max(_mmb_tuple) == b"c"
    _mmb_set: set[bytes] = {b"c", b"a", b"b"}
    assert min(_mmb_set) == b"a"
    assert max(_mmb_set) == b"c"
    assert min(deque([b"c", b"a", b"b"])) == b"a"
    assert max(deque([b"c", b"a", b"b"])) == b"c"
    assert min(s for s in [b"yy", b"xx", b"zz"]) == b"xx"
    assert max(s for s in [b"yy", b"xx", b"zz"]) == b"zz"
    assert min(b"a", b"b") == b"a"
    assert max(b"a", b"b") == b"b"
    assert len(min(_mmb_list)) == 1


_fold_dict_set_bytes()


# =============================================================================
# FOLDED: p4_methods.py + p21_container_methods.py  (container methods)
# Print-based p4 converted to asserts via CPython output; exact dups with the
# list/tuple and dict/set folds dropped, distinctive cases (duplicates in count,
# word-count bucket, ValueError/KeyError raising, empty-set edges, interaction
# probes) kept.
# =============================================================================
def _fold_container_methods():
    # ----- list method chain (p4: incremental build, pop value, sort) -----
    xs = []
    for sq in range(5):
        xs.append(sq * sq)
    assert xs == [0, 1, 4, 9, 16]
    assert xs.pop() == 16
    assert xs == [0, 1, 4, 9]
    xs.insert(0, 99)
    assert xs == [99, 0, 1, 4, 9]
    xs.extend([7, 8])
    assert xs == [99, 0, 1, 4, 9, 7, 8]
    assert xs.index(99) == 0
    assert xs.count(4) == 1
    assert [1, 2, 2, 3, 2].count(2) == 3
    ys = xs.copy()
    ys.reverse()
    assert ys == [8, 7, 9, 4, 1, 0, 99]
    assert xs == [99, 0, 1, 4, 9, 7, 8]  # copy did not mutate original
    zs = [5, 3, 8, 1, 9, 2]
    zs.sort()
    assert zs == [1, 2, 3, 5, 8, 9]
    assert zs.pop(0) == 1
    assert zs == [2, 3, 5, 8, 9]
    zs.clear()
    assert zs == []
    assert len(zs) == 0

    # ----- dict methods (p4: get with default, sorted views, setdefault present) -----
    d = {"a": 1, "b": 2, "c": 3}
    assert d.get("b") == 2
    assert d.get("missing") is None
    assert d.get("missing", -1) == -1
    assert sorted(d.keys()) == ["a", "b", "c"]
    assert sorted(d.values()) == [1, 2, 3]
    assert sorted(d.items()) == [("a", 1), ("b", 2), ("c", 3)]
    assert d.pop("a") == 1
    assert sorted(d.keys()) == ["b", "c"]
    d.setdefault("z", 99)
    assert d.get("z") == 99
    d.setdefault("b", 999)  # already present; unchanged
    assert d.get("b") == 2
    d.update({"x": 10, "y": 20})
    assert d.get("x") == 10
    assert d.get("y") == 20
    dcopy = d.copy()
    dcopy.clear()
    assert len(dcopy) == 0
    assert len(d) == 5

    # ----- set methods (p4: discard absent no-error, copy independence) -----
    s = set()
    s.add(1)
    s.add(2)
    s.add(2)
    s.add(3)
    assert len(s) == 3
    s.discard(1)
    s.discard(99)  # absent — no error
    s.remove(2)
    assert len(s) == 1
    a = {1, 2, 3, 4}
    b = {3, 4, 5, 6}
    assert sorted(a.union(b)) == [1, 2, 3, 4, 5, 6]
    assert sorted(a.intersection(b)) == [3, 4]
    assert sorted(a.difference(b)) == [1, 2]
    c = a.copy()
    c.add(100)
    assert len(a) == 4
    assert len(c) == 5

    # ----- methods feeding loops / comprehensions (p4) -----
    acc = []
    d2 = {"k1": 1, "k2": 2, "k3": 3}
    for k in sorted(d2.keys()):
        acc.append(d2.get(k))
    assert acc == [1, 2, 3]
    bucket = {}
    for word in ["apple", "banana", "apple", "cherry", "banana", "apple"]:
        bucket[word] = bucket.get(word, 0) + 1
    assert sorted(bucket.items()) == [("apple", 3), ("banana", 2), ("cherry", 1)]

    # ----- tuple.index / count, incl. ValueError (p21) -----
    assert (1, 2, 3, 2).index(2) == 1
    assert (1, 2, 3, 2).count(2) == 2
    assert (5, 5, 5).count(5) == 3
    assert (1, 2, 1, 3).index(1) == 0
    assert ("a", "b", "c").index("c") == 2
    assert ("x", "y", "x").count("x") == 2
    nums_t = (10, 20, 30, 20, 10)
    assert nums_t.count(20) == 2
    assert nums_t.index(30) == 2
    miss_caught = False
    try:
        (1, 2, 3).index(99)
    except ValueError:
        miss_caught = True
    assert miss_caught

    # ----- set issubset/issuperset/isdisjoint with empty-set edges (p21) -----
    assert {1, 2}.issubset({1, 2, 3}) == True
    assert {1, 4}.issubset({1, 2, 3}) == False
    assert {1, 2, 3}.issuperset({1, 2}) == True
    assert {1, 2}.issuperset({1, 4}) == False
    assert {1, 2}.isdisjoint({3, 4}) == True
    assert {1, 2}.isdisjoint({2, 3}) == False
    assert set().issubset({1, 2}) == True
    assert {1, 2}.isdisjoint(set()) == True

    # ----- set *_update returning None + alias mutation (p21) -----
    s1 = {1, 2, 3, 4}
    r1 = s1.intersection_update({2, 3, 5})
    assert r1 is None
    assert sorted(s1) == [2, 3]
    assert len(s1) == 2
    s2 = {1, 2, 3, 4}
    s2.difference_update({2, 4})
    assert sorted(s2) == [1, 3]
    s3 = {1, 2, 3}
    s3.symmetric_difference_update({2, 3, 4})
    assert sorted(s3) == [1, 4]

    # ----- set.symmetric_difference new-set (operands unchanged) (p21) -----
    sa = {1, 2, 3}
    sb = {2, 3, 4}
    sd = sa.symmetric_difference(sb)
    assert sorted(sd) == [1, 4]
    assert sorted(sa) == [1, 2, 3]
    assert sorted(sb) == [2, 3, 4]

    # ----- list.remove returns None; ValueError on miss (p21) -----
    li = [10, 20, 30, 20]
    ret = li.remove(20)
    assert ret is None
    assert li == [10, 30, 20]
    li_str = ["a", "b", "c"]
    li_str.remove("b")
    assert li_str == ["a", "c"]
    rm_miss = False
    try:
        [1, 2, 3].remove(99)
    except ValueError:
        rm_miss = True
    assert rm_miss

    # ----- dict.popitem LIFO, 2-tuple subscript, empty→KeyError (p21) -----
    dp = {"a": 1, "b": 2, "c": 3}
    pk, pv = dp.popitem()
    assert pk == "c" and pv == 3
    assert sorted(dp.keys()) == ["a", "b"]
    assert len(dp) == 2
    dp2 = {"x": 10, "y": 20}
    item = dp2.popitem()
    assert item[0] == "y" and item[1] == 20
    dp3 = {"only": 42}
    ok, ov = dp3.popitem()
    assert ok == "only" and ov == 42
    empty_caught = False
    try:
        dp3.popitem()
    except KeyError:
        empty_caught = True
    assert empty_caught

    # ----- interaction probes (cross green features) (p21) -----
    src = (1, 1, 2, 3, 3, 3)
    counts = [src.count(x) for x in (1, 2, 3)]
    assert counts == [2, 1, 3]
    pool = {1, 2, 3, 4, 5}
    pool.intersection_update({2, 4, 6, 8})
    assert len(pool) == 2
    assert (2 in pool) == True and (6 in pool) == False
    labels = ("red", "green", "blue")
    picked = ["red", "green", "blue"][labels.index("green")]
    assert picked == "green"


_fold_container_methods()


# =============================================================================
# FOLDED: p35_counter.py  (collections.Counter — print-based converted to assert)
# Distinct from the target's Counter section: repr/most-common order, subscript
# read/write, update/subtract, truthiness, annotated funcs, word-frequency.
# =============================================================================
def _fold_counter():
    # construction + repr (most-common order)
    c = Counter("aabbbc")  # a:2, b:3, c:1
    assert str(c) == "Counter({'b': 3, 'a': 2, 'c': 1})"
    assert repr(c) == "Counter({'b': 3, 'a': 2, 'c': 1})"
    assert f"{c}" == "Counter({'b': 3, 'a': 2, 'c': 1})"
    assert str(Counter([1, 1, 2, 3, 3, 3])) == "Counter({3: 3, 1: 2, 2: 1})"
    assert str(Counter(("x", "y", "x"))) == "Counter({'x': 2, 'y': 1})"
    assert str(Counter()) == "Counter()"
    assert str(Counter(ch for ch in "hello")) == "Counter({'l': 2, 'h': 1, 'e': 1, 'o': 1})"

    # subscript read: present, missing→0 (no KeyError, no insert)
    assert c["a"] == 2
    assert c["b"] == 3
    assert c["x"] == 0
    before = len(c)
    _ = c["zzz"]
    assert len(c) == before

    # subscript write + augmented assignment
    c["a"] += 1  # 2 -> 3
    c["z"] += 5  # missing -> 0 -> 5
    c["c"] = 10
    assert c["a"] == 3 and c["z"] == 5 and c["c"] == 10

    # len / membership (count 0 key still a member)
    assert len(c) == 4  # a, b, c, z
    assert ("a" in c) == True and ("x" in c) == False
    c["zero"] = 0
    assert "zero" in c
    assert "x" not in c

    # iteration (keys), sorted, list, keys/values/items
    assert sorted(c) == ["a", "b", "c", "z", "zero"]
    assert sorted(list(c)) == ["a", "b", "c", "z", "zero"]
    total_keys = 0
    for _k in c:
        total_keys += 1
    assert total_keys == len(c)
    assert sorted(c.keys()) == ["a", "b", "c", "z", "zero"]
    assert sorted(c.values()) == [0, 3, 3, 5, 10]
    assert sorted(c.items()) == [("a", 3), ("b", 3), ("c", 10), ("z", 5), ("zero", 0)]
    doubled = {k: v * 2 for k, v in c.items()}
    assert doubled["z"] == 10
    assert sorted(doubled.items()) == [("a", 6), ("b", 6), ("c", 20), ("z", 10), ("zero", 0)]

    # most_common / total + n-arg edges
    c2 = Counter("aabbbcccc")  # a:2, b:3, c:4
    assert c2.most_common(2) == [("c", 4), ("b", 3)]
    assert c2.most_common() == [("c", 4), ("b", 3), ("a", 2)]
    assert c2.total() == 9
    assert c2.most_common(0) == []
    assert c2.most_common(-1) == []
    assert c2.most_common(100) == [("c", 4), ("b", 3), ("a", 2)]
    assert Counter("abab").most_common() == [("a", 2), ("b", 2)]  # tie keeps insertion order

    # update / subtract (negative counts allowed)
    cu = Counter()
    cu.update("aax")
    assert cu["a"] == 2 and cu["x"] == 1
    cu.update(["a", "y", "y"])
    assert cu["a"] == 3 and cu["y"] == 2
    cu.subtract("aaa")
    assert cu["a"] == 0
    neg = Counter("ab")
    neg.subtract("aabb")
    assert neg["a"] == -1 and neg["b"] == -1
    assert str(neg) == "Counter({'a': -1, 'b': -1})"

    # truthiness
    assert bool(Counter()) == False
    assert bool(Counter("a")) == True
    assert not Counter()

    # Counter through annotated functions
    def _total_count(counter: Counter) -> int:
        return counter.total()

    def _char_freq(text: str) -> Counter:
        return Counter(text)

    assert _total_count(Counter("hello")) == 5
    freq = _char_freq("mississippi")  # m:1, i:4, s:4, p:2
    assert freq.most_common(1) == [("i", 4)]
    assert freq["s"] == 4 and freq["m"] == 1 and freq["q"] == 0

    # word-frequency use
    words = "the quick brown fox the lazy dog the".split()
    wc = Counter(words)
    assert wc.most_common(1) == [("the", 3)]
    assert wc["the"] == 3 and wc["fox"] == 1 and wc["cat"] == 0


_fold_counter()


# =============================================================================
# FOLDED: p51_container_aug_ops.py  (set &=/-=/^=/|= aliasing, dict.fromkeys class form)
# =============================================================================
def _fold_container_aug_ops():
    # dict.fromkeys class form (bare dict receiver)
    df = dict.fromkeys(["a", "b", "c"], 0)
    assert sorted(df.items()) == [("a", 0), ("b", 0), ("c", 0)]
    e = dict.fromkeys(["x", "y"])
    assert sorted(e.keys()) == ["x", "y"]
    assert e["x"] is None
    assert sorted(dict.fromkeys(range(3), 9).items()) == [(0, 9), (1, 9), (2, 9)]
    assert sorted(dict.fromkeys((1, 2), "v").items()) == [(1, "v"), (2, "v")]
    # Value aliasing: one shared value object across all keys
    shared = dict.fromkeys([1, 2, 3], [])
    shared[1].append("z")
    assert shared[2] == ["z"]
    assert (shared[1] is shared[2]) is True
    assert (shared[2] is shared[3]) is True

    # set &= with alias witness
    s = {1, 2, 3, 4}
    t = s
    s &= {2, 3, 5}
    assert t is s
    assert sorted(s) == [2, 3]
    # set -= with alias witness
    a = {1, 2, 3, 4}
    b = a
    a -= {2, 4}
    assert b is a
    assert sorted(a) == [1, 3]
    # set ^= with alias witness
    cc = {1, 2, 3}
    g = cc
    cc ^= {2, 3, 4}
    assert g is cc
    assert sorted(cc) == [1, 4]
    # |= regression (already in-place)
    p = {1, 2}
    q = p
    p |= {3, 4}
    assert q is p
    assert sorted(q) == [1, 2, 3, 4]

    # numeric augmented ops still produce new values (not set ops)
    n = 10
    n -= 3
    assert n == 7
    x = 6
    x &= 3
    assert x == 2
    y = 6
    y ^= 3
    assert y == 5
    z = 12
    z |= 1
    assert z == 13

    # aug ops on subscript / attribute set targets
    holder = {"s": {1, 2, 3}}
    holder["s"] &= {2, 3, 4}
    assert sorted(holder["s"]) == [2, 3]
    box = _AugBox()
    alias = box.s
    box.s -= {1, 2}
    assert sorted(box.s) == [3, 4]
    assert (alias is box.s) is True


_fold_container_aug_ops()


# From p15_tuple_slice_slot.py: a tuple slice `t[a:b]` stored into an annotated
# fixed-arity `tuple[T, ...]` LOCAL slot. Slicing yields a variable-length
# `tuple[T, ...]`, but a fixed-arity annotation is a CPython-legal (never
# arity-enforced) slot for it. The repr-contract admits this `tuple` -> `tuple`
# store when every element's `Repr` matches per index; `len()` reflects the real
# runtime length, not the annotated arity. Probes three element-repr families
# (Tagged `int`, `Raw(F64)` `float`, `Heap(Str)`) and crosses the slice-into-slot
# with iteration and nested/flat unpacking. The annotated-slot locals MUST stay
# annotated — they are exactly what is under test.
def _fold_p15_tuple_slice_slot():
    # ===== Tagged int elements =====
    nums: tuple[int, int, int, int, int, int] = (0, 1, 2, 3, 4, 5)

    s_mid: tuple[int, int, int, int, int, int] = nums[1:4]
    assert len(s_mid) == 3
    assert s_mid[0] == 1
    assert s_mid[1] == 2
    assert s_mid[2] == 3

    s_head: tuple[int, int, int, int, int, int] = nums[:3]
    assert len(s_head) == 3
    assert s_head[0] == 0
    assert s_head[2] == 2

    s_tail: tuple[int, int, int, int, int, int] = nums[3:]
    assert len(s_tail) == 3
    assert s_tail[0] == 3
    assert s_tail[2] == 5

    s_full: tuple[int, int, int, int, int, int] = nums[:]
    assert len(s_full) == 6
    assert s_full[0] == 0
    assert s_full[5] == 5

    s_step: tuple[int, int, int, int, int, int] = nums[::2]
    assert len(s_step) == 3
    assert s_step[0] == 0
    assert s_step[1] == 2
    assert s_step[2] == 4

    s_neg: tuple[int, int, int, int, int, int] = nums[-2:]
    assert len(s_neg) == 2
    assert s_neg[0] == 4
    assert s_neg[1] == 5

    s_rev: tuple[int, int, int, int, int, int] = nums[::-1]
    assert len(s_rev) == 6
    assert s_rev[0] == 5
    assert s_rev[5] == 0

    # ===== Raw(F64) float elements =====
    fs: tuple[float, float, float, float] = (1.0, 2.0, 3.0, 4.0)
    fslice: tuple[float, float, float, float] = fs[1:3]
    assert len(fslice) == 2
    assert fslice[0] == 2.0
    assert fslice[1] == 3.0

    # ===== Heap(Str) elements =====
    ws: tuple[str, str, str] = ("alpha", "beta", "gamma")
    wslice: tuple[str, str, str] = ws[0:2]
    assert len(wslice) == 2
    assert wslice[0] == "alpha"
    assert wslice[1] == "beta"

    # ===== Interaction: iterate a slice stored in an annotated slot =====
    total = 0
    for v in s_mid:
        total += v
    assert total == 6

    # ===== Interaction: unpack a slice stored in an annotated slot =====
    u: tuple[int, int, int, int, int, int] = nums[2:5]
    a, b, c = u
    assert a == 2
    assert b == 3
    assert c == 4

    # Nested unpacking from a sliced-into-slot tuple paired with a literal.
    p, (q, r) = u[0], (u[1], u[2])
    assert p == 2
    assert q == 3
    assert r == 4


_fold_p15_tuple_slice_slot()


# ===== Priority 1: starred displays, {**a,**b} merge, slice assign/del =====
def _fold_p1_displays_slices():
    # --- Feature A: `*` in list / set / tuple displays ---
    a = [1, 2, 3]
    assert [*a, 4, *a] == [1, 2, 3, 4, 1, 2, 3]
    assert [0, *a] == [0, 1, 2, 3]
    assert [*range(3), *"ab"] == [0, 1, 2, "a", "b"]
    # set display spread (order-independent → compare sorted)
    sset = {*a, *[3, 4, 5]}
    assert sorted(sset) == [1, 2, 3, 4, 5]
    # tuple display spread
    assert (*a, 99) == (1, 2, 3, 99)
    assert (0, *a, 4) == (0, 1, 2, 3, 4)
    # starred-tuple as a value
    x = *a, 0
    assert x == (1, 2, 3, 0)

    # --- Feature B: `{**a, **b}` dict merge ---
    d1 = {"x": 1, "y": 2}
    d2 = {"y": 20, "z": 30}
    assert {**d1, **d2} == {"x": 1, "y": 20, "z": 30}
    # later keys (literal + spread) override earlier ones, left-to-right
    assert {**d1, "w": 99, **d2} == {"x": 1, "y": 20, "w": 99, "z": 30}
    assert {**d1, "y": 7} == {"x": 1, "y": 7}

    # --- dict.update with keyword arguments ---
    du = {"a": 1}
    kw = {"b": 2, "c": 3}
    du.update(**kw)  # `**` spread (via rt_obj_method)
    assert du == {"a": 1, "b": 2, "c": 3}
    du = {"x": 0}
    du.update({"y": 9}, z=7, **{"w": 5})  # positional + named + spread
    assert du == {"x": 0, "y": 9, "z": 7, "w": 5}
    du = {"a": 1}
    du.update(a=10, b=20)  # plain named kwargs (no `**`)
    assert du == {"a": 10, "b": 20}
    du = {}
    du.update({"p": 1}, q=2)  # positional mapping + named
    assert du == {"p": 1, "q": 2}
    du = {"k": 1}
    du.update()  # zero-argument no-op
    assert du == {"k": 1}
    du = {}
    assert du.update(a=1) is None  # returns None
    assert du == {"a": 1}

    # --- Feature D: slice assignment ---
    li = [0, 1, 2, 3, 4, 5]
    li[1:3] = [9, 8]  # equal length
    assert li == [0, 9, 8, 3, 4, 5]
    li = [0, 1, 2, 3, 4, 5]
    li[1:3] = [100]  # shrink
    assert li == [0, 100, 3, 4, 5]
    li = [0, 1, 2, 3, 4, 5]
    li[1:3] = [7, 7, 7]  # grow
    assert li == [0, 7, 7, 7, 3, 4, 5]
    li = [0, 1, 2, 3, 4, 5]
    li[-3:] = [88]  # negative bound
    assert li == [0, 1, 2, 88]
    li = [1, 2, 3, 4]
    li[:] = []  # full clear
    assert li == []
    li = [1, 2, 3]
    li[1:1] = [9, 9]  # insert at empty region
    assert li == [1, 9, 9, 2, 3]
    li = [1, 2, 3]
    li[1:2] = li  # self-alias snapshot
    assert li == [1, 1, 2, 3, 3]
    # extended slice (step != 1)
    li = [0, 1, 2, 3, 4, 5, 6, 7]
    li[::2] = [10, 20, 30, 40]
    assert li == [10, 1, 20, 3, 30, 5, 40, 7]
    li = [0, 1, 2, 3, 4, 5]
    li[::-1] = [10, 11, 12, 13, 14, 15]  # negative step
    assert li == [15, 14, 13, 12, 11, 10]
    # extended-slice size mismatch → ValueError
    raised = False
    try:
        bad = [0, 1, 2, 3]
        bad[::2] = [1, 2, 3]
    except ValueError:
        raised = True
    assert raised

    # --- Feature D: slice deletion ---
    li = [0, 1, 2, 3, 4, 5]
    del li[1:3]
    assert li == [0, 3, 4, 5]
    li = [0, 1, 2, 3, 4, 5, 6, 7]
    del li[::2]  # extended del
    assert li == [1, 3, 5, 7]
    li = [0, 1, 2, 3, 4, 5, 6, 7]
    del li[::-2]  # negative-step del
    assert li == [0, 2, 4, 6]
    li = [1, 2, 3]
    del li[2:1]  # empty slice → no-op
    assert li == [1, 2, 3]
    li = [0, 1, 2, 3, 4]
    del li[:]  # delete all
    assert li == []


_fold_p1_displays_slices()

print("All collections tests passed!")
