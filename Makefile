.PHONY: run build clean test check fmt clippy

run:
	cargo run

build:
	cargo build

release:
	cargo build --release

clean:
	cargo clean

test:
	cargo test

check:
	cargo check

fmt:
	cargo fmt

clippy:
	cargo clippy

install:
	cargo install --path .

help:
	@echo "Available targets:"
	@echo "  run      - Run the application"
	@echo "  build    - Build the application in debug mode"
	@echo "  release  - Build the application in release mode"
	@echo "  clean    - Clean build artifacts"
	@echo "  test     - Run tests"
	@echo "  check    - Check code without building"
	@echo "  fmt      - Format code"
	@echo "  clippy   - Run clippy linter"
	@echo "  install  - Install the binary"
	@echo "  help     - Show this help message"