# Contributing to TUI Framework

Thank you for your interest in contributing to TUI Framework! This document provides guidelines and instructions for contributing.

## Development Setup

1. Clone the repository
2. Install Rust toolchain (latest stable)
3. Run tests: `cargo test`
4. Build: `cargo build`

## Code Style

- Follow Rust standard formatting: `cargo fmt`
- Run clippy: `cargo clippy`
- Ensure all tests pass: `cargo test`
- Ensure examples compile: `cargo build --examples`

## Testing

The project follows a test-first approach:

- **Unit Tests**: Test individual components in `src/`
- **Integration Tests**: Test component interactions in `tests/`
- **Contract Tests**: Verify trait implementations comply with contracts in `tests/contract/`
- **Examples**: Serve as integration tests and documentation

Run all tests:
```bash
cargo test
```

Run specific test suites:
```bash
cargo test --lib              # Unit tests only
cargo test --test keymap      # Integration tests
cargo test contract           # Contract tests
```

## Project Structure

```
cli-framework/
├── src/              # Source code
│   ├── app/         # Application builder and runtime
│   ├── command/     # Command system
│   ├── data_source/ # Data source trait
│   ├── keymap/      # Keybinding system
│   ├── view/        # View trait and theme
│   ├── widget/      # Standard widgets (GridView, LogView, etc.)
│   ├── message/     # Message system
│   ├── auth/        # Optional authentication
│   ├── retry/       # Retry policies
│   └── observability/ # OpenTelemetry integration
├── tests/           # Test suites
│   ├── unit/        # Unit tests
│   ├── integration/ # Integration tests
│   └── contract/    # Contract tests
├── examples/        # Example applications
└── specs/           # Specification documents
```

## Making Changes

1. **Create a branch**: `git checkout -b feature/your-feature-name`
2. **Write tests first** (test-first approach)
3. **Implement the feature**
4. **Ensure all tests pass**
5. **Update documentation** if needed
6. **Commit with clear messages**

## Commit Messages

Follow conventional commit format:
- `feat: add new feature`
- `fix: fix bug`
- `docs: update documentation`
- `test: add tests`
- `refactor: refactor code`

## Pull Request Process

1. Ensure all tests pass
2. Ensure examples compile
3. Update documentation if needed
4. Create a pull request with a clear description
5. Reference any related issues

## Design Principles

The framework follows these principles:

1. **Opinionated Defaults**: Provide sensible defaults that work for most cases
2. **Flexibility**: Allow customization where needed
3. **Test-First**: Write tests before implementation
4. **Documentation**: Public API must be documented
5. **Error Handling**: Use `anyhow::Result` for framework operations

## Areas for Contribution

- Bug fixes
- Performance improvements
- Additional widgets
- Documentation improvements
- Example applications
- Test coverage improvements

## Questions?

Open an issue for questions or discussions about contributions.

