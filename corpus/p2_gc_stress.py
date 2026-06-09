# GC stress: a heap string and an accumulator stay live across ~14 MB of
# float-box allocations (which trigger several collections). If the live roots
# were not registered in the shadow frame, the collector would free them and
# this would print garbage or crash.
s = "survivor string that must not be freed"
acc = 0.0
for i in range(300000):
    acc = acc + 1.5
print(s)
print(acc)
print(len(s))
