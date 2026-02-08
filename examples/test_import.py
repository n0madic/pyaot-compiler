# Comprehensive import testing
# Tests both "from module import name" and "import module" syntax

# ============================================================
# SECTION 1: Test "from module import" syntax
# ============================================================

from math_utils import add, multiply

# Test add function
sum_result: int = add(2, 3)
assert sum_result == 5, "sum_result should equal 5"
print(sum_result)

# Test multiply function
product: int = multiply(4, 5)
assert product == 20, "product should equal 20"
print(product)

# Test combined usage
combined: int = add(multiply(2, 3), 4)
assert combined == 10, "combined should equal 10"
print(combined)

# ============================================================
# SECTION 2: Test "import module" syntax
# ============================================================

import math_utils

# Test function calls via module.function()
result: int = math_utils.add(2, 3)
assert result == 5, "result should equal 5"
print(result)

product2: int = math_utils.multiply(4, 5)
assert product2 == 20, "product2 should equal 20"
print(product2)

# Test global variable access via module.VAR
pi: float = math_utils.PI
assert pi > 3.14, "pi should be greater than 3.14"
assert pi < 3.15, "pi should be less than 3.15"
print(pi)

e: float = math_utils.E
assert e > 2.71, "e should be greater than 2.71"
assert e < 2.72, "e should be less than 2.72"
print(e)

name: str = math_utils.NAME
assert name == "math_utils", "name should equal \"math_utils\""
print(name)

# Test class instantiation via module.ClassName()
p = math_utils.Point(3, 4)
assert p.x == 3, "p.x should equal 3"
assert p.y == 4, "p.y should equal 4"
print(p.x)
print(p.y)

# Test method calls on instances from imported module
sum_result2: int = p.sum()
assert sum_result2 == 7, "sum_result2 should equal 7"
print(sum_result2)

# Test creating multiple instances
p2 = math_utils.Point(10, 20)
assert p2.x == 10, "p2.x should equal 10"
assert p2.y == 20, "p2.y should equal 20"
assert p2.sum() == 30, "p2.sum() should equal 30"

# ============================================================
# SECTION 3: Relative imports - "from .module import func"
# mypackage/__init__.py uses: from .utils import double
# ============================================================

import mypackage

# Verify mypackage.__init__.py successfully used "from .utils import double"
doubled_value: int = mypackage.get_doubled_value()
assert doubled_value == 42, "doubled_value should equal 42"
print(doubled_value)

# Verify direct access still works
assert mypackage.helper() == 42, "mypackage.helper() should equal 42"
assert mypackage.greet == "Hello from mypackage", "mypackage.greet should equal \"Hello from mypackage\""
print(mypackage.greet)

# ============================================================
# SECTION 4: Relative imports - "from .. import var"
# mypackage/math/__init__.py uses: from .. import greet
# ============================================================

from mypackage.math import get_parent_greeting

# Verify mypackage.math.__init__.py successfully used "from .. import greet"
parent_greet: str = get_parent_greeting()
assert parent_greet == "Hello from mypackage", "parent_greet should equal \"Hello from mypackage\""
print(parent_greet)

# ============================================================
# SECTION 5: Relative imports - "from ..module import func"
# mypackage/math/ops.py uses: from ..utils import double
# ============================================================

from mypackage.math.ops import doubled_ten, get_doubled

# Verify mypackage.math.ops successfully used "from ..utils import double"
doubled: int = doubled_ten()
assert doubled == 20, "doubled should equal 20"
print(doubled)

# Additional verification
doubled_five: int = get_doubled(5)
assert doubled_five == 10, "doubled_five should equal 10"
print(doubled_five)

# ============================================================
# SECTION 6: Relative imports - "from . import VAR"
# mypackage/math/ops.py uses: from . import PI
# Tests importing a variable (not function) via relative import
# ============================================================

from mypackage.math import PI
from mypackage.math.ops import get_pi

# Verify direct access to PI from parent package
assert PI > 3.14, "PI should be greater than 3.14"
assert PI < 3.15, "PI should be less than 3.15"
print(PI)

# Verify get_pi() function that uses relative-imported PI
pi_from_func: float = get_pi()
assert pi_from_func > 3.14, "pi_from_func should be greater than 3.14"
assert pi_from_func < 3.15, "pi_from_func should be less than 3.15"
print(pi_from_func)

print("All import tests passed!")
