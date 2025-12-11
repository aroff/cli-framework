# Tasks: HTTP Retry Integration

**Input**: Design documents from `/specs/005-http-retry-integration/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: TDD is mandatory per constitution. All test tasks must be written FIRST and must FAIL before implementation.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3, US4)
- Include exact file paths in descriptions

## Path Conventions

- **Single project**: `src/`, `tests/` at repository root
- Paths: `src/http_retry/`, `tests/unit/`, `tests/integration/`, `tests/contract/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project structure verification and test directory setup

- [X] T001 Verify project structure exists per implementation plan (src/http_retry/, tests/, examples/)
- [X] T002 [P] Create test directory structure: tests/unit/http_retry_client.rs, tests/unit/http_error_classification.rs, tests/integration/http_retry.rs, tests/contract/http_retry_api.rs
- [X] T003 [P] Create example directory: examples/http_retry_demo/main.rs
- [X] T004 [P] Create src/http_retry/ module directory with mod.rs placeholder

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: HTTP error classification utilities that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

### Tests for Foundational (Write FIRST, ensure they FAIL)

- [X] T005 [P] Contract test for http_errors::is_retryable_http_error() with network errors in tests/contract/http_retry_api.rs
- [X] T006 [P] Contract test for http_errors::is_retryable_http_error() with 5xx server errors in tests/contract/http_retry_api.rs
- [X] T007 [P] Contract test for http_errors::is_retryable_http_error() with 429 rate limit errors in tests/contract/http_retry_api.rs
- [X] T008 [P] Contract test for http_errors::is_retryable_http_error() with 4xx client errors (should return false) in tests/contract/http_retry_api.rs
- [X] T009 [P] Unit test for http_errors::is_timeout() helper function in tests/unit/http_error_classification.rs
- [X] T010 [P] Unit test for http_errors::is_connection_error() helper function in tests/unit/http_error_classification.rs

### Implementation for Foundational

- [X] T011 [P] Create src/http_retry/mod.rs with module documentation and http_errors module declaration
- [X] T012 [P] Implement http_errors::is_retryable_http_error() function that checks network errors, 5xx, and 429 in src/http_retry/mod.rs
- [X] T013 [P] Implement http_errors::is_retryable_http_error() logic to return false for 4xx errors (except 429) in src/http_retry/mod.rs
- [X] T014 [P] Implement http_errors::is_timeout() helper function in src/http_retry/mod.rs
- [X] T015 [P] Implement http_errors::is_connection_error() helper function in src/http_retry/mod.rs
- [X] T016 Export http_retry module in src/lib.rs (with feature gate or documentation note about reqwest dependency)
- [X] T016A [P] Verification test: http_errors::is_retryable_http_error() correctly classifies all error types (network, 5xx, 429, 4xx) in integration context in tests/integration/http_retry.rs

**Checkpoint**: Foundation ready - HTTP error classification utilities complete and tested. User story implementation can now begin.

---

## Phase 3: User Story 1 - Automatic Retry for Transient Network Failures (Priority: P1) 🎯 MVP

**Goal**: Enable automatic retry for HTTP requests that fail due to transient network errors, 5xx server errors, and 429 rate limits without requiring manual retry logic in application code.

**Independent Test**: Run a CLI command that makes HTTP requests to a mock server that fails initially then succeeds, and verify the request is automatically retried and eventually succeeds without application code changes.

### Tests for User Story 1 (Write FIRST, ensure they FAIL)

- [X] T017 [P] [US1] Contract test for RetryableHttpClient::new() constructor in tests/contract/http_retry_api.rs
- [X] T018 [P] [US1] Contract test for RetryableHttpClient::get() method signature in tests/contract/http_retry_api.rs
- [X] T019 [P] [US1] Integration test for successful request on first attempt (no retries) using wiremock in tests/integration/http_retry.rs
- [X] T020 [P] [US1] Integration test for automatic retry on transient network error (connection timeout) using wiremock in tests/integration/http_retry.rs
- [X] T021 [P] [US1] Integration test for automatic retry on 5xx server error using wiremock in tests/integration/http_retry.rs
- [X] T022 [P] [US1] Integration test for automatic retry on 429 rate limit error using wiremock in tests/integration/http_retry.rs
- [X] T023 [P] [US1] Integration test for exponential backoff delay between retries using wiremock in tests/integration/http_retry.rs
- [X] T023A [P] [US1] Integration test for request builder configuration (headers, body) before retry execution using wiremock in tests/integration/http_retry.rs
- [X] T024 [P] [US1] Unit test for RetryableHttpClient default policy (3 attempts, 1s initial, 10s max) in tests/unit/http_retry_client.rs
- [X] T024A [P] [US1] Integration test for retry policy timeout integration with AsyncRetryExecutor (policy timeout shorter than HTTP client timeout) using wiremock in tests/integration/http_retry.rs
- [X] T024B [P] [US1] Integration test for HTTP client timeout handling during retry attempts using wiremock in tests/integration/http_retry.rs
- [X] T024C [P] [US1] Integration test for AsyncRetryExecutor timeout support integration (verify AsyncRetryExecutor timeout works with HTTP retry) using wiremock in tests/integration/http_retry.rs
- [X] T024D [P] [US1] Integration test for SC-003: Verify failed requests retried within 10 seconds of initial failure (first retry attempt timing) using wiremock in tests/integration/http_retry.rs
- [X] T024E [P] [US1] Performance test for SC-004: Verify no performance degradation for successful requests (retry logic overhead negligible on first-attempt success, measure latency difference) in tests/integration/http_retry.rs

### Implementation for User Story 1

- [X] T025 [US1] Create src/http_retry/client.rs with module documentation
- [X] T026 [US1] Define RetryableHttpClient struct with fields (client: reqwest::Client, retry_executor: AsyncRetryExecutor, default_policy: RetryPolicy) in src/http_retry/client.rs
- [X] T027 [US1] Implement RetryableHttpClient::new() constructor with default policy (3 attempts, 1s initial, 10s max exponential backoff) in src/http_retry/client.rs
- [X] T028 [US1] Implement RetryableHttpClient::client() method to return reference to underlying client in src/http_retry/client.rs
- [X] T029 [US1] Implement RetryableHttpClient::execute_with_retry() generic method that wraps request builder execution with AsyncRetryExecutor in src/http_retry/client.rs
- [X] T030 [US1] Implement RetryableHttpClient::execute_with_retry() to use default error classifier (http_errors::is_retryable_http_error) in src/http_retry/client.rs
- [X] T031 [US1] Implement RetryableHttpClient::get() method using execute_with_retry in src/http_retry/client.rs
- [X] T032 [US1] Implement RetryableHttpClient::post() method using execute_with_retry in src/http_retry/client.rs
- [X] T033 [US1] Implement RetryableHttpClient::put() method using execute_with_retry in src/http_retry/client.rs
- [X] T034 [US1] Implement RetryableHttpClient::delete() method using execute_with_retry in src/http_retry/client.rs
- [X] T035 [US1] Export RetryableHttpClient from src/http_retry/mod.rs
- [X] T036 [US1] Add basic example demonstrating automatic retry in examples/http_retry_demo/main.rs

**Checkpoint**: At this point, User Story 1 should be fully functional - applications can make HTTP requests with automatic retry on transient failures. Test independently by running example with mock server that fails then succeeds.

---

## Phase 4: User Story 2 - Configurable Retry Policies (Priority: P2)

**Goal**: Enable developers to configure custom retry policies (max attempts, delay strategy, timeouts) for different HTTP requests, with support for global default and per-request override.

**Independent Test**: Run a CLI command that configures different retry policies (exponential backoff, fixed delay, no retries) and verify each policy is applied correctly to its respective requests.

### Tests for User Story 2 (Write FIRST, ensure they FAIL)

- [X] T037 [P] [US2] Contract test for RetryableHttpClient::with_policy() constructor in tests/contract/http_retry_api.rs
- [X] T038 [P] [US2] Contract test for RetryableHttpClient::execute_with_policy() method in tests/contract/http_retry_api.rs
- [X] T039 [P] [US2] Integration test for custom retry policy with exponential backoff using wiremock in tests/integration/http_retry.rs
- [X] T040 [P] [US2] Integration test for custom retry policy with fixed delay using wiremock in tests/integration/http_retry.rs
- [X] T041 [P] [US2] Integration test for custom retry policy with no retries using wiremock in tests/integration/http_retry.rs
- [X] T042 [P] [US2] Integration test for per-request policy override using wiremock in tests/integration/http_retry.rs
- [X] T043 [P] [US2] Unit test for RetryableHttpClient policy cloning for per-request overrides in tests/unit/http_retry_client.rs

### Implementation for User Story 2

- [X] T044 [US2] Implement RetryableHttpClient::with_policy() constructor that accepts custom RetryPolicy in src/http_retry/client.rs
- [X] T045 [US2] Implement RetryableHttpClient::execute_with_policy() method that accepts per-request policy override in src/http_retry/client.rs
- [X] T046 [US2] Ensure execute_with_policy() creates new AsyncRetryExecutor with overridden policy (cloned) in src/http_retry/client.rs
- [X] T047 [US2] Verify execute_with_policy() uses default error classifier unless custom classifier provided in src/http_retry/client.rs
- [X] T048 [US2] Add example demonstrating custom retry policies in examples/http_retry_demo/main.rs

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently - automatic retry with configurable policies.

---

## Phase 5: User Story 3 - Smart Error Classification (Priority: P2)

**Goal**: Automatically determine which HTTP errors should be retried (network errors, 5xx, 429) and which should not (4xx except 429), ensuring retries only happen when they might succeed.

**Independent Test**: Run a CLI command that makes HTTP requests returning different status codes (404, 401, 500, 429) and verify only retryable errors trigger retry logic.

### Tests for User Story 3 (Write FIRST, ensure they FAIL)

- [X] T049 [P] [US3] Integration test for 404 Not Found error (should NOT retry) using wiremock in tests/integration/http_retry.rs
- [X] T050 [P] [US3] Integration test for 401 Unauthorized error (should NOT retry) using wiremock in tests/integration/http_retry.rs
- [X] T051 [P] [US3] Integration test for 500 Internal Server Error (should retry) using wiremock in tests/integration/http_retry.rs
- [X] T052 [P] [US3] Integration test for request timeout error (should retry) using wiremock in tests/integration/http_retry.rs
- [X] T053 [P] [US3] Integration test for connection refused error (should retry) using wiremock in tests/integration/http_retry.rs
- [X] T054 [P] [US3] Unit test for error classification edge cases (invalid status codes, missing status) in tests/unit/http_error_classification.rs
- [X] T054A [P] [US3] Integration test for SC-006: Verify 100% error classification accuracy across all standard HTTP error scenarios (network errors, 4xx, 5xx, 429) using wiremock in tests/integration/http_retry.rs

### Implementation for User Story 3

- [X] T055 [US3] [VERIFICATION] Ensure RetryableHttpClient::execute_with_retry() uses http_errors::is_retryable_http_error() as default classifier (verify T016A passes) in src/http_retry/client.rs
- [X] T056 [US3] Ensure RetryableHttpClient::execute_with_retry() uses http_errors::is_retryable_http_error() as default classifier in src/http_retry/client.rs
- [X] T057 [US3] Add example demonstrating smart error classification in examples/http_retry_demo/main.rs

**Checkpoint**: At this point, User Stories 1, 2, AND 3 should all work independently - automatic retry with smart error classification.

---

## Phase 6: User Story 4 - Custom Error Classification (Priority: P3)

**Goal**: Enable developers to provide custom error classification logic to override default behavior for domain-specific retry requirements.

**Independent Test**: Run a CLI command that provides a custom error classifier that marks 404 as retryable, and verify the custom classifier is used to determine retry behavior.

### Tests for User Story 4 (Write FIRST, ensure they FAIL)

- [X] T058 [P] [US4] Contract test for RetryableHttpClient::execute_with_classifier() method in tests/contract/http_retry_api.rs
- [X] T059 [P] [US4] Integration test for custom error classifier that marks 404 as retryable using wiremock in tests/integration/http_retry.rs
- [X] T060 [P] [US4] Integration test for custom error classifier that only retries on 503 using wiremock in tests/integration/http_retry.rs
- [X] T061 [P] [US4] Unit test for custom error classifier thread safety (Send + Sync) in tests/unit/http_retry_client.rs

### Implementation for User Story 4

- [X] T062 [US4] Implement RetryableHttpClient::execute_with_classifier() method that accepts custom error classifier function in src/http_retry/client.rs
- [X] T063 [US4] Ensure execute_with_classifier() enforces Send + Sync bounds on classifier function in src/http_retry/client.rs
- [X] T064 [US4] Verify execute_with_classifier() uses custom classifier instead of default in src/http_retry/client.rs
- [X] T065 [US4] Add example demonstrating custom error classification in examples/http_retry_demo/main.rs

**Checkpoint**: At this point, User Stories 1, 2, 3, AND 4 should all work independently - full HTTP retry integration with custom error classification.

---

## Phase 7: Retry-After Header Support (FR-003)

**Goal**: Respect Retry-After header from 429 responses, using header delay when present or falling back to policy delay.

**Independent Test**: Run a CLI command that receives 429 response with Retry-After header and verify the retry delay matches the header value.

### Tests for Retry-After Support (Write FIRST, ensure they FAIL)

- [X] T066 [P] Integration test for 429 response with Retry-After header (seconds format) using wiremock in tests/integration/http_retry.rs
- [X] T067 [P] Integration test for 429 response with Retry-After header (HTTP-date format) using wiremock in tests/integration/http_retry.rs
- [X] T068 [P] Integration test for 429 response without Retry-After header (fallback to policy delay) using wiremock in tests/integration/http_retry.rs
- [X] T069 [P] Unit test for Retry-After header parsing (seconds integer) in tests/unit/http_retry_client.rs
- [X] T070 [P] Unit test for Retry-After header parsing (HTTP-date) in tests/unit/http_retry_client.rs
- [X] T071 [P] Unit test for Retry-After header parsing error handling (fallback to policy) in tests/unit/http_retry_client.rs

### Implementation for Retry-After Support

- [X] T072 Implement parse_retry_after_header() helper function that parses Retry-After header (seconds or HTTP-date) in src/http_retry/client.rs
- [X] T073 Modify execute_with_retry() to check for Retry-After header in 429 responses and use parsed delay if present in src/http_retry/client.rs
- [X] T074 Ensure Retry-After header parsing errors fall back to policy delay gracefully in src/http_retry/client.rs
- [X] T075 Add example demonstrating Retry-After header handling in examples/http_retry_demo/main.rs

---

## Phase 8: Logging Support (FR-016)

**Goal**: Log retry attempts with configurable verbosity levels (debug/info/warn) using standard Rust logging.

**Independent Test**: Run a CLI command with logging enabled and verify retry attempts are logged at appropriate levels.

### Tests for Logging Support (Write FIRST, ensure they FAIL)

- [X] T076 [P] Unit test for retry attempt logging at debug level in tests/unit/http_retry_client.rs
- [X] T077 [P] Unit test for retry attempt logging at info level in tests/unit/http_retry_client.rs
- [X] T078 [P] Unit test for retry exhaustion logging at warn level in tests/unit/http_retry_client.rs
- [X] T079 [P] Integration test for logging integration with env_logger in tests/integration/http_retry.rs

### Implementation for Logging Support

- [X] T080 Add log dependency to Cargo.toml (if not already present)
- [X] T081 Implement retry attempt logging using log::debug!() in RetryableHttpClient::execute_with_retry() in src/http_retry/client.rs
- [X] T082 Implement retry start/completion logging using log::info!() in src/http_retry/client.rs
- [X] T083 Implement retry exhaustion logging using log::warn!() when all attempts fail in src/http_retry/client.rs
- [X] T084 Add example demonstrating logging configuration in examples/http_retry_demo/main.rs

---

## Phase 9: Additional HTTP Methods (FR-013)

**Goal**: Support all standard HTTP methods (PATCH, HEAD, OPTIONS) in addition to GET, POST, PUT, DELETE.

**Independent Test**: Run a CLI command that uses PATCH, HEAD, and OPTIONS methods and verify they work with retry logic.

### Tests for Additional HTTP Methods (Write FIRST, ensure they FAIL)

- [X] T085 [P] Integration test for PATCH method with retry using wiremock in tests/integration/http_retry.rs
- [X] T086 [P] Integration test for HEAD method with retry using wiremock in tests/integration/http_retry.rs
- [X] T087 [P] Integration test for OPTIONS method with retry using wiremock in tests/integration/http_retry.rs

### Implementation for Additional HTTP Methods

- [X] T088 Implement RetryableHttpClient::patch() method using execute_with_retry in src/http_retry/client.rs
- [X] T089 Implement RetryableHttpClient::head() method using execute_with_retry in src/http_retry/client.rs
- [X] T090 Implement RetryableHttpClient::options() method using execute_with_retry in src/http_retry/client.rs

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, examples, edge cases, and final integration

### Documentation

- [ ] T091 Add comprehensive doc comments to all public API methods in src/http_retry/client.rs with usage examples
- [ ] T092 Add module-level documentation explaining idempotency risks for non-idempotent HTTP methods in src/http_retry/mod.rs
- [ ] T093 Update framework documentation to note reqwest dependency requirement (feature gate or documentation note)

### Examples

- [ ] T094 Complete examples/http_retry_demo/main.rs with comprehensive examples demonstrating all features
- [ ] T095 Add example demonstrating request builder configuration (headers, body) before retry (verify T023A test pattern, FR-014) in examples/http_retry_demo/main.rs

### Edge Cases & Error Handling

- [ ] T096 Handle request cancellation during retry delay (return cancellation error) in src/http_retry/client.rs
- [ ] T097 Verify concurrent requests with same retry policy work independently (inherent in design - no shared state, each request has own retry executor instance) in tests/integration/http_retry.rs
- [ ] T098 Verify retry policy timeout respects HTTP client timeout when both configured (verify T024A, T024B, and T024C pass, FR-012) in tests/integration/http_retry.rs
- [ ] T099 Verify all retry attempts exhausted returns last error (FR-011) in tests/integration/http_retry.rs
- [ ] T100 Verify first successful response returned immediately without additional retries (FR-010) in tests/integration/http_retry.rs
- [ ] T100A Integration test for SC-002: Statistical test for 95%+ recovery rate from transient network failures (run 100 requests with transient failures, verify >=95 succeed after retries) in tests/integration/http_retry.rs

### Code Quality

- [ ] T101 Run cargo fmt on all new files
- [ ] T102 Run cargo clippy and fix all warnings
- [ ] T103 Ensure all tests pass: cargo test
- [ ] T104 Verify feature is opt-in (existing applications unaffected) - no breaking changes

---

## Dependencies

### Story Completion Order

1. **Phase 2 (Foundational)** → Must complete before all user stories
2. **Phase 3 (US1 - P1)** → MVP, can be completed independently
3. **Phase 4 (US2 - P2)** → Depends on US1 (uses RetryableHttpClient from US1)
4. **Phase 5 (US3 - P2)** → Depends on US1 (uses RetryableHttpClient from US1)
5. **Phase 6 (US4 - P3)** → Depends on US3 (extends error classification from US3)
6. **Phase 7 (Retry-After)** → Depends on US1 (extends retry logic)
7. **Phase 8 (Logging)** → Depends on US1 (adds logging to retry logic)
8. **Phase 9 (Additional Methods)** → Depends on US1 (adds more HTTP methods)
9. **Phase 10 (Polish)** → Depends on all previous phases

### Parallel Execution Opportunities

**Within Phase 2 (Foundational)**:
- T005-T010, T016A (all contract/unit tests) can run in parallel
- T011-T015 (all implementation tasks) can run in parallel

**Within Phase 3 (US1)**:
- T017-T024E (all tests) can run in parallel
- T031-T034 (HTTP method implementations) can run in parallel

**Within Phase 4 (US2)**:
- T037-T043 (all tests) can run in parallel

**Within Phase 5 (US3)**:
- T049-T054A (all tests) can run in parallel

**Within Phase 6 (US4)**:
- T058-T061 (all tests) can run in parallel

**Across Phases** (after dependencies met):
- Phase 4 (US2) and Phase 5 (US3) can run in parallel (both depend on US1)
- Phase 7 (Retry-After) and Phase 8 (Logging) can run in parallel (both depend on US1)

## Implementation Strategy

### MVP Scope (Minimum Viable Product)

**Suggested MVP**: Phase 2 (Foundational) + Phase 3 (US1)

This delivers the core value proposition:
- Automatic retry for transient network failures
- Default retry policy (3 attempts, exponential backoff)
- Smart error classification (network/5xx/429 retryable, 4xx not)
- Basic HTTP methods (GET, POST, PUT, DELETE)

**MVP Independent Test**: Run a CLI command that makes HTTP requests to a mock server that fails initially then succeeds, and verify the request is automatically retried and eventually succeeds.

### Incremental Delivery

1. **MVP**: Phase 2 + Phase 3 (US1) - Core automatic retry
2. **Increment 1**: Phase 4 (US2) - Configurable policies
3. **Increment 2**: Phase 5 (US3) - Smart error classification (mostly done in foundational, verify integration)
4. **Increment 3**: Phase 6 (US4) - Custom error classification
5. **Increment 4**: Phase 7 + Phase 8 - Retry-After header and logging
6. **Increment 5**: Phase 9 - Additional HTTP methods
7. **Final**: Phase 10 - Polish and documentation

## Summary

- **Total Tasks**: 113
- **Tasks per User Story**:
  - US1 (P1): 27 tasks (T017-T036, T023A, T024A-T024E)
  - US2 (P2): 12 tasks (T037-T048)
  - US3 (P2): 10 tasks (T049-T057, T054A)
  - US4 (P3): 8 tasks (T058-T065)
  - Retry-After: 10 tasks (T066-T075)
  - Logging: 9 tasks (T076-T084)
  - Additional Methods: 6 tasks (T085-T090)
  - Polish: 15 tasks (T091-T104, T100A)
  - Setup/Foundational: 17 tasks (T001-T016, T016A)
- **Parallel Opportunities**: Many test tasks can run in parallel within each phase
- **MVP Scope**: Phase 2 + Phase 3 (US1) - 44 tasks total
- **Independent Test Criteria**: Each user story has clear independent test criteria defined

