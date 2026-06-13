# §9 str methods (runtime-ready batch). Wires ~20 string methods whose runtime
# impls AND core-defs descriptors already existed but were not dispatched:
# split/rsplit/splitlines, replace, lstrip/rstrip, removeprefix/removesuffix,
# expandtabs, partition/rpartition, encode, rindex, and the ASCII predicates
# isdigit/isalpha/isalnum/isspace/isupper/islower/isascii.
#
# MECHANISM: a str receiver routes to `lower_str_method`, which dispatches off a
# declarative `StrPlan` table (descriptor + per-arg reprs + return spec); typeck
# types the result in `method_call_ty`'s `SemTy::Str` arm. No codegen edit — the
# runtime fn is resolved by symbol. The numeric `maxsplit`/`tabsize` ride a RAW
# i64 slot (descriptors retyped to a matching Raw MIR semantic — B16), never a
# tagged int misread as a count.
#
# The `==` assertions are the spec (Principle 9); multi-byte (Cyrillic / café)
# inputs exercise the codepoint `char_len` recount paths. SCOPE LIMITS (all
# UNPROBED here): positional-only (kwargs rejected); `replace` has no `count`;
# `splitlines` no `keepends`; `encode` ignores encoding (always UTF-8);
# `rindex`/`index`/`find` take no `start`/`end`; predicates are ASCII-only
# (`"café".isalpha()` → False here vs CPython True), so non-ASCII predicate
# cases are kept OUT.

# ===== split: whitespace / sep / maxsplit (RAW slot) / Cyrillic =====
assert "  a b  c  ".split() == ["a", "b", "c"]
assert "a,b,c".split(",") == ["a", "b", "c"]
assert "a,b,c,d".split(",", 2) == ["a", "b", "c,d"]
assert "".split() == []
assert "раз два три".split() == ["раз", "два", "три"]
assert "мир,труд,май".split(",") == ["мир", "труд", "май"]
print("split:", "a,b,c".split(","), "a,b,c,d".split(",", 2))
print("split ws:", "  a b  c  ".split())
print("split cyr:", "мир,труд,май".split(","))

# ===== rsplit: sep+maxsplit / no-arg whitespace / explicit None / Cyrillic =====
assert "a-b-c".rsplit("-", 1) == ["a-b", "c"]
assert "one two three".rsplit() == ["one", "two", "three"]
assert "a b c".rsplit(None, 1) == ["a b", "c"]
assert "раз-два-три".rsplit("-", 1) == ["раз-два", "три"]
print("rsplit:", "a-b-c".rsplit("-", 1), "a b c".rsplit(None, 1))
print("rsplit cyr:", "раз-два-три".rsplit("-", 1))

# ===== splitlines: \n / \r\n / \r mix / Cyrillic / empty =====
assert "a\nb\nc".splitlines() == ["a", "b", "c"]
assert "a\r\nb\rc\n".splitlines() == ["a", "b", "c"]
assert "".splitlines() == []
assert "one line".splitlines() == ["one line"]
assert "привет\nмир".splitlines() == ["привет", "мир"]
print("splitlines:", "a\r\nb\rc\n".splitlines())
print("splitlines cyr:", "привет\nмир".splitlines())

# ===== replace: ASCII / byte-ratio change (café → recount) / growth =====
assert "a,b,c".replace(",", ";") == "a;b;c"
assert "café".replace("é", "e") == "cafe"
assert len("café".replace("é", "e")) == 4
assert "aaa".replace("a", "bb") == "bbbbbb"
assert "hello".replace("l", "L") == "heLLo"
print("replace:", "a,b,c".replace(",", ";"), "café".replace("é", "e"))

# ===== lstrip / rstrip: whitespace + chars-set, codepoint-correct len =====
assert "  hi  ".lstrip() == "hi  "
assert "  hi  ".rstrip() == "  hi"
assert "xxcafé".lstrip("x") == "café"
assert "caféxx".rstrip("x") == "café"
assert len("  café".lstrip()) == 4
assert len("café  ".rstrip()) == 4
assert "café".lstrip(None) == "café"
print("strip:", "  hi  ".lstrip() + "|", "|" + "  hi  ".rstrip())
print("strip chars:", "xxcafé".lstrip("x"), "caféxx".rstrip("x"))

# ===== removeprefix / removesuffix: exact char_len subtract (Cyrillic) =====
assert "foobar".removeprefix("foo") == "bar"
assert "foobar".removesuffix("bar") == "foo"
assert "hello".removeprefix("xyz") == "hello"
assert "мир".removeprefix("ми") == "р"
assert len("мир".removeprefix("ми")) == 1
print("remove:", "foobar".removeprefix("foo"), "foobar".removesuffix("bar"))
print("remove cyr:", "мир".removeprefix("ми"))

# ===== expandtabs: RAW tabsize / default 8 / Cyrillic + tab =====
assert "a\tb".expandtabs(4) == "a   b"
assert "a\tb".expandtabs() == "a       b"
assert "a\tб".expandtabs(4) == "a   б"
assert "\tб".expandtabs(2) == "  б"
print("expandtabs:", repr("a\tb".expandtabs(4)), repr("a\tb".expandtabs()))

# ===== partition / rpartition: 3-tuple unpack through the gradual seam =====
a, sep, b = "key=value".partition("=")
assert a == "key" and sep == "=" and b == "value"
assert "no-sep".partition("=") == ("no-sep", "", "")
assert "a=b=c".partition("=") == ("a", "=", "b=c")
assert "a=b=c".rpartition("=") == ("a=b", "=", "c")
ca, csep, cb = "имя=значение".partition("=")
assert ca == "имя" and csep == "=" and cb == "значение"
print("partition:", "key=value".partition("="))
print("rpartition:", "a=b=c".rpartition("="))

# ===== encode: utf-8 bytes, codepoint↔byte length =====
assert "café".encode() == b"caf\xc3\xa9"
assert len("café".encode()) == 5
assert "x".encode("utf-8") == b"x"
assert "abc".encode() == b"abc"
print("encode:", "café".encode(), "len", len("café".encode()))

# ===== rindex: found (codepoint offset) / Cyrillic / miss → ValueError =====
assert "abcabc".rindex("b") == 4
assert "abcabc".rindex("a") == 3
assert "абвабв".rindex("б") == 4
miss_caught = False
try:
    "abc".rindex("z")
except ValueError:
    miss_caught = True
assert miss_caught
print("rindex:", "abcabc".rindex("b"), "абвабв".rindex("б"), "miss", miss_caught)

# ===== predicates: ASCII-only (non-ASCII diverges → kept out) =====
assert "123".isdigit() == True
assert "12a".isdigit() == False
assert "abc".isalpha() == True
assert "abc1".isalpha() == False
assert "abc123".isalnum() == True
assert "abc!".isalnum() == False
assert " \t".isspace() == True
assert "a b".isspace() == False
assert "ABC".isupper() == True
assert "Abc".isupper() == False
assert "abc".islower() == True
assert "Abc".islower() == False
assert "abc".isascii() == True
assert "café".isascii() == False
print("predicates:", "123".isdigit(), "abc".isalpha(), "café".isascii())

# ===== interaction probes (cross green features) =====
parts = "a,b,c".split(",")
assert len(parts[0]) == 1
assert f"first={parts[0]}" == "first=a"
total = 0
for x in "1,2,3,4".split(","):
    total += int(x)
assert total == 10
if "42".isdigit():
    flag = "numeric"
else:
    flag = "other"
assert flag == "numeric"
joined = "-".join("a,b,c".split(","))
assert joined == "a-b-c"
print("interaction:", parts, total, flag, joined)

print("All str-method tests passed!")
