# Test file for math module (compile-time constants)

import math

# Test math.pi
print("math.pi:", math.pi)
assert math.pi > 3.14
assert math.pi < 3.15

# Test math.e
print("math.e:", math.e)
assert math.e > 2.71
assert math.e < 2.72

# Test math.tau (2 * pi)
print("math.tau:", math.tau)
assert math.tau > 6.28
assert math.tau < 6.29

# Test math.inf (positive infinity)
print("math.inf:", math.inf)
assert math.inf > 1000000

# Test math.nan (not a number)
# NaN is special - it's not equal to itself
print("math.nan:", math.nan)

# Test from import style
from math import pi, e, tau, inf, nan

assert pi > 3.14
assert pi < 3.15
assert e > 2.71
assert e < 2.72
assert tau > 6.28
assert tau < 6.29
assert inf > 1000000

# Test using constants in expressions
circumference_factor: float = 2.0 * pi
assert circumference_factor > 6.28
assert circumference_factor < 6.29

# ===========================================================
# Test math functions
# ===========================================================

# Test math.sqrt()
print("Testing math.sqrt()...")
sqrt_result: float = math.sqrt(4.0)
print("sqrt(4.0):", sqrt_result)
assert sqrt_result == 2.0

sqrt_9: float = math.sqrt(9.0)
assert sqrt_9 == 3.0

sqrt_2: float = math.sqrt(2.0)
assert sqrt_2 > 1.41
assert sqrt_2 < 1.42

# Test math.sin()
print("Testing math.sin()...")
sin_0: float = math.sin(0.0)
print("sin(0):", sin_0)
assert sin_0 == 0.0

sin_half_pi: float = math.sin(pi / 2.0)
print("sin(pi/2):", sin_half_pi)
assert sin_half_pi > 0.999
assert sin_half_pi < 1.001

# Test math.cos()
print("Testing math.cos()...")
cos_0: float = math.cos(0.0)
print("cos(0):", cos_0)
assert cos_0 == 1.0

cos_pi: float = math.cos(pi)
print("cos(pi):", cos_pi)
assert cos_pi > -1.001
assert cos_pi < -0.999

# Test math.tan()
print("Testing math.tan()...")
tan_0: float = math.tan(0.0)
print("tan(0):", tan_0)
assert tan_0 == 0.0

tan_quarter_pi: float = math.tan(pi / 4.0)
print("tan(pi/4):", tan_quarter_pi)
assert tan_quarter_pi > 0.999
assert tan_quarter_pi < 1.001

# Test math.ceil()
print("Testing math.ceil()...")
ceil_result: int = math.ceil(3.2)
print("ceil(3.2):", ceil_result)
assert ceil_result == 4

ceil_neg: int = math.ceil(-3.8)
print("ceil(-3.8):", ceil_neg)
assert ceil_neg == -3

ceil_int: int = math.ceil(5.0)
assert ceil_int == 5

# Test math.floor()
print("Testing math.floor()...")
floor_result: int = math.floor(3.8)
print("floor(3.8):", floor_result)
assert floor_result == 3

floor_neg: int = math.floor(-3.2)
print("floor(-3.2):", floor_neg)
assert floor_neg == -4

floor_int: int = math.floor(5.0)
assert floor_int == 5

# Test math.factorial()
print("Testing math.factorial()...")
factorial_0: int = math.factorial(0)
print("factorial(0):", factorial_0)
assert factorial_0 == 1

factorial_1: int = math.factorial(1)
assert factorial_1 == 1

factorial_5: int = math.factorial(5)
print("factorial(5):", factorial_5)
assert factorial_5 == 120

factorial_10: int = math.factorial(10)
print("factorial(10):", factorial_10)
assert factorial_10 == 3628800

# Test from import for functions
from math import sqrt, sin, cos, tan, ceil, floor, factorial

sqrt_from_import: float = sqrt(16.0)
assert sqrt_from_import == 4.0

sin_from_import: float = sin(0.0)
assert sin_from_import == 0.0

cos_from_import: float = cos(0.0)
assert cos_from_import == 1.0

tan_from_import: float = tan(0.0)
assert tan_from_import == 0.0

ceil_from_import: int = ceil(2.1)
assert ceil_from_import == 3

floor_from_import: int = floor(2.9)
assert floor_from_import == 2

factorial_from_import: int = factorial(6)
assert factorial_from_import == 720

# ===========================================================
# Test new math functions: logarithms
# ===========================================================

print("Testing math.log()...")
log_e: float = math.log(e)
print("log(e):", log_e)
assert log_e > 0.999
assert log_e < 1.001

log_10: float = math.log(10.0)
assert log_10 > 2.3
assert log_10< 2.31

print("Testing math.log2()...")
log2_8: float = math.log2(8.0)
print("log2(8):", log2_8)
assert log2_8 == 3.0

log2_16: float = math.log2(16.0)
assert log2_16 == 4.0

print("Testing math.log10()...")
log10_100: float = math.log10(100.0)
print("log10(100):", log10_100)
assert log10_100 == 2.0

log10_1000: float = math.log10(1000.0)
assert log10_1000 == 3.0

# ===========================================================
# Test new math functions: exponential
# ===========================================================

print("Testing math.exp()...")
exp_0: float = math.exp(0.0)
print("exp(0):", exp_0)
assert exp_0 == 1.0

exp_1: float = math.exp(1.0)
print("exp(1):", exp_1)
assert exp_1 > 2.71
assert exp_1 < 2.72

exp_2: float = math.exp(2.0)
assert exp_2 > 7.38
assert exp_2 < 7.39

# ===========================================================
# Test new math functions: inverse trig
# ===========================================================

print("Testing math.asin()...")
asin_0: float = math.asin(0.0)
print("asin(0):", asin_0)
assert asin_0 == 0.0

asin_half: float = math.asin(0.5)
print("asin(0.5):", asin_half)
assert asin_half > 0.523
assert asin_half < 0.524

print("Testing math.acos()...")
acos_1: float = math.acos(1.0)
print("acos(1):", acos_1)
assert acos_1 == 0.0

acos_half: float = math.acos(0.5)
print("acos(0.5):", acos_half)
assert acos_half > 1.047
assert acos_half < 1.048

print("Testing math.atan()...")
atan_0: float = math.atan(0.0)
print("atan(0):", atan_0)
assert atan_0 == 0.0

atan_1: float = math.atan(1.0)
print("atan(1):", atan_1)
assert atan_1 > 0.785
assert atan_1 < 0.786

# ===========================================================
# Test new math functions: hyperbolic
# ===========================================================

print("Testing math.sinh()...")
sinh_0: float = math.sinh(0.0)
print("sinh(0):", sinh_0)
assert sinh_0 == 0.0

sinh_1: float = math.sinh(1.0)
assert sinh_1 > 1.175
assert sinh_1 < 1.176

print("Testing math.cosh()...")
cosh_0: float = math.cosh(0.0)
print("cosh(0):", cosh_0)
assert cosh_0 == 1.0

cosh_1: float = math.cosh(1.0)
assert cosh_1 > 1.543
assert cosh_1 < 1.544

print("Testing math.tanh()...")
tanh_0: float = math.tanh(0.0)
print("tanh(0):", tanh_0)
assert tanh_0 == 0.0

tanh_1: float = math.tanh(1.0)
assert tanh_1 > 0.761
assert tanh_1 < 0.762

# ===========================================================
# Test new math functions: utilities
# ===========================================================

print("Testing math.fabs()...")
fabs_pos: float = math.fabs(5.5)
print("fabs(5.5):", fabs_pos)
assert fabs_pos == 5.5

fabs_neg: float = math.fabs(-5.5)
print("fabs(-5.5):", fabs_neg)
assert fabs_neg == 5.5

fabs_zero: float = math.fabs(0.0)
assert fabs_zero == 0.0

print("Testing math.degrees()...")
degrees_pi: float = math.degrees(pi)
print("degrees(pi):", degrees_pi)
assert degrees_pi > 179.9
assert degrees_pi < 180.1

degrees_half_pi: float = math.degrees(pi / 2.0)
assert degrees_half_pi > 89.9
assert degrees_half_pi < 90.1

print("Testing math.radians()...")
radians_180: float = math.radians(180.0)
print("radians(180):", radians_180)
assert radians_180 > 3.14
assert radians_180 < 3.15

radians_90: float = math.radians(90.0)
assert radians_90 > 1.57
assert radians_90 < 1.58

print("Testing math.trunc()...")
trunc_pos: int = math.trunc(3.7)
print("trunc(3.7):", trunc_pos)
assert trunc_pos == 3

trunc_neg: int = math.trunc(-3.7)
print("trunc(-3.7):", trunc_neg)
assert trunc_neg == -3

trunc_zero: int = math.trunc(0.5)
assert trunc_zero == 0

# ===========================================================
# Test new math functions: special value checks
# ===========================================================

print("Testing math.isnan()...")
isnan_nan: bool = math.isnan(nan)
print("isnan(nan):", isnan_nan)
assert isnan_nan == True

isnan_float: bool = math.isnan(1.0)
print("isnan(1.0):", isnan_float)
assert isnan_float == False

isnan_inf: bool = math.isnan(inf)
assert isnan_inf == False

print("Testing math.isinf()...")
isinf_inf: bool = math.isinf(inf)
print("isinf(inf):", isinf_inf)
assert isinf_inf == True

isinf_float: bool = math.isinf(1.0)
print("isinf(1.0):", isinf_float)
assert isinf_float == False

isinf_nan: bool = math.isinf(nan)
assert isinf_nan == False

print("Testing math.isfinite()...")
isfinite_float: bool = math.isfinite(1.0)
print("isfinite(1.0):", isfinite_float)
assert isfinite_float == True

isfinite_inf: bool = math.isfinite(inf)
print("isfinite(inf):", isfinite_inf)
assert isfinite_inf == False

isfinite_nan: bool = math.isfinite(nan)
assert isfinite_nan == False

# ===========================================================
# Test new math functions: two-argument functions
# ===========================================================

print("Testing math.atan2()...")
atan2_1_1: float = math.atan2(1.0, 1.0)
print("atan2(1, 1):", atan2_1_1)
assert atan2_1_1 > 0.785
assert atan2_1_1 < 0.786

atan2_0_1: float = math.atan2(0.0, 1.0)
assert atan2_0_1 == 0.0

atan2_1_0: float = math.atan2(1.0, 0.0)
assert atan2_1_0 > 1.57
assert atan2_1_0 < 1.58

print("Testing math.fmod()...")
fmod_result: float = math.fmod(7.5, 2.5)
print("fmod(7.5, 2.5):", fmod_result)
assert fmod_result == 0.0

fmod_remainder: float = math.fmod(7.0, 3.0)
assert fmod_remainder == 1.0

print("Testing math.copysign()...")
copysign_pos_neg: float = math.copysign(5.0, -1.0)
print("copysign(5, -1):", copysign_pos_neg)
assert copysign_pos_neg == -5.0

copysign_neg_pos: float = math.copysign(-5.0, 1.0)
assert copysign_neg_pos == 5.0

copysign_pos_pos: float = math.copysign(5.0, 1.0)
assert copysign_pos_pos == 5.0

print("Testing math.hypot()...")
hypot_3_4: float = math.hypot(3.0, 4.0)
print("hypot(3, 4):", hypot_3_4)
assert hypot_3_4 == 5.0

hypot_5_12: float = math.hypot(5.0, 12.0)
assert hypot_5_12 == 13.0

print("Testing math.pow()...")
pow_2_3: float = math.pow(2.0, 3.0)
print("pow(2, 3):", pow_2_3)
assert pow_2_3 == 8.0

pow_10_2: float = math.pow(10.0, 2.0)
assert pow_10_2 == 100.0

pow_2_neg1: float = math.pow(2.0, -1.0)
assert pow_2_neg1 == 0.5

# ===========================================================
# Test new math functions: integer functions
# ===========================================================

print("Testing math.gcd()...")
gcd_48_18: int = math.gcd(48, 18)
print("gcd(48, 18):", gcd_48_18)
assert gcd_48_18 == 6

gcd_100_50: int = math.gcd(100, 50)
assert gcd_100_50 == 50

gcd_17_13: int = math.gcd(17, 13)
assert gcd_17_13 == 1

gcd_neg: int = math.gcd(-48, 18)
assert gcd_neg == 6

print("Testing math.lcm()...")
lcm_12_18: int = math.lcm(12, 18)
print("lcm(12, 18):", lcm_12_18)
assert lcm_12_18 == 36

lcm_4_6: int = math.lcm(4, 6)
assert lcm_4_6 == 12

lcm_same: int = math.lcm(7, 7)
assert lcm_same == 7

lcm_zero: int = math.lcm(5, 0)
assert lcm_zero == 0

print("Testing math.comb()...")
comb_5_2: int = math.comb(5, 2)
print("comb(5, 2):", comb_5_2)
assert comb_5_2 == 10

comb_10_3: int = math.comb(10, 3)
assert comb_10_3 == 120

comb_n_0: int = math.comb(5, 0)
assert comb_n_0 == 1

comb_n_n: int = math.comb(5, 5)
assert comb_n_n == 1

comb_k_gt_n: int = math.comb(3, 5)
assert comb_k_gt_n == 0

print("Testing math.perm()...")
perm_5_2: int = math.perm(5, 2)
print("perm(5, 2):", perm_5_2)
assert perm_5_2 == 20

perm_10_3: int = math.perm(10, 3)
assert perm_10_3 == 720

perm_n_0: int = math.perm(5, 0)
assert perm_n_0 == 1

perm_n_n: int = math.perm(5, 5)
assert perm_n_n == 120

perm_k_gt_n: int = math.perm(3, 5)
assert perm_k_gt_n == 0

# Test from import for new functions
from math import log, log2, log10, exp, asin, acos, atan
from math import sinh, cosh, tanh, fabs, degrees, radians, trunc
from math import isnan, isinf, isfinite
from math import atan2, fmod, copysign, hypot, pow
from math import gcd, lcm, comb, perm

log_from_import: float = log(e)
assert log_from_import > 0.999
assert log_from_import < 1.001

log2_from_import: float = log2(8.0)
assert log2_from_import == 3.0

exp_from_import: float = exp(1.0)
assert exp_from_import > 2.71
assert exp_from_import < 2.72

asin_from_import: float = asin(0.0)
assert asin_from_import == 0.0

sinh_from_import: float = sinh(0.0)
assert sinh_from_import == 0.0

fabs_from_import: float = fabs(-5.0)
assert fabs_from_import == 5.0

trunc_from_import: int = trunc(3.7)
assert trunc_from_import == 3

isnan_from_import: bool = isnan(nan)
assert isnan_from_import == True

atan2_from_import: float = atan2(1.0, 1.0)
assert atan2_from_import > 0.785
assert atan2_from_import < 0.786

gcd_from_import: int = gcd(48, 18)
assert gcd_from_import == 6

comb_from_import: int = comb(5, 2)
assert comb_from_import == 10

print("All math module tests passed!")
