"""Assert-based smoke test for the Python `requests` package via httpbin.

The call sites use **kwargs** so the script is a drop-in user of either:

- `pip install requests` (the real library), OR
- pyaot's bundled `site-packages/requests/__init__.py`

Both expose the same keyword-argument surface (`params=`, `headers=`,
`auth=`, `timeout=`, `data=`, `json=`). Running this test therefore
validates that pyaot's `site-packages/requests` is a faithful drop-in for
the standard pip package on the covered API slice.

Run:
    # Against pyaot's bundled requests (CPython fallback)
    PYTHONPATH=site-packages python3 examples/packages/test_requests.py
    # Against pip-installed requests
    python3 examples/packages/test_requests.py
    # Compiled
    pyaot examples/packages/test_requests.py -o /tmp/test_requests
    /tmp/test_requests
"""

import json
import requests
from http.client import HTTPResponse


# =============================================================================
# GET — params + headers + auth + timeout as kwargs
# =============================================================================
resp: HTTPResponse = requests.get(
    "https://httpbin.org/get",
    params={"q": "hello", "page": "1"},
    headers={"X-Test": "pyaot"},
    timeout=10.0,
)
assert resp.status_code == 200, f"GET expected 200, got {resp.status_code}"
body: str = resp.text
assert "q=hello" in body and "page=1" in body, (
    "query-string params should reach the server (httpbin echoes them)"
)
assert "X-Test" in body or "pyaot" in body, (
    "custom headers should reach the server"
)


# =============================================================================
# POST with bytes body (kwargs form)
# =============================================================================
r1: HTTPResponse = requests.post(
    "https://httpbin.org/post",
    data=b"raw-bytes-body",
    headers={"Content-Type": "application/octet-stream"},
    timeout=10.0,
)
assert r1.status_code == 200, f"POST bytes expected 200, got {r1.status_code}"


# =============================================================================
# POST with dict form body (auto-urlencoded)
# =============================================================================
r3: HTTPResponse = requests.post(
    "https://httpbin.org/post",
    data={"name": "alice", "role": "admin"},
    timeout=10.0,
)
assert r3.status_code == 200, f"POST form expected 200, got {r3.status_code}"
r3_body: str = r3.text
assert "alice" in r3_body and "admin" in r3_body, (
    "form-encoded values should be echoed back"
)


# =============================================================================
# POST with json= (auto-serialised + Content-Type)
# =============================================================================
r4: HTTPResponse = requests.post(
    "https://httpbin.org/post",
    json={"key": "val"},
    timeout=10.0,
)
assert r4.status_code == 200, f"POST json expected 200, got {r4.status_code}"
r4_body: str = r4.text
assert "application/json" in r4_body, (
    "json POST should set Content-Type: application/json"
)
assert '"key"' in r4_body and '"val"' in r4_body, (
    "json body should be echoed back"
)


# =============================================================================
# PUT / DELETE
# =============================================================================
r5: HTTPResponse = requests.put("https://httpbin.org/put", data=b"x", timeout=10.0)
assert r5.status_code == 200, f"PUT expected 200, got {r5.status_code}"

r6: HTTPResponse = requests.delete("https://httpbin.org/delete", timeout=10.0)
assert r6.status_code == 200, f"DELETE expected 200, got {r6.status_code}"


# =============================================================================
# Basic auth (success + failure via HTTPError caught by the facade)
# =============================================================================
r7: HTTPResponse = requests.get(
    "https://httpbin.org/basic-auth/alice/s3cr3t",
    auth=("alice", "s3cr3t"),
    timeout=10.0,
)
assert r7.status_code == 200, (
    f"basic-auth with correct creds should return 200, got {r7.status_code}"
)

r8: HTTPResponse = requests.get(
    "https://httpbin.org/basic-auth/alice/s3cr3t",
    timeout=10.0,
)
assert r8.status_code == 401, (
    f"basic-auth without creds should return 401, got {r8.status_code}"
)


# =============================================================================
# JSON parsing via response.json()
# =============================================================================
r9: HTTPResponse = requests.get("https://httpbin.org/get", timeout=10.0)
parsed = r9.json()
assert parsed is not None, "response.json() should return a parsed object"

print("All requests tests passed!")
