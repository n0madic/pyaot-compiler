.PHONY: build test clean runtime cli all

all: build

# Build everything
build:
	cargo build --workspace --release

# Build only the runtime library
runtime:
	cargo build -p pyaot-runtime --release

# Build only the CLI
cli:
	cargo build -p pyaot --release

# Run tests
test:
	cargo test --workspace

# Run clippy
lint:
	cargo clippy --workspace -- -D warnings

# Format code
fmt:
	cargo fmt --all

# Clean build artifacts
clean:
	cargo clean

# Build and run an example
example:
	@echo "Building runtime..."
	@cargo build -p pyaot-runtime --release
	@echo "Building compiler..."
	@cargo build -p pyaot --release
	@echo "Compiling example..."
	@./target/release/pyaot examples/hello.py -o hello_out --verbose

# Check that everything compiles
check:
	cargo check --workspace

# Generate documentation
docs:
	cargo doc --workspace --no-deps --open
