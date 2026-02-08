#!/bin/bash
set -e

echo "Building compiler..."
cargo build --workspace --release

echo ""
echo "Running example tests..."
echo ""

for file in examples/test_*.py; do
    name=$(basename "$file" .py)
    expected_file="examples/${name}.expected"

    echo "=== Testing $name ==="
    ./target/release/pyaot "$file" -o "/tmp/$name"

    # Check if .expected file exists for output comparison
    if [ -f "$expected_file" ]; then
        "/tmp/$name" > "/tmp/${name}.actual" 2>&1
        exit_code=$?
        if [ $exit_code -ne 0 ]; then
            echo "✗ $name failed with exit code $exit_code"
            cat "/tmp/${name}.actual"
            exit 1
        fi
        # Compare output
        if diff -u "$expected_file" "/tmp/${name}.actual"; then
            echo "✓ $name passed (output matched)"
        else
            echo "✗ $name failed: output mismatch"
            exit 1
        fi
    else
        # No .expected file: just check exit code
        "/tmp/$name"
        exit_code=$?
        if [ $exit_code -eq 0 ]; then
            echo "✓ $name passed"
        else
            echo "✗ $name failed with exit code $exit_code"
            exit 1
        fi
    fi
    echo ""
done

echo "All tests passed!"
