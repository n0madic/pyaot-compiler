# ── a coroutine-style generator receiving values via send() ──
def accumulator():
    total = 0
    while True:
        x = yield
        total = total + x
        print("total", total)


acc = accumulator()
next(acc)  # prime to the first yield
acc.send(10)
acc.send(5)
acc.send(100)
acc.close()


# ── send() that both receives and the loop reacts ──
def echo_twice():
    while True:
        x = yield
        print("once", x)
        print("twice", x)


e = echo_twice()
next(e)
e.send("hi")
e.send("yo")
e.close()


# ── close() ends a generator early ──
def ticking():
    n = 0
    while True:
        n = n + 1
        yield n


t = ticking()
print(next(t))
print(next(t))
t.close()
print("closed")
