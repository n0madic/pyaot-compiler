#!/usr/bin/env python3
"""Fallback timer used by run.sh when hyperfine is unavailable.

Usage: _timer.py <warmup> <runs> <cmd> [args...]
Prints the mean wall-clock seconds of <runs> measured executions to stdout.
"""
from __future__ import annotations

import subprocess
import sys
import time


def main() -> int:
    warmup = int(sys.argv[1])
    runs = int(sys.argv[2])
    cmd = sys.argv[3:]
    for _ in range(warmup):
        subprocess.run(cmd, stdout=subprocess.DEVNULL, check=True)
    samples = []
    for _ in range(runs):
        start = time.perf_counter()
        subprocess.run(cmd, stdout=subprocess.DEVNULL, check=True)
        samples.append(time.perf_counter() - start)
    print(f"{sum(samples) / len(samples):.6f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
