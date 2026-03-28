# Test collections module: defaultdict, Counter, deque, OrderedDict
from collections import defaultdict, Counter, deque, OrderedDict

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

print("All collections tests passed!")
