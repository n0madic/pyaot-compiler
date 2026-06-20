# §9 str predicates are now Unicode-aware (codepoint `char::is_*` instead of
# per-byte `is_ascii_*`). This probe restricts to codepoints where Rust's
# `char::is_*` and CPython's category definitions AGREE — Latin accented,
# Cyrillic, Greek, ASCII digits, Unicode whitespace — plus the ASCII cases that
# must not regress (for ASCII chars `char::is_*` == `is_ascii_*`). The residual
# divergence on obscure Numeric_Type codepoints (`½`, `Ⅷ`, superscripts) is a
# narrower documented limit and is NOT probed here.

# ---- isalpha (accented Latin, Cyrillic, Greek) ----
print("café".isalpha())
print("über".isalpha())
print("Привет".isalpha())
print("Ωμέγα".isalpha())
print("naïve".isalpha())
print("café!".isalpha())
print("abc".isalpha())
print("abc1".isalpha())
print("".isalpha())

# ---- isupper / islower (Unicode case) ----
print("Ñ".isupper())
print("ñ".islower())
print("ÜBER".isupper())
print("über".islower())
print("ПРИВЕТ".isupper())
print("привет".islower())
print("Привет".isupper())
print("Привет".islower())
print("ABC".isupper())
print("abc".islower())
print("Abc".isupper())
print("ÅÄÖ".isupper())
print("123".isupper())
print("".isupper())

# ---- isalnum (letters + digits across scripts) ----
print("café123".isalnum())
print("Привет42".isalnum())
print("abc123".isalnum())
print("abc 123".isalnum())
print("".isalnum())

# ---- isdigit (ASCII digits — the agreeing set) ----
print("123".isdigit())
print("0".isdigit())
print("12a".isdigit())
print("".isdigit())

# ---- isspace (Unicode whitespace) ----
print(" ".isspace())
print("\t\n\r ".isspace())
print(" ".isspace())
print(" ".isspace())
print("a b".isspace())
print("".isspace())

# ---- isascii (unchanged) ----
print("hello".isascii())
print("café".isascii())
print("".isascii())

print("Unicode predicate tests passed!")
