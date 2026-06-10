# Phase 7C gate — custom exception classes.

class AppError(Exception):
    pass

class ParseError(AppError):
    pass

class LimitError(ValueError):
    pass

class HttpError(Exception):
    def __init__(self, status: int, reason: str):
        self.status = status
        self.reason = reason

# ── raise / catch a custom class ──
def basic() -> str:
    try:
        raise AppError("app failed")
    except AppError:
        return "caught"

print(basic())

# ── caught by user parent ──
def by_parent() -> str:
    try:
        raise ParseError("bad token")
    except AppError:
        return "parent"

print(by_parent())

# ── caught by builtin parent ──
def by_builtin_parent() -> str:
    try:
        raise LimitError("too big")
    except ValueError:
        return "builtin-parent"

print(by_builtin_parent())

# ── caught by Exception ──
def by_exception() -> str:
    try:
        raise ParseError("deep")
    except Exception:
        return "exception"

print(by_exception())

# ── specific handler wins over parent listed later ──
def specific_first() -> str:
    try:
        raise ParseError("p")
    except ParseError:
        return "specific"
    except AppError:
        return "general"

print(specific_first())

# ── unrelated handler does not catch ──
def unrelated() -> str:
    try:
        try:
            raise AppError("a")
        except ValueError:
            return "wrong"
    except AppError:
        return "outer"

print(unrelated())

# ── message surface for __init__-less custom classes ──
def message() -> str:
    try:
        raise AppError("hello world")
    except AppError as e:
        return str(e)

print(message())

# ── custom exception with fields ──
def fields() -> str:
    try:
        raise HttpError(404, "Not Found")
    except HttpError as e:
        if e.status == 404:
            return e.reason
        return "wrong"

print(fields())

# ── class name of a caught custom exception ──
def class_name() -> str:
    try:
        raise ParseError("x")
    except ParseError as e:
        return e.__class__.__name__

print(class_name())

# ── raise e (re-raise a caught instance) ──
def raise_instance() -> str:
    try:
        try:
            raise HttpError(500, "boom")
        except HttpError as e:
            raise e
    except HttpError as e2:
        if e2.status == 500:
            return "instance"
        return "lost"

print(raise_instance())

# ── as-binding for tuple of custom classes ──
def tuple_clause(kind: int) -> str:
    try:
        if kind == 0:
            raise AppError("a")
        raise LimitError("l")
    except (AppError, LimitError):
        return "either"

print(tuple_clause(0), tuple_clause(1))

print("p7_custom_exc done")
