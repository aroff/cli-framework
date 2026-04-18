# Architectural Enhancements: CLI Framework

This document proposes architectural enhancements for the CLI Framework to improve its flexibility, scalability, and maintainability.

## 1. Unified Logging Strategy
Replace all instances of `eprintln!` and `println!` in library code (especially in `PluginRegistryManager` and `ailoop`) with the `log` crate. This allows application developers to control the output via logging configurations (e.g., `env_logger`, `tracing`).

- **Target:** `src/plugin/registry.rs`, `src/ailoop/mod.rs`

## 2. Trait-Based Service Discovery
Instead of using internal wrappers like `CliAppContextWrapper`, implement a more robust service discovery or dependency injection system. This could involve an `Extension` trait that `AppContext` can implement to provide access to framework services like LLMs, ailoop, and telemetry.

- **Target:** `src/app/builder.rs`, `src/app/context.rs`

## 3. Robust Error Collection in Plugin Registry
Modify `PluginRegistryManager::load()` to return a result or a collection of errors (`Vec<anyhow::Error>`) instead of printing them. This allows the application to handle plugin loading failures gracefully (e.g., showing a warning in the UI).

- **Target:** `src/plugin/registry.rs`

## 4. Async Plugin Initialization
Support asynchronous plugin initialization. Currently, plugins are loaded synchronously from files. Adding an `init()` method to the plugin manifest that returns a `Future` would allow plugins to perform setup tasks (like network checks or DB connections) at startup.

- **Target:** `src/plugin/mod.rs`

## 5. Extensible LLM Provider Registry
The `LlmProviderFactory` currently has a hardcoded match statement for OpenAI and Anthropic. Refactor this into a registry system where custom providers can be registered at runtime, similar to the command registry.

- **Target:** `src/llm/mod.rs`

## 6. Command Lifecycle Hooks
Introduce pre-execution and post-execution hooks for commands. This would allow for cross-cutting concerns like:
- Execution timing (telemetry)
- Permission checks (RBAC)
- Automatic logging of command usage
- Context cleanup after execution

- **Target:** `src/command/mod.rs`, `src/app/builder.rs`

## 7. Formal State Management
Add a standard way to manage shared application state within the `App` struct. While `AppContext` is provided by the user, the framework could provide a synchronized `State` container (e.g., using `Arc<RwLock<T>>`) that integrates with the command execution flow.

- **Target:** `src/app/mod.rs`
