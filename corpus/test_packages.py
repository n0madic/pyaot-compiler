# Test package imports with __init__.py
# This tests various patterns of package imports

# ============================================================
# Test 1: Import package (loads __init__.py)
# ============================================================

import mypackage

# Test attribute access from package __init__.py
assert mypackage.greet == "Hello from mypackage", "mypackage.greet should equal \"Hello from mypackage\""
print(mypackage.greet)

# Test function call from package __init__.py
result1: int = mypackage.helper()
assert result1 == 42, "result1 should equal 42"
print(result1)

# ============================================================
# Test 2: From package import
# ============================================================

from mypackage import helper, greet

# Test imported function
result2: int = helper()
assert result2 == 42, "result2 should equal 42"
print(result2)

# Test imported variable
assert greet == "Hello from mypackage", "greet should equal \"Hello from mypackage\""
print(greet)

# ============================================================
# Test 3: From submodule import
# ============================================================

from mypackage.utils import double, add_ten

# Test imported functions
result3: int = double(5)
assert result3 == 10, "result3 should equal 10"
print(result3)

result4: int = add_ten(5)
assert result4 == 15, "result4 should equal 15"
print(result4)

# ============================================================
# Test 4: From nested subpackage import
# ============================================================

from mypackage.math import PI, square

# Test imported constant
assert PI > 3.14, "PI should be greater than 3.14"
assert PI < 3.15, "PI should be less than 3.15"
print(PI)

# Test imported function
result5: int = square(4)
assert result5 == 16, "result5 should equal 16"
print(result5)

# ============================================================
# Test 5: From deeply nested submodule import
# ============================================================

from mypackage.math.ops import add, multiply

# Test imported functions
result6: int = add(2, 3)
assert result6 == 5, "result6 should equal 5"
print(result6)

result7: int = multiply(4, 5)
assert result7 == 20, "result7 should equal 20"
print(result7)

print("All package tests passed!")
