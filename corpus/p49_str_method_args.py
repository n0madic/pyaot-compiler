# §9 str/bytes method arguments closed: `replace` count, `find`/`index`/`rfind`/
# `rindex` start/end, and encoding-honoring `encode`/`decode`.
#
# `count` and the search `start`/`end` bounds ride RAW i64 slots (B16: a count
# rides a machine register, never a tagged int misread as a width). The search
# bounds are codepoint offsets with CPython slice clamping (negatives add the
# length, start floored at 0, end capped at the length); the returned index is
# absolute. `index`/`rindex` raise `ValueError` on a miss. `encode`/`decode`
# honor utf-8/ascii/latin-1 and raise `UnicodeEncodeError`/`UnicodeDecodeError`
# (subclasses of `ValueError`) on an out-of-range codepoint/byte, or
# `LookupError` for an unknown encoding name; the error paths print fixed strings
# so the exact runtime message is divergence-safe (both runtimes raise).


# ===== replace count (str) =====
print("aaaa".replace("a", "b", 2))
print("aaaa".replace("a", "b"))
print("aaaa".replace("a", "b", 0))
print("aaaa".replace("a", "b", 100))
print("abc".replace("", "X", 2))
print("abc".replace("", "X"))
print("one two one two one".replace("one", "1", 1))
print("café cafē".replace("caf", "C", 1))

# ===== replace count (bytes) =====
print(b"aaaa".replace(b"a", b"b", 2))
print(b"aaaa".replace(b"a", b"b"))
print(b"xyxyxy".replace(b"xy", b"Z", 2))

# ===== find / rfind / index / rindex with start / end (str) =====
s = "abcabcabc"
print(s.find("bc", 2))
print(s.find("bc", 2, 4))
print(s.find("bc", 2, 5))
print(s.find("abc", -3))
print(s.rfind("bc", 0, 4))
print(s.rfind("bc"))
print(s.rfind("bc", 0, -1))
print(s.index("bc", 3))
print(s.rindex("bc", 0, 7))
print(s.find("zzz"))
print(s.find("bc", 100))
print("café".find("é"))
print("café".find("é", 2))
print("café".find("f", 0, 3))
print("café".find("f", 0, 2))
print("naïve naïve".find("ï", 4))


def index_miss(label, fn):
    try:
        fn()
        print("ERROR:", label, "no raise")
    except ValueError:
        print("caught", label, "ValueError")


index_miss("index", lambda: "abc".index("z", 0, 2))
index_miss("rindex", lambda: "abcabc".rindex("z"))
index_miss("index-bound", lambda: "abcabc".index("bc", 0, 2))

# ===== find / rfind with start / end (bytes) =====
bs = b"abcabc"
print(bs.find(b"bc", 2))
print(bs.find(b"bc", 2, 4))
print(bs.find(b"bc", -4))
print(bs.rfind(b"bc", 0, 4))
print(bs.rfind(b"bc"))
print(bs.count(b"bc"))

# ===== encode / decode correct paths =====
print("café".encode("utf-8"))
print("café".encode("UTF_8"))
print("hello".encode("ascii"))
print("café".encode("latin-1"))
print("café".encode("latin1"))
print(b"caf\xc3\xa9".decode("utf-8"))
print(b"hello".decode("ascii"))
print(b"caf\xe9".decode("latin-1"))
print(b"caf\xe9".decode("iso-8859-1"))
print("Ωmega".encode("utf-8").decode("utf-8"))


# ===== encode / decode error paths (fixed strings, divergence-safe) =====
# Precise type AND a super-catch through the MRO (UnicodeError ⊂ ValueError).
try:
    "café".encode("ascii")
except UnicodeError:
    print("caught enc-ascii UnicodeError")
try:
    "café".encode("ascii")
except ValueError:
    print("caught enc-ascii ValueError")
try:
    "€".encode("latin-1")
except UnicodeError:
    print("caught enc-latin1 UnicodeError")
try:
    "x".encode("zzz-codec")
except LookupError:
    print("caught enc-unknown LookupError")
try:
    b"\xff\xfe".decode("utf-8")
except UnicodeDecodeError:
    print("caught dec-utf8 UnicodeDecodeError")
try:
    b"\xff\xfe".decode("utf-8")
except ValueError:
    print("caught dec-utf8 ValueError")
try:
    b"\xe9".decode("ascii")
except UnicodeError:
    print("caught dec-ascii UnicodeError")
try:
    b"x".decode("zzz-codec")
except LookupError:
    print("caught dec-unknown LookupError")

print("str/bytes method-arg (count / start-end / encode-decode) tests passed!")
