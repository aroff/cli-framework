# Bugs Report: CLI Framework

This document reports the bugs and critical omissions identified during the codebase scan of the CLI Framework.

## 1. Omission of `App::run()` Method
The `App` struct in `src/app/builder.rs` is missing the `run()` method, which is mentioned in `README.md` and method comments in `AppBuilder::build()`. This makes it impossible for application developers to follow the documented "Quick Start" examples.

- **File:** `src/app/builder.rs`
- **Impact:** Critical (Breaking change / Omission)
- **Status:** Not implemented

## 2. Unimplemented Anthropic LLM Provider
The Anthropic provider implementation for command resolution is currently a placeholder and returns an error for all resolution requests.

- **File:** `src/llm/anthropic.rs`
- **Impact:** High (Broken feature)
- **Status:** `TODO: Implement Anthropic API integration`

## 3. Simulated `ailoop` Integration
The `ailoop-core` integration in `src/ailoop/mod.rs` only simulates human-in-the-loop interactions. It prints messages to the console but doesn't actually wait for responses or integrate with an external ailoop server.

- **File:** `src/ailoop/mod.rs`
- **Impact:** High (Broken core feature)
- **Status:** Simulated approval / `TODO: Real implementation`

## 4. Potential Panic in JSON Response Parsing
The LLM response parsing logic in both OpenAI and Anthropic providers uses risky string slicing (`response[json_start..=json_end]`). This will panic if:
1. No opening brace `{` is found (though `unwrap_or(0)` handles this).
2. No closing brace `}` is found (though `unwrap_or(response.len())` handles this).
3. **Critical:** If the last brace `}` appears before the first brace `{`, or if they are missing and `response.len()` is 0, the range `json_start..=json_end` will panic.

- **Files:** `src/llm/openai.rs`, `src/llm/anthropic.rs`
- **Impact:** Medium (Stability)

## 5. Documentation and Code Mismatch (TUI vs. CLI)
The `docs/getting-started.md` and significant portions of `README.md` refer to a "TUI Framework" and describe TUI-specific concepts (like `View`, `GridView`, `Frame`, and `ratatui` integration) which do not exist in this CLI Framework.

- **Files:** `docs/getting-started.md`, `README.md`
- **Impact:** High (Confusing for users)

## 6. Omission of "Ask" Command Registration
`AppBuilder::build` contains a note that "Ask command registration is deferred", but there is no code to actually register the `ask` command even if an LLM provider is configured.

- **File:** `src/app/builder.rs`
- **Impact:** Medium (Feature inaccessible)

## 7. Fragile `unwrap()` usage in `http_retry/client.rs`
While technically safe in the current flow, the `unwrap()` and `as_ref().unwrap()` calls on `last_error` in `src/http_retry/client.rs` (lines 212, 214, 246, 248) are non-idiomatic and could lead to panics if the code is refactored and the logic that ensures `last_error` is `Some` is changed.

- **File:** `src/http_retry/client.rs`
- **Impact:** Low (Code quality)
