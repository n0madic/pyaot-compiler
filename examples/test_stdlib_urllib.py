# Test urllib.parse module functionality
from urllib.parse import urlparse, quote, unquote, urljoin, urlencode, parse_qs
from urllib.request import urlopen
from urllib.error import HTTPError

# =============================================================================
# Test urlparse - Parse URLs into components
# =============================================================================

# Test basic HTTPS URL
url1 = urlparse("https://example.com/path?query=1#frag")
assert url1.scheme == "https", f"Expected 'https', got '{url1.scheme}'"
assert url1.netloc == "example.com", f"Expected 'example.com', got '{url1.netloc}'"
assert url1.path == "/path", f"Expected '/path', got '{url1.path}'"
assert url1.query == "query=1", f"Expected 'query=1', got '{url1.query}'"
assert url1.fragment == "frag", f"Expected 'frag', got '{url1.fragment}'"

# Test HTTP URL with port
url2 = urlparse("http://localhost:8080/api/v1")
assert url2.scheme == "http", f"Expected 'http', got '{url2.scheme}'"
assert url2.netloc == "localhost:8080", f"Expected 'localhost:8080', got '{url2.netloc}'"
assert url2.path == "/api/v1", f"Expected '/api/v1', got '{url2.path}'"

# Test FTP URL
url3 = urlparse("ftp://files.example.com/pub/file.txt")
assert url3.scheme == "ftp", f"Expected 'ftp', got '{url3.scheme}'"
assert url3.netloc == "files.example.com", f"Expected 'files.example.com', got '{url3.netloc}'"
assert url3.path == "/pub/file.txt", f"Expected '/pub/file.txt', got '{url3.path}'"

# Test URL with user info
url4 = urlparse("https://user:pass@example.com/secure")
assert url4.netloc == "user:pass@example.com", f"Expected 'user:pass@example.com', got '{url4.netloc}'"

# Test URL without scheme (path only)
url5 = urlparse("/just/a/path")
assert url5.scheme == "", f"Expected empty scheme, got '{url5.scheme}'"
assert url5.path == "/just/a/path", f"Expected '/just/a/path', got '{url5.path}'"

# Test URL with query only
url6 = urlparse("?key=value&foo=bar")
assert url6.query == "key=value&foo=bar", f"Expected 'key=value&foo=bar', got '{url6.query}'"

# Test empty URL
url7 = urlparse("")
assert url7.scheme == "", f"Expected empty scheme, got '{url7.scheme}'"
assert url7.netloc == "", f"Expected empty netloc, got '{url7.netloc}'"
assert url7.path == "", f"Expected empty path, got '{url7.path}'"

# Test geturl() method
url8 = urlparse("https://example.com/path?q=1#section")
reassembled = url8.geturl()
assert reassembled == "https://example.com/path?q=1#section", f"Expected original URL, got '{reassembled}'"

print("urlparse tests passed!")

# =============================================================================
# Test quote - Percent-encode strings
# =============================================================================

# Test basic encoding
encoded1 = quote("hello world")
assert encoded1 == "hello%20world", f"Expected 'hello%20world', got '{encoded1}'"

# Test encoding special characters
encoded2 = quote("a=b&c=d")
assert encoded2 == "a%3Db%26c%3Dd", f"Expected 'a%3Db%26c%3Dd', got '{encoded2}'"

# Test with safe parameter
encoded3 = quote("a/b/c", "/")
assert encoded3 == "a/b/c", f"Expected 'a/b/c', got '{encoded3}'"

# Test alphanumeric unchanged
encoded4 = quote("ABC123")
assert encoded4 == "ABC123", f"Expected 'ABC123', got '{encoded4}'"

# Test empty string
encoded5 = quote("")
assert encoded5 == "", f"Expected empty string, got '{encoded5}'"

# Test unicode
encoded6 = quote("hello%world")
assert encoded6 == "hello%25world", f"Expected 'hello%25world', got '{encoded6}'"

print("quote tests passed!")

# =============================================================================
# Test unquote - Decode percent-encoded strings
# =============================================================================

# Test basic decoding
decoded1 = unquote("hello%20world")
assert decoded1 == "hello world", f"Expected 'hello world', got '{decoded1}'"

# Test decoding special characters
decoded2 = unquote("a%3Db%26c%3Dd")
assert decoded2 == "a=b&c=d", f"Expected 'a=b&c=d', got '{decoded2}'"

# Test plus is NOT decoded as space (only unquote_plus does that)
decoded3 = unquote("hello+world")
assert decoded3 == "hello+world", f"Expected 'hello+world', got '{decoded3}'"

# Test lowercase hex
decoded4 = unquote("hello%2fworld")
assert decoded4 == "hello/world", f"Expected 'hello/world', got '{decoded4}'"

# Test already decoded string
decoded5 = unquote("already decoded")
assert decoded5 == "already decoded", f"Expected 'already decoded', got '{decoded5}'"

# Test empty string
decoded6 = unquote("")
assert decoded6 == "", f"Expected empty string, got '{decoded6}'"

print("unquote tests passed!")

# =============================================================================
# Test urljoin - Join base URL with relative URL
# =============================================================================

# Test basic join
joined1 = urljoin("https://example.com/a/", "b")
assert joined1 == "https://example.com/a/b", f"Expected 'https://example.com/a/b', got '{joined1}'"

# Test absolute path
joined2 = urljoin("https://example.com/a/b", "/c")
assert joined2 == "https://example.com/c", f"Expected 'https://example.com/c', got '{joined2}'"

# Test parent directory
joined3 = urljoin("https://example.com/a/b/", "../c")
assert joined3 == "https://example.com/a/c", f"Expected 'https://example.com/a/c', got '{joined3}'"

# Test absolute URL (should ignore base)
joined4 = urljoin("https://example.com/", "https://other.com/path")
assert joined4 == "https://other.com/path", f"Expected 'https://other.com/path', got '{joined4}'"

# Test empty relative URL
joined5 = urljoin("https://example.com/path", "")
assert joined5 == "https://example.com/path", f"Expected 'https://example.com/path', got '{joined5}'"

# Test query string in relative
joined6 = urljoin("https://example.com/path", "?query=1")
assert joined6 == "https://example.com/path?query=1", f"Expected 'https://example.com/path?query=1', got '{joined6}'"

# Test double dots
joined7 = urljoin("https://example.com/a/b/c/", "../../d")
assert joined7 == "https://example.com/a/d", f"Expected 'https://example.com/a/d', got '{joined7}'"

print("urljoin tests passed!")

# =============================================================================
# Test urlencode - Encode dict as query string
# =============================================================================

# Test basic encoding
params1: dict[str, str] = {"key": "value"}
encoded_qs1 = urlencode(params1)
assert encoded_qs1 == "key=value", f"Expected 'key=value', got '{encoded_qs1}'"

# Test multiple parameters (order may vary due to dict)
params2: dict[str, str] = {"a": "1", "b": "2"}
encoded_qs2 = urlencode(params2)
# Check both possible orders since dict order depends on hash
assert "a=1" in encoded_qs2 and "b=2" in encoded_qs2, f"Expected 'a=1' and 'b=2' in '{encoded_qs2}'"
assert "&" in encoded_qs2, f"Expected '&' separator in '{encoded_qs2}'"

# Test encoding special characters in values
params3: dict[str, str] = {"query": "hello world"}
encoded_qs3 = urlencode(params3)
assert encoded_qs3 == "query=hello+world", f"Expected 'query=hello+world', got '{encoded_qs3}'"

# Test empty dict
params4: dict[str, str] = {}
encoded_qs4 = urlencode(params4)
assert encoded_qs4 == "", f"Expected empty string, got '{encoded_qs4}'"

print("urlencode tests passed!")

# =============================================================================
# Test parse_qs - Parse query string to dict
# =============================================================================

# Test basic parsing
parsed1 = parse_qs("key=value")
assert "key" in parsed1, "Expected 'key' in result"
assert parsed1["key"][0] == "value", f"Expected 'value', got '{parsed1['key'][0]}'"

# Test multiple values for same key
parsed2 = parse_qs("a=1&a=2")
assert "a" in parsed2, "Expected 'a' in result"
assert len(parsed2["a"]) == 2, f"Expected 2 values, got {len(parsed2['a'])}"
assert parsed2["a"][0] == "1", f"Expected '1', got '{parsed2['a'][0]}'"
assert parsed2["a"][1] == "2", f"Expected '2', got '{parsed2['a'][1]}'"

# Test multiple keys
parsed3 = parse_qs("foo=bar&baz=qux")
assert "foo" in parsed3 and "baz" in parsed3, "Expected both 'foo' and 'baz' in result"
assert parsed3["foo"][0] == "bar", f"Expected 'bar', got '{parsed3['foo'][0]}'"
assert parsed3["baz"][0] == "qux", f"Expected 'qux', got '{parsed3['baz'][0]}'"

# Test URL-encoded values
parsed4 = parse_qs("msg=hello%20world")
assert parsed4["msg"][0] == "hello world", f"Expected 'hello world', got '{parsed4['msg'][0]}'"

# Test plus as space
parsed5 = parse_qs("msg=hello+world")
assert parsed5["msg"][0] == "hello world", f"Expected 'hello world', got '{parsed5['msg'][0]}'"

# Test empty query string
parsed6 = parse_qs("")
assert len(parsed6) == 0, f"Expected empty dict, got {len(parsed6)} items"

# Test with leading ? (CPython does NOT strip it — '?' becomes part of first key)
parsed7 = parse_qs("?key=value")
assert "?key" in parsed7, f"Expected '?key' in result, got {list(parsed7.keys())}"
assert parsed7["?key"][0] == "value", f"Expected 'value', got '{parsed7['?key'][0]}'"

print("parse_qs tests passed!")

# =============================================================================
# Combined/Integration tests
# =============================================================================

# Test round-trip: parse, modify, reassemble
original_url = "https://api.example.com/v1/users?page=1&limit=10#results"
parsed = urlparse(original_url)
assert parsed.scheme == "https"
assert parsed.netloc == "api.example.com"
assert parsed.path == "/v1/users"

# Parse the query string
query_params = parse_qs(parsed.query)
assert query_params["page"][0] == "1"
assert query_params["limit"][0] == "10"

# Reassemble
reassembled = parsed.geturl()
assert reassembled == original_url, f"Expected '{original_url}', got '{reassembled}'"

print("Integration tests passed!")

print("All urllib.parse tests passed!")

# =============================================================================
# Test urllib.request module
# =============================================================================

# NOTE: These tests require network connectivity and access to httpbin.org
# They will be skipped in the example test suite to avoid network dependencies
# For manual testing, uncomment and run with a network connection

try:
    # Test basic GET request
    response_get = urlopen("https://httpbin.org/get", None, 10.0)
    assert response_get.status == 200, f"Expected status 200, got {response_get.status}"
    assert "httpbin.org" in response_get.url, f"Expected httpbin.org in URL, got '{response_get.url}'"
    body_get = response_get.read()
    assert len(body_get) > 0, "Expected non-empty body"
    assert response_get.getcode() == 200, f"Expected getcode() 200, got {response_get.getcode()}"
    assert response_get.geturl() == response_get.url, "geturl() should match url field"

    # Test headers access
    headers_get = response_get.headers
    assert headers_get is not None, "headers should not be None"

    # Test POST request
    response_post = urlopen("https://httpbin.org/post", b"key=value", 10.0)
    assert response_post.status == 200, f"Expected status 200, got {response_post.status}"

    # Test HTTP error status
    # CPython raises HTTPError for 4xx/5xx; our runtime returns the response object.
    # Use try/except to handle both behaviors.
    try:
        response_404 = urlopen("https://httpbin.org/status/404", None, 10.0)
        # If we get here, runtime returned the response (compiled mode)
        assert response_404.status == 404, f"Expected status 404, got {response_404.status}"
    except HTTPError:
        # CPython raises HTTPError for 404
        pass

    print("urllib.request tests passed!")
except IOError:
    print("urllib.request tests skipped (no network)")

print("All urllib tests passed!")
