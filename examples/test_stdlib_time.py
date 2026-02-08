# Test file for time module

import time

# Test time.time()
time_t1: float = time.time()
print("time.time():", time_t1)
assert time_t1 > 1700000000.0  # After 2023
assert time_t1 < 2000000000.0  # Before 2033

time_t2: float = time.time()
assert time_t2 >= time_t1

# Test time.sleep()
time_start: float = time.time()
time.sleep(0.1)
time_end: float = time.time()
time_elapsed: float = time_end - time_start
print("sleep elapsed:", time_elapsed)
assert time_elapsed >= 0.09
assert time_elapsed < 0.5

# Test time.monotonic()
time_m1: float = time.monotonic()
assert time_m1 >= 0.0
time.sleep(0.05)
time_m2: float = time.monotonic()
assert time_m2 > time_m1

# Test time.perf_counter()
time_p1: float = time.perf_counter()
assert time_p1 >= 0.0
time.sleep(0.05)
time_p2: float = time.perf_counter()
assert time_p2 > time_p1

# Test time.ctime()
ctime_str: str = time.ctime()
print("time.ctime():", ctime_str)
# ctime returns format like "Mon Feb  2 14:00:00 2026" (24 chars)
assert len(ctime_str) >= 20  # Should be around 24 chars
assert len(ctime_str) <= 30

# Test time.ctime(seconds) with specific timestamp
# 1700000000 = Nov 14, 2023
ctime_specific: str = time.ctime(1700000000.0)
print("time.ctime(1700000000):", ctime_specific)
assert "2023" in ctime_specific  # Year should be 2023
assert "Nov" in ctime_specific   # Month should be November

# Test from import
from time import sleep, time as get_time, monotonic, perf_counter, ctime
time_t3: float = get_time()
assert time_t3 > 1700000000.0

# Test ctime from import
ctime_imported: str = ctime()
assert len(ctime_imported) >= 20

# Test sleep(0) returns immediately
time_start2: float = get_time()
sleep(0.0)
time_end2: float = get_time()
assert time_end2 - time_start2 < 0.1

# Test time.localtime()
localtime_now: time.struct_time = time.localtime()
print("localtime() tm_year:", localtime_now.tm_year)
assert localtime_now.tm_year >= 2024  # Year should be reasonable
assert localtime_now.tm_year <= 2100
assert localtime_now.tm_mon >= 1
assert localtime_now.tm_mon <= 12
assert localtime_now.tm_mday >= 1
assert localtime_now.tm_mday <= 31
assert localtime_now.tm_hour >= 0
assert localtime_now.tm_hour <= 23
assert localtime_now.tm_min >= 0
assert localtime_now.tm_min <= 59
assert localtime_now.tm_sec >= 0
assert localtime_now.tm_sec <= 61  # 60-61 for leap seconds
assert localtime_now.tm_wday >= 0
assert localtime_now.tm_wday <= 6
assert localtime_now.tm_yday >= 1
assert localtime_now.tm_yday <= 366
assert localtime_now.tm_isdst >= -1
assert localtime_now.tm_isdst <= 1

# Test time.localtime(seconds) with specific timestamp
# 1700000000 = Nov 14, 2023, 21:13:20 UTC
localtime_specific: time.struct_time = time.localtime(1700000000.0)
print("localtime(1700000000) tm_year:", localtime_specific.tm_year)
assert localtime_specific.tm_year == 2023
assert localtime_specific.tm_mon == 11  # November

# Test time.gmtime()
gmtime_now: time.struct_time = time.gmtime()
print("gmtime() tm_year:", gmtime_now.tm_year)
assert gmtime_now.tm_year >= 2024
assert gmtime_now.tm_mon >= 1
assert gmtime_now.tm_mon <= 12

# Test time.gmtime(seconds) with epoch
# Unix epoch = Jan 1, 1970, 00:00:00 UTC
gmtime_epoch: time.struct_time = time.gmtime(0.0)
print("gmtime(0) tm_year:", gmtime_epoch.tm_year)
assert gmtime_epoch.tm_year == 1970
assert gmtime_epoch.tm_mon == 1
assert gmtime_epoch.tm_mday == 1
assert gmtime_epoch.tm_hour == 0
assert gmtime_epoch.tm_min == 0
assert gmtime_epoch.tm_sec == 0
assert gmtime_epoch.tm_wday == 3  # Thursday (Monday=0)
assert gmtime_epoch.tm_yday == 1

# Test time.mktime() round-trip
ts_original: float = time.time()
lt_from_ts: time.struct_time = time.localtime(ts_original)
ts_back: float = time.mktime(lt_from_ts)
# Allow for small rounding differences (within 1 second)
ts_diff: float = ts_original - ts_back
if ts_diff < 0.0:
    ts_diff = ts_diff * -1.0
assert ts_diff < 1.0

# Test mktime with known value
# Create struct_time from epoch and convert back
gmtime_for_mktime: time.struct_time = time.gmtime(86400.0)  # Jan 2, 1970 UTC
# Note: mktime treats input as local time, so result depends on timezone
mktime_result: float = time.mktime(gmtime_for_mktime)
# Just verify it returns a reasonable float
assert mktime_result != 0.0

# Test time.strftime()
strftime_lt: time.struct_time = time.localtime()
strftime_result: str = time.strftime("%Y-%m-%d", strftime_lt)
print("strftime result:", strftime_result)
# Verify format: YYYY-MM-DD (10 chars with dashes)
assert len(strftime_result) == 10
assert strftime_result[4] == "-"
assert strftime_result[7] == "-"

# Test strftime with specific timestamp
strftime_epoch: time.struct_time = time.gmtime(0.0)
strftime_epoch_result: str = time.strftime("%Y-%m-%d %H:%M:%S", strftime_epoch)
print("strftime epoch:", strftime_epoch_result)
assert strftime_epoch_result == "1970-01-01 00:00:00"

# Test time.strptime()
strptime_result: time.struct_time = time.strptime("2023-11-15", "%Y-%m-%d")
print("strptime year:", strptime_result.tm_year)
assert strptime_result.tm_year == 2023
assert strptime_result.tm_mon == 11
assert strptime_result.tm_mday == 15

# Test strptime with time
strptime_datetime: time.struct_time = time.strptime("2026-02-02 14:30:45", "%Y-%m-%d %H:%M:%S")
assert strptime_datetime.tm_year == 2026
assert strptime_datetime.tm_mon == 2
assert strptime_datetime.tm_mday == 2
assert strptime_datetime.tm_hour == 14
assert strptime_datetime.tm_min == 30
assert strptime_datetime.tm_sec == 45

# Test strftime/strptime round-trip
roundtrip_lt: time.struct_time = time.localtime()
roundtrip_str: str = time.strftime("%Y-%m-%d %H:%M:%S", roundtrip_lt)
roundtrip_parsed: time.struct_time = time.strptime(roundtrip_str, "%Y-%m-%d %H:%M:%S")
assert roundtrip_parsed.tm_year == roundtrip_lt.tm_year
assert roundtrip_parsed.tm_mon == roundtrip_lt.tm_mon
assert roundtrip_parsed.tm_mday == roundtrip_lt.tm_mday
assert roundtrip_parsed.tm_hour == roundtrip_lt.tm_hour
assert roundtrip_parsed.tm_min == roundtrip_lt.tm_min
assert roundtrip_parsed.tm_sec == roundtrip_lt.tm_sec

# Test from import for new functions
from time import localtime, gmtime, mktime, strftime, strptime
lt_imported: time.struct_time = localtime()
assert lt_imported.tm_year >= 2024

gt_imported: time.struct_time = gmtime()
assert gt_imported.tm_year >= 2024

print("All time module tests passed!")
