"""Phase 5D — @staticmethod, @classmethod, @property/@setter, class attributes."""


class Temperature:
    # Class-level attributes (shared across instances).
    scale = "celsius"
    count = 0

    def __init__(self, degrees: float):
        self._degrees = degrees
        Temperature.count = Temperature.count + 1

    @property
    def degrees(self) -> float:
        return self._degrees

    @degrees.setter
    def degrees(self, value: float):
        self._degrees = value

    @staticmethod
    def freezing() -> float:
        return 0.0

    @classmethod
    def unit(cls) -> str:
        return Temperature.scale


t = Temperature(25.0)
print(t.degrees)               # property getter
t.degrees = 30.0               # property setter
print(t.degrees)

print(Temperature.freezing())  # staticmethod via class
print(t.freezing())            # staticmethod via instance

print(Temperature.unit())      # classmethod via class
print(t.unit())                # classmethod via instance

print(Temperature.scale)       # class attribute read via class
Temperature.scale = "kelvin"   # class attribute write
print(Temperature.scale)
print(t.unit())                # reflects the updated class attribute

u = Temperature(10.0)
print(Temperature.count)       # class attribute mutated by each __init__
print(u.degrees)
