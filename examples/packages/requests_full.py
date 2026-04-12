"""Smoke test for the full requests-style API against httpbin.org."""

import requests

# GET with query params + custom headers
resp = requests.get(
    "https://httpbin.org/get",
    params={"q": "hello", "page": "1"},
    headers={"X-Test-Header": "pyaot"},
    timeout=10.0,
)
print("GET status:", resp.status)
print("GET url:   ", resp.url)

# POST with a bytes body and a custom Content-Type
body: bytes = b'{"key":"value"}'
resp2 = requests.post(
    "https://httpbin.org/post",
    data=body,
    headers={"Content-Type": "application/json"},
    timeout=10.0,
)
print("POST status:", resp2.status)

# PUT with body
resp3 = requests.put(
    "https://httpbin.org/put",
    data=b"updated",
    timeout=10.0,
)
print("PUT status:", resp3.status)

# DELETE
resp4 = requests.delete(
    "https://httpbin.org/delete",
    headers={"X-Test-Header": "bye"},
    timeout=10.0,
)
print("DELETE status:", resp4.status)
