# Scalar / value builtins (PLAN §5): pow, divmod, all, any, id, round, bin,
# hex, oct.
#
# MECHANISM: recognized by NAME in the frontend before the undefined-name error
# (like sum/min/max/set/next), gated on the name being UNSHADOWED. Two shapes:
#   - pure desugar (zero new runtime): pow → `**` (BinOp::Pow), divmod → staged
#     `(a // b, a % b)`, all/any → an iterator loop with truthiness short-circuit;
#   - declarative runtime call (StdlibFunctionDef + rt_*): id (wraps rt_id_obj),
#     round (rt_builtin_round, banker's), bin/hex/oct (rt_builtin_bin/hex/oct,
#     BIGNUM-AWARE — tagged Value in, NOT a raw i64 — PITFALLS B16).
#
# The `==` / bignum / banker's / sign assertions are the spec (Principle 9); the
# bignum bin/hex prints are the B16 differential gate. Out of scope (unprobed):
# 1-arg / 3-arg modular pow, map/filter (A4), format/ascii.

# ===== pow → `**` (bignum + numeric-tower correct) =====
assert pow(2, 3) == 8
assert pow(2, 10) == 1024
assert pow(5, 0) == 1
assert pow(2, -1) == 0.5  # negative exponent → float, exactly like `**`
assert pow(2, 64) == 2 ** 64  # bignum result
assert pow(10, 20) == 10 ** 20

# ===== divmod → (a // b, a % b), CPython floor/sign semantics (B1) =====
assert divmod(17, 5) == (3, 2)
assert divmod(-7, 2) == (-4, 1)
assert divmod(7, -2) == (-4, -1)
assert divmod(-7, -2) == (3, -1)
assert divmod(7.5, 2) == (3.0, 1.5)
assert divmod(20, 4) == (5, 0)

# ===== all / any — list, genexpr, empty, short-circuit, mixed, range =====
assert all([True, True, True]) == True
assert all([True, False, True]) == False
assert all([]) == True  # empty → seed
assert any([False, False, True]) == True
assert any([False, False, False]) == False
assert any([]) == False  # empty → seed
assert all([1, 2, 3]) == True  # truthy non-bools
assert all([1, 0, 3]) == False  # 0 is falsy
assert any([0, 0, 5]) == True
assert all(x > 0 for x in [1, 2, 3]) == True  # generator comprehension
assert any(x > 2 for x in [1, 2, 3]) == True
assert all(x < 0 for x in [1, 2, 3]) == False
assert all(x < 5 for x in range(5)) == True  # over range
assert any(x == 3 for x in range(5)) == True
assert all(["a", "b"]) == True  # non-empty strings truthy
assert all(["a", "", "b"]) == False  # empty string falsy

# short-circuit witness: a falsy early element stops `all` before a later truthy
witness = []


def tap(v):
    witness.append(v)
    return v


assert all(tap(x) for x in [1, 0, 1]) == False
assert witness == [1, 0]  # stopped at the first falsy, never saw the trailing 1

# ===== id — stability, distinctness, consistency with §2 `is` =====
id_x = [1, 2, 3]
assert id(id_x) == id(id_x)  # stable across calls
a = [1]
b = [1]
assert id(a) != id(b)  # distinct live objects have distinct ids
assert (a is b) == (id(a) == id(b))  # consistent with `is`
assert (a is a) == (id(a) == id(a))

# ===== round — banker's (round-half-to-even, B1) =====
assert round(2.5) == 2  # half → even (down)
assert round(3.5) == 4  # half → even (up)
assert round(0.5) == 0
assert round(-0.5) == 0  # -0.0 → 0
assert round(1.5) == 2
assert round(2.675, 2) == 2.67  # 2.675 is 2.6749999… as a double
assert round(3.14159, 2) == 3.14  # ndigits present → float result
assert round(7.5 / 2.5) == 3  # 3.0 → int 3
assert round(5) == 5  # int stays int
assert round(123.456, 1) == 123.5

# ===== bin / hex / oct — bignum-aware (B16) =====
assert bin(10) == "0b1010"
assert bin(0) == "0b0"
assert bin(-5) == "-0b101"  # sign before the prefix
assert hex(255) == "0xff"
assert hex(-255) == "-0xff"
assert oct(8) == "0o10"
assert oct(-8) == "-0o10"
assert bin(True) == "0b1"  # bool formats as its int value
assert hex(False) == "0x0"
assert bin(2 ** 100) == "0b1" + "0" * 100  # bignum (B16)

# ===== interaction probes (cross with green features) =====
assert f"{divmod(17, 5)}" == "(3, 2)"  # f-string of a tuple
assert f"{bin(10)}" == "0b1010"
assert f"round={round(3.14159, 2)}" == "round=3.14"
assert pow(2, 3) + round(1.5) == 10  # 8 + 2
q, r = divmod(17, 5)  # unpack a divmod result
assert q == 3
assert r == 2
assert bin(10) + " " + hex(255) == "0b1010 0xff"

# ===== print directly — the differential gate for concrete values (incl. B16) ==
print(pow(2, 10))
print(pow(2, 64))
print(divmod(17, 5))
print(divmod(-7, 2))
print(divmod(7.5, 2))
print(all([1, 2, 3]))
print(any([0, 0, 0]))
print(round(2.5))
print(round(3.14159, 2))
print(bin(10))
print(hex(255))
print(oct(8))
print(bin(-5))
print(bin(2 ** 100))
print(hex(2 ** 100))

print("scalar builtins tests passed")
