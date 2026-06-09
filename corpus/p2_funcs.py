def add(a: int, b: int) -> int:
    return a + b


def factorial(n: int) -> int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)


def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)


def is_even(n: int) -> bool:
    return n % 2 == 0


def area(r: float) -> float:
    return 3.14159 * r * r


def greet(name: str) -> str:
    return name


def countdown(n: int) -> int:
    total = 0
    while n > 0:
        total = total + n
        n = n - 1
    return total


def classify(n: int) -> str:
    if n < 0:
        return "negative"
    elif n == 0:
        return "zero"
    else:
        return "positive"


print(add(3, 4))
print(factorial(5))
print(fib(10))
print(is_even(4))
print(is_even(7))
print(area(2.0))
print(greet("hello"))
print(countdown(5))
print(classify(-3))
print(classify(0))
print(classify(42))
print(add(factorial(4), fib(7)))
