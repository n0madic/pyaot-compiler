# Phase 8H stage B — codepoint-correct string model (#12).
# len/индексация/срезы/итерация/реверс/кейс — на кириллице и emoji.

s = "привет"
print(len(s))
print(s[0], s[2], s[-1], s[-3])
print(s[1:4])
print(s[:3], s[3:])
print(s[-4:-1])
print(s[::-1])
print(s[::2])
print(s[1::2])
print(s[5:1:-1])

m = "aбвgд"  # mixed ASCII + Cyrillic
print(len(m))
print(m[1], m[3])
print(m[::-1])

e = "x😀y"
print(len(e))
print(e[0], e[1], e[2])
print(e[::-1])
print(e[1:])

# Iteration walks codepoints
for ch in s:
    print(ch)
chars = []
for ch in e:
    chars.append(ch)
print(chars)

# reversed()
for ch in reversed(s):
    print(ch)

# find / index / count are codepoint-based
print(s.find("вет"))
print(s.index("р"))
print("абабаб".count("аб"))
print(s.find("нет"))

# in / concat
print("ив" in s)
print("xy" in e)
print(s + "!" + e)

# case conversions (Unicode-aware)
print(s.upper())
print("ПРИВЕТ".lower())
print("straße".upper())
print("привет мир".title())
print("привет".capitalize())
print("ПрИвЕт".swapcase())

# alignment widths are in characters
print("[" + "пр".center(6) + "]")
print("[" + "пр".ljust(5, "-") + "]")
print("[" + "пр".rjust(5, "*") + "]")
print("[" + "ab".center(5) + "]")
print("-42".zfill(6))
print("пр".zfill(4))

# ord/chr round-trip
print(ord("ё"), chr(1105))
print(ord("😀"), chr(128512))

# f-strings with unicode values
name = "мир"
print(f"привет, {name}!")
print(f"{name:>6}")
print(f"{name:*^7}")

print("p8h unicode passed!")
