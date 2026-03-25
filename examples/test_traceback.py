# Test: Python-style tracebacks for exceptions

# Test 1: Caught ZeroDivisionError — program should continue
def divide(a: int, b: int) -> int:
    return a // b

def caller() -> int:
    return divide(10, 0)

try:
    result: int = caller()
except ZeroDivisionError:
    print("caught ZeroDivisionError")

# Test 2: Multiple try/except in sequence — stack stays consistent
def might_fail(n: int) -> int:
    return 100 // n

for i in range(3):
    try:
        val: int = might_fail(i)
        print(val)
    except ZeroDivisionError:
        print("skip zero")

# Test 3: Nested try/except — inner catch, outer continues
def nested_test() -> None:
    try:
        x: int = 1 // 0
    except ZeroDivisionError:
        print("inner caught")
    print("after inner try")

try:
    nested_test()
    print("outer ok")
except ZeroDivisionError:
    print("outer caught")

print("done")
