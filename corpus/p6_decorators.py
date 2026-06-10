from typing import Callable


# ── a logging wrapper that forwards *args/**kwargs ──
def logged(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        print("call")
        return func(*args, **kwargs)
    return wrapper


@logged
def add(a, b):
    return a + b


print(add(2, 3))
print(add(10, 20))


# ── a counting wrapper that keeps state via nonlocal ──
def counted(func: Callable[..., int]) -> Callable[..., int]:
    count = 0

    def wrapper(*args, **kwargs) -> int:
        nonlocal count
        count = count + 1
        print("n=" + str(count))
        return func(*args, **kwargs)

    return wrapper


@counted
def square(x):
    return x * x


print(square(4))
print(square(5))
print(square(6))


# ── stacked decorators apply innermost-first; order visible via prints ──
def deco_a(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        print("a")
        return func(*args, **kwargs)
    return wrapper


def deco_b(func: Callable[..., int]) -> Callable[..., int]:
    def wrapper(*args, **kwargs) -> int:
        print("b")
        return func(*args, **kwargs)
    return wrapper


@deco_a
@deco_b
def hello():
    print("hello")
    return 0


print(hello())


# ── a decorator factory @repeat(3) ──
def repeat(n: int) -> Callable[[Callable[..., int]], Callable[..., int]]:
    def decorator(func: Callable[..., int]) -> Callable[..., int]:
        def wrapper(*args, **kwargs) -> int:
            result = 0
            for _ in range(n):
                result = func(*args, **kwargs)
            return result
        return wrapper
    return decorator


@repeat(3)
def announce(msg):
    print(msg)
    return 1


print(announce("hi"))


# ── decorators over non-int return types (float / str) ──
def trace_f(func: Callable[..., float]) -> Callable[..., float]:
    def wrapper(*args, **kwargs) -> float:
        print("f")
        return func(*args, **kwargs)
    return wrapper


@trace_f
def scaled(x) -> float:
    return x * 1.5


print(scaled(4))


def trace_s(func: Callable[..., str]) -> Callable[..., str]:
    def wrapper(*args, **kwargs) -> str:
        print("s")
        return func(*args, **kwargs)
    return wrapper


@trace_s
def shout(text) -> str:
    return text + "!"


print(shout("hey"))
