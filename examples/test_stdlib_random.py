import random

# All tests use fixed seeds for deterministic, reproducible results

# ===== SECTION: random.seed() and random.random() =====

random.seed(42)
r1: float = random.random()
r2: float = random.random()
r3: float = random.random()

# Values must be in [0.0, 1.0)
assert r1 >= 0.0 and r1 < 1.0, "random() must be in [0, 1)"
assert r2 >= 0.0 and r2 < 1.0, "random() must be in [0, 1)"
assert r3 >= 0.0 and r3 < 1.0, "random() must be in [0, 1)"

# Same seed must produce same sequence
random.seed(42)
assert random.random() == r1, "seed(42) must reproduce first value"
assert random.random() == r2, "seed(42) must reproduce second value"
assert random.random() == r3, "seed(42) must reproduce third value"

print("random.seed() and random.random() passed")

# ===== SECTION: random.randint() =====

random.seed(100)
ri1: int = random.randint(1, 10)
ri2: int = random.randint(1, 10)
ri3: int = random.randint(1, 10)

assert ri1 >= 1 and ri1 <= 10, "randint must be in [1, 10]"
assert ri2 >= 1 and ri2 <= 10, "randint must be in [1, 10]"
assert ri3 >= 1 and ri3 <= 10, "randint must be in [1, 10]"

# Reproducibility
random.seed(100)
assert random.randint(1, 10) == ri1, "randint reproducibility 1"
assert random.randint(1, 10) == ri2, "randint reproducibility 2"
assert random.randint(1, 10) == ri3, "randint reproducibility 3"

# Edge case: a == b
random.seed(0)
assert random.randint(5, 5) == 5, "randint(5, 5) must be 5"

print("random.randint() passed")

# ===== SECTION: random.uniform() =====

random.seed(200)
u1: float = random.uniform(1.0, 5.0)
u2: float = random.uniform(-10.0, 10.0)

assert u1 >= 1.0 and u1 <= 5.0, "uniform must be in [1.0, 5.0]"
assert u2 >= -10.0 and u2 <= 10.0, "uniform must be in [-10.0, 10.0]"

# Reproducibility
random.seed(200)
assert random.uniform(1.0, 5.0) == u1, "uniform reproducibility 1"
assert random.uniform(-10.0, 10.0) == u2, "uniform reproducibility 2"

print("random.uniform() passed")

# ===== SECTION: random.randrange() =====

random.seed(300)
rr1: int = random.randrange(10)
rr2: int = random.randrange(5, 15)
rr3: int = random.randrange(0, 20, 2)

assert rr1 >= 0 and rr1 < 10, "randrange(10) must be in [0, 10)"
assert rr2 >= 5 and rr2 < 15, "randrange(5, 15) must be in [5, 15)"
assert rr3 >= 0 and rr3 < 20 and rr3 % 2 == 0, "randrange(0, 20, 2) must be even in [0, 20)"

# Reproducibility
random.seed(300)
assert random.randrange(10) == rr1, "randrange reproducibility 1"
assert random.randrange(5, 15) == rr2, "randrange reproducibility 2"
assert random.randrange(0, 20, 2) == rr3, "randrange reproducibility 3"

print("random.randrange() passed")

# ===== SECTION: random.choice() =====

random.seed(400)
items: list[str] = ["apple", "banana", "cherry", "date"]
c1: str = random.choice(items)
c2: str = random.choice(items)
c3: str = random.choice(items)

found1: bool = False
for item in items:
    if item == c1:
        found1 = True
assert found1, "choice must return an element from the list"

# Reproducibility
random.seed(400)
assert random.choice(items) == c1, "choice reproducibility 1"
assert random.choice(items) == c2, "choice reproducibility 2"
assert random.choice(items) == c3, "choice reproducibility 3"

print("random.choice() passed")

# ===== SECTION: random.shuffle() =====

random.seed(500)
nums: list[int] = [1, 2, 3, 4, 5]
random.shuffle(nums)

# After shuffle, same elements should exist
assert len(nums) == 5, "shuffle preserves length"
total: int = 0
for n in nums:
    total = total + n
assert total == 15, "shuffle preserves sum (all elements present)"

# Reproducibility
random.seed(500)
nums2: list[int] = [1, 2, 3, 4, 5]
random.shuffle(nums2)
for i in range(5):
    assert nums[i] == nums2[i], f"shuffle reproducibility at index {i}"

print("random.shuffle() passed")

# ===== SECTION: random.sample() =====

random.seed(600)
population: list[int] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
s1: list[int] = random.sample(population, 3)

assert len(s1) == 3, "sample(k=3) must return 3 elements"

# All elements must come from population
for elem in s1:
    found_elem: bool = False
    for p in population:
        if p == elem:
            found_elem = True
    assert found_elem, f"sample element {elem} must be in population"

# No duplicates in sample
assert s1[0] != s1[1] and s1[1] != s1[2] and s1[0] != s1[2], "sample must not have duplicates"

# Reproducibility
random.seed(600)
s2: list[int] = random.sample(population, 3)
for i in range(3):
    assert s1[i] == s2[i], f"sample reproducibility at index {i}"

print("random.sample() passed")

# ===== SECTION: random.gauss() =====

random.seed(700)
g1: float = random.gauss(0.0, 1.0)
g2: float = random.gauss(0.0, 1.0)
g3: float = random.gauss(5.0, 0.5)

# Gauss values are unbounded but should be near the mean for small sigma
assert g3 > 2.0 and g3 < 8.0, "gauss(5.0, 0.5) should be near 5.0"

# Reproducibility
random.seed(700)
assert random.gauss(0.0, 1.0) == g1, "gauss reproducibility 1"
assert random.gauss(0.0, 1.0) == g2, "gauss reproducibility 2"
assert random.gauss(5.0, 0.5) == g3, "gauss reproducibility 3"

# Test gauss with caching behavior (CPython caches one value per call pair)
random.seed(42)
gauss_vals: list[float] = []
for i in range(10):
    gauss_vals.append(random.gauss(0.0, 1.0))

random.seed(42)
for i in range(10):
    assert random.gauss(0.0, 1.0) == gauss_vals[i], f"gauss caching reproducibility at {i}"

print("random.gauss() passed")

# ===== SECTION: random.choices() =====

# NOTE: In CPython, random.choices() uses keyword args: choices(pop, weights=w, k=k)
# Our compiler passes them positionally: choices(pop, weights, k)
# Tests use positional form matching our compiler's calling convention.

random.seed(800)
colors: list[str] = ["red", "green", "blue"]
ch_weights: list[float] = [10.0, 1.0, 1.0]

ch1: list[str] = random.choices(colors, ch_weights, 5)  # type: ignore
assert len(ch1) == 5, "choices(k=5) must return 5 elements"

# All elements must come from population
for elem in ch1:
    found_ch: bool = False
    for c in colors:
        if c == elem:
            found_ch = True
    assert found_ch, "choices element must be in population"

# With heavy weight on "red", expect mostly reds
red_count: int = 0
random.seed(42)
many_choices: list[str] = random.choices(colors, ch_weights, 100)  # type: ignore
for elem in many_choices:
    if elem == "red":
        red_count = red_count + 1
assert red_count > 50, f"with weights [10,1,1], red should dominate, got {red_count}"

# Reproducibility
random.seed(800)
ch2: list[str] = random.choices(colors, ch_weights, 5)  # type: ignore
for i in range(5):
    assert ch1[i] == ch2[i], f"choices reproducibility at index {i}"

# choices with int population
random.seed(900)
int_pop: list[int] = [1, 2, 3, 4, 5]
int_ch_weights: list[float] = [1.0, 1.0, 1.0, 1.0, 1.0]
int_ch: list[int] = random.choices(int_pop, int_ch_weights, 3)  # type: ignore
assert len(int_ch) == 3, "choices with int population returns correct count"

for elem in int_ch:
    found_int: bool = False
    for p in int_pop:
        if p == elem:
            found_int = True
    assert found_int, "int choices element must be in population"

# Reproducibility for int choices
random.seed(900)
int_ch2: list[int] = random.choices(int_pop, int_ch_weights, 3)  # type: ignore
for i in range(3):
    assert int_ch[i] == int_ch2[i], f"int choices reproducibility at index {i}"

print("random.choices() passed")

print("All random tests passed!")
