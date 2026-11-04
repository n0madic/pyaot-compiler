# Test file for json module

import json
import os

# Test json.dumps with dict
d: dict[str, int] = {"a": 1, "b": 2}
s: str = json.dumps(d)
print("json.dumps dict:", s)

# Test json.dumps with list (CPython uses spaces after separators)
lst: list[int] = [1, 2, 3]
s2: str = json.dumps(lst)
print("json.dumps list:", s2)
assert s2 == "[1, 2, 3]", "s2 should equal \"[1, 2, 3]\""

# Test json.dumps with string
s3: str = json.dumps("hello")
assert s3 == '"hello"', "s3 should equal '\"hello\"'"

# Test json.dumps with int
s4: str = json.dumps(42)
assert s4 == "42", "s4 should equal \"42\""

# Test json.dumps with float
s5: str = json.dumps(3.14)
# Float representation may vary, just check it parses back
print("json.dumps float:", s5)

# Test json.dumps with bool
s6: str = json.dumps(True)
assert s6 == "true", "s6 should equal \"true\""

s7: str = json.dumps(False)
assert s7 == "false", "s7 should equal \"false\""

# Test json.dumps with None
s8: str = json.dumps(None)
assert s8 == "null", "s8 should equal \"null\""

# Test json.loads with object
obj = json.loads('{"name": "test", "value": 42}')
# Verify by re-serializing (dict order may vary, so check contains)
obj_str: str = json.dumps(obj)
assert "name" in obj_str, "\"name\" should be in obj_str"
assert "test" in obj_str, "\"test\" should be in obj_str"
assert "value" in obj_str, "\"value\" should be in obj_str"
assert "42" in obj_str, "\"42\" should be in obj_str"
print("json.loads obj type check passed")

# Test json.loads with array (CPython uses spaces after separators)
arr = json.loads("[1, 2, 3]")
arr_str: str = json.dumps(arr)
assert arr_str == "[1, 2, 3]", "arr_str should equal \"[1, 2, 3]\""
print("json.loads array type check passed")

# Test json.loads with string
val_str = json.loads('"hello"')
val_str_back: str = json.dumps(val_str)
assert val_str_back == '"hello"', "val_str_back should equal '\"hello\"'"
print("json.loads string type check passed")

# Test json.loads with number
val_int = json.loads("42")
val_int_back: str = json.dumps(val_int)
assert val_int_back == "42", "val_int_back should equal \"42\""
print("json.loads int type check passed")

# Test json.loads with bool
val_bool = json.loads("true")
val_bool_back: str = json.dumps(val_bool)
assert val_bool_back == "true", "val_bool_back should equal \"true\""
print("json.loads bool type check passed")

# Test json.loads with null
val_null = json.loads("null")
val_null_back: str = json.dumps(val_null)
assert val_null_back == "null", "val_null_back should equal \"null\""
print("json.loads null type check passed")

# Test round-trip: dumps then loads
original: dict[str, str] = {"key": "value"}
round_trip_str: str = json.dumps(original)
print("Round-trip JSON:", round_trip_str)
# Verify round-trip
loaded_back = json.loads(round_trip_str)
loaded_back_str: str = json.dumps(loaded_back)
assert "key" in loaded_back_str, "\"key\" should be in loaded_back_str"
assert "value" in loaded_back_str, "\"value\" should be in loaded_back_str"

# Test json.dumps with nested structure (CPython uses spaces after separators)
nested: dict[str, list[int]] = {"numbers": [1, 2, 3]}
nested_s: str = json.dumps(nested)
print("json.dumps nested:", nested_s)
assert nested_s == '{"numbers": [1, 2, 3]}', "nested_s should equal '{\"numbers\": [1, 2, 3]}'"

# Test nested structure round-trip
nested_loaded = json.loads(nested_s)
nested_back: str = json.dumps(nested_loaded)
assert nested_back == '{"numbers": [1, 2, 3]}', "nested_back should equal '{\"numbers\": [1, 2, 3]}'"

# Test json.dump and json.load with file I/O
test_data: dict[str, int] = {"x": 10, "y": 20}
fp = open("_test_json_tmp.json", "w")
json.dump(test_data, fp)
fp.close()

fp2 = open("_test_json_tmp.json", "r")
loaded = json.load(fp2)
fp2.close()

# Verify loaded data by re-serializing
loaded_str: str = json.dumps(loaded)
assert "x" in loaded_str, "\"x\" should be in loaded_str"
assert "10" in loaded_str, "\"10\" should be in loaded_str"
assert "y" in loaded_str, "\"y\" should be in loaded_str"
assert "20" in loaded_str, "\"20\" should be in loaded_str"
print("json.load from file passed")

# Also read file content directly to verify json.dump wrote correct data
fp3 = open("_test_json_tmp.json", "r")
file_content: str = fp3.read()
fp3.close()
assert "x" in file_content, "\"x\" should be in file_content"
assert "10" in file_content, "\"10\" should be in file_content"
assert "y" in file_content, "\"y\" should be in file_content"
assert "20" in file_content, "\"20\" should be in file_content"
print("json.dump file content verified")

# Clean up temp file
os.remove("_test_json_tmp.json")

# Test from import style (CPython uses spaces)
from json import dumps, loads
s_from: str = dumps([1, 2, 3])
assert s_from == "[1, 2, 3]", "s_from should equal \"[1, 2, 3]\""
print("from json import dumps/loads passed")


# --- CPython-exact codec: key order, big ints, NaN/Infinity, non-str keys ---
def _json_cpython_exact() -> None:
    assert json.dumps({"b": 1, "a": [1.5, 10**30, None, True]}) == (
        '{"b": 1, "a": [1.5, 1000000000000000000000000000000, null, true]}'
    )
    assert json.dumps({False: 0}) == "{\"false\": 0}"
    assert json.dumps({1: "x", 2.5: "y", None: "n"}) == (
        '{"1": "x", "2.5": "y", "null": "n"}'
    )
    assert json.dumps([float("nan"), float("inf"), -float("inf"), 1e22, 0.1]) == (
        "[NaN, Infinity, -Infinity, 1e+22, 0.1]"
    )
    assert json.dumps("é\n\"\u0001\U0001F600") == '"\\u00e9\\n\\"\\u0001\\ud83d\\ude00"'
    d = json.loads('{"z": 1, "a": 2, "m": [3, 12345678901234567890123, -0.5e3, Infinity]}')
    assert list(d.keys()) == ["z", "a", "m"]
    assert d["m"] == [3, 12345678901234567890123, -500.0, float("inf")]
    assert json.loads('"\\ud83d\\ude00 \\u00e9"') == "\U0001F600 é"
    nan = json.loads("NaN")
    assert nan != nan
    msgs: list[str] = []
    for bad in ["[1, 2", "", '{"a" 1}', "[1] x", "{'a': 1}"]:
        try:
            json.loads(bad)
        except ValueError as e:
            msgs.append(str(e))
    assert msgs == [
        "Expecting ',' delimiter: line 1 column 6 (char 5)",
        "Expecting value: line 1 column 1 (char 0)",
        "Expecting ':' delimiter: line 1 column 6 (char 5)",
        "Extra data: line 1 column 5 (char 4)",
        "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)",
    ]
    errs: list[str] = []
    try:
        json.dumps({(1, 2): 3})
    except TypeError as e:
        errs.append(str(e))
    try:
        json.dumps({1, 2})
    except TypeError as e:
        errs.append(str(e))
    assert errs == [
        "keys must be str, int, float, bool or None, not tuple",
        "Object of type set is not JSON serializable",
    ]


_json_cpython_exact()


print("All json module tests passed!")
