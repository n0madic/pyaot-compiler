# §9 bytes methods (runtime-ready batch). Wires the bytes method surface whose
# runtime impls AND core-defs descriptors already existed but were not dispatched:
# startswith/endswith, find/rfind, count, replace, split/rsplit, strip/lstrip/
# rstrip, upper/lower, join — plus the pre-existing decode.
#
# MECHANISM: a bytes receiver routes to `lower_bytes_method`, the exact sibling of
# `lower_str_method` — a declarative `BytesPlan` table (descriptor + per-arg reprs
# + return spec) → the shared `emit_seq_method`; typeck types the result in
# `method_call_ty`'s `SemTy::Bytes` arm. No codegen edit — the runtime fn is
# resolved by symbol. The numeric `maxsplit` rides a RAW i64 slot (B16), never a
# tagged int misread as a count. find/rfind use dedicated 2-arg runtime fns (no
# op_tag, unlike str's shared search).
#
# The `==` assertions are the spec (Principle 9); non-ASCII byte content
# (b"\xc3\xa9" = "é" in UTF-8, valid UTF-8 so not the §14 gap) exercises the
# byte-accurate (non-codepoint) paths. SCOPE LIMITS (all UNPROBED): positional-
# only (kwargs rejected); `replace` has no `count`; the strip family takes NO
# `chars` (whitespace only); `find`/`rfind` take no `start`/`end`; `decode`
# ignores its encoding (always UTF-8); upper/lower are ASCII-only (non-ASCII
# bytes pass through, matching CPython).

# ===== startswith / endswith =====
assert b"hello".startswith(b"he") == True
assert b"hello".startswith(b"lo") == False
assert b"hello".endswith(b"lo") == True
assert b"hello".endswith(b"he") == False
assert b"caf\xc3\xa9".startswith(b"caf") == True
print("starts/ends:", b"hello".startswith(b"he"), b"hello".endswith(b"lo"))

# ===== find / rfind (dedicated 2-arg fns, byte offsets) =====
assert b"abcabc".find(b"b") == 1
assert b"abcabc".rfind(b"b") == 4
assert b"abcabc".find(b"z") == -1
assert b"caf\xc3\xa9".find(b"\xc3\xa9") == 3
print("find/rfind:", b"abcabc".find(b"b"), b"abcabc".rfind(b"b"), b"abcabc".find(b"z"))

# ===== count =====
assert b"abcabc".count(b"a") == 2
assert b"aaaa".count(b"aa") == 2
assert b"abc".count(b"z") == 0
print("count:", b"abcabc".count(b"a"), b"aaaa".count(b"aa"))

# ===== replace (2-arg, no count) =====
assert b"a,b,c".replace(b",", b";") == b"a;b;c"
assert b"aaa".replace(b"a", b"bb") == b"bbbbbb"
assert b"caf\xc3\xa9".replace(b"\xc3\xa9", b"e") == b"cafe"
print("replace:", b"a,b,c".replace(b",", b";"))

# ===== split: whitespace / sep / maxsplit (RAW slot) / explicit None =====
assert b"a,b,c".split(b",") == [b"a", b"b", b"c"]
assert b"  a b  c  ".split() == [b"a", b"b", b"c"]
assert b"a,b,c,d".split(b",", 1) == [b"a", b"b,c,d"]
assert b"a b c".split(None) == [b"a", b"b", b"c"]
assert b"".split() == []
print("split:", b"a,b,c".split(b","), b"a,b,c,d".split(b",", 1))
print("split ws:", b"  a b  c  ".split())

# ===== rsplit: sep+maxsplit / no-arg whitespace =====
assert b"a-b-c".rsplit(b"-", 1) == [b"a-b", b"c"]
assert b"one two three".rsplit() == [b"one", b"two", b"three"]
assert b"a b c".rsplit(None, 1) == [b"a b", b"c"]
print("rsplit:", b"a-b-c".rsplit(b"-", 1), b"a b c".rsplit(None, 1))

# ===== strip / lstrip / rstrip (whitespace only — no chars) =====
assert b"  hi  ".strip() == b"hi"
assert b"  hi  ".lstrip() == b"hi  "
assert b"  hi  ".rstrip() == b"  hi"
assert b"\t\n x \r\n".strip() == b"x"
print("strip:", b"  hi  ".strip(), b"  hi  ".lstrip() + b"|", b"|" + b"  hi  ".rstrip())

# ===== upper / lower (ASCII-only; non-ASCII bytes pass through) =====
assert b"Hello".upper() == b"HELLO"
assert b"Hello".lower() == b"hello"
assert b"caf\xc3\xa9".upper() == b"CAF\xc3\xa9"
print("upper/lower:", b"Hello".upper(), b"Hello".lower())

# ===== join (materializes the iterable, like str.join) =====
assert b",".join([b"a", b"b", b"c"]) == b"a,b,c"
assert b"".join([b"x", b"y", b"z"]) == b"xyz"
assert b"-".join([b"solo"]) == b"solo"
print("join:", b",".join([b"a", b"b", b"c"]))

# ===== decode round-trip with str.encode (§9-str) =====
assert b"caf\xc3\xa9".decode() == "café"
assert "café".encode() == b"caf\xc3\xa9"
assert "café".encode().decode() == "café"
assert b"abc".decode() == "abc"
assert b"abc".decode("utf-8") == "abc"
print("decode:", b"caf\xc3\xa9".decode(), "round", "café".encode().decode())

# ===== interaction probes (cross green features; bytes NOT in an f-string) =====
# len() + iteration (bytes iterates to ints)
assert len(b"hello") == 5
total = 0
for byte in b"abc":
    total += byte
assert total == 97 + 98 + 99
# split result feeds a for-loop + len()
parts = b"10,20,30".split(b",")
assert len(parts) == 3
acc = 0
for p in parts:
    acc += len(p)
assert acc == 6
# join over the result of a split round-trips
assert b",".join(b"a,b,c".split(b",")) == b"a,b,c"
# membership: `bytes in bytes` (subsequence search) + `int in bytes` (byte value),
# cross-checked against find/count
assert (b"ana" in b"banana") == True
assert (b"xyz" in b"banana") == False
assert (b"" in b"banana") == True
assert (98 in b"abc") == True  # 98 == ord('b')
assert (b"banana".find(b"a") != -1) == True
assert b"banana".count(b"a") == 3
print("interaction:", len(b"hello"), total, parts, acc, b"ana" in b"banana")

print("All bytes-method tests passed!")
