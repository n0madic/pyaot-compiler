"""Basic smoke test for the `requests` third-party package."""

import requests

resp = requests.get("https://httpbin.org/get", timeout=10.0)
print("status:", resp.status)
print("url:", resp.url)
