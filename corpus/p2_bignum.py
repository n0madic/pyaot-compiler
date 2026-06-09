print(2 ** 100)
print(2 ** 64)
print(10 ** 30)


def factorial(n: int) -> int:
    result = 1
    for i in range(1, n + 1):
        result = result * i
    return result


print(factorial(30))
print(factorial(20))
print(factorial(10))


def fact_rec(n: int) -> int:
    if n <= 1:
        return 1
    return n * fact_rec(n - 1)


print(fact_rec(25))

big = 2 ** 100
print(big + 1)
print(big * 2)
print(big - big)
print(big // 1000000)
print(big % 7)
print(big > 0)
print(big == 2 ** 100)
print(2 ** 100 < 2 ** 101)
print(-(2 ** 70))
print(2 ** 100 // 2 ** 100)
print((10 ** 20) - (10 ** 20))
xb = 2 ** 100
print(xb & 1)
print(xb | 1)
print(xb ^ 1)
print(xb >> 4)
print(xb << 4)
print((xb << 4) >> 4 == xb)
print(2 ** 100 & 1)
print((2 ** 100 + 1) & 255)
print(xb & 0)
