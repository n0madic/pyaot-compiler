"""Phase 5C — container dunders: __len__ / __getitem__ / __setitem__ / __contains__."""


class IntList:
    def __init__(self, data: list[int]):
        self.data = data

    def __len__(self) -> int:
        return len(self.data)

    def __getitem__(self, i: int) -> int:
        return self.data[i]

    def __setitem__(self, i: int, value: int):
        self.data[i] = value

    def __contains__(self, x: int) -> bool:
        return x in self.data

    def __repr__(self) -> str:
        return "IntList(len=" + str(len(self.data)) + ")"


box = IntList([10, 20, 30, 40])
print(len(box))      # __len__ → 4
print(box[0])        # __getitem__ → 10
print(box[2])        # __getitem__ → 30
print(box[-1])       # __getitem__ (negative index handled by the inner list) → 40

box[1] = 99          # __setitem__
print(box[1])        # 99

print(20 in box)     # __contains__ → False (slot 1 is now 99)
print(30 in box)     # __contains__ → True
print(99 in box)     # __contains__ → True
print(5 in box)      # __contains__ → False
print(99 not in box) # negated __contains__ → False

print(len(box))      # still 4
print(box)           # __repr__ → IntList(len=4)
