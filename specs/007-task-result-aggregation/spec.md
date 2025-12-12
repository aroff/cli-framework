# Feature Specification: Task Result Aggregation

**Feature Branch**: `007-task-result-aggregation`  
**Created**: 2025-01-27  
**Status**: Draft  
**Input**: Enhancement to simplify result aggregation from batch operations

## Purpose

Enable CLI applications built with the framework to easily aggregate and summarize results from batch task operations without writing manual aggregation logic. Applications should be able to collect statistics (success/failure counts, error summaries) and generate user-friendly reports automatically.

This feature addresses the common need for batch operations where applications must process multiple items and provide clear feedback to users about overall outcomes, success rates, and specific failures. Without this feature, developers must manually track counts, collect errors, and format summaries, which is error-prone and time-consuming.

## Clarifications

### Session 2025-01-27

- Q: How should cancelled tasks be handled in result aggregation? → A: Track cancelled tasks separately with a cancelled count (consistent with batch task management). Cancelled tasks are excluded from success rate calculations, which are based on successful vs failed tasks only.
- Q: When errors are filtered out during aggregation, what should happen to them? → A: Filtered errors are excluded from failure counts but still collected in a separate collection for reference, allowing applications to distinguish between "real" failures and filtered/ignored errors while maintaining complete auditability.
- Q: Should the system enforce a limit on error collection for very large batches, or should it attempt to collect all errors regardless of batch size? → A: Collect all errors by default, but allow applications to opt-in to error collection limits when needed. This provides flexibility: small batches get complete error information, while large batches can opt for memory-efficient operation when needed.
- Q: How should task identification be included in formatted error output? → A: Always include task identifier in each error line (use provided identifier or fallback to positional index). Format example: "1. [file: image.jpg] Error message" or "1. [Task 0] Error message" to provide clear context for each error.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Batch Processing Summary (Priority: P1)

A developer building a CLI tool needs to process multiple files and report a summary to users showing how many files succeeded, how many failed, and details about failures. For example, after processing 45 files, users should see "Processed 45 files, 3 failed" along with details of which files failed and why.

**Why this priority**: This is the most common use case - developers need to provide clear feedback to users about batch operation outcomes. Without aggregation utilities, developers must write repetitive code to count successes, collect errors, and format summaries.

**Current limitation**: Developers must manually iterate through results, count successes and failures, collect errors, and format summary messages. This code is repetitive and error-prone.

**Independent Test**:  
Given a batch operation that processes 45 files with 3 failures, a developer can:
- Automatically receive aggregated statistics (total: 45, successful: 42, failed: 3)
- Access all error messages from failed operations
- Generate a formatted summary message without manual string formatting
- Check if all operations succeeded with a simple boolean check

**Acceptance Scenarios**:

1. **Given** a batch operation processes 45 files with 3 failures, **When** results are aggregated, **Then** the developer receives statistics showing 45 total, 42 successful, 3 failed, and can access all 3 error messages.
2. **Given** a batch operation completes, **When** the developer requests a summary message, **Then** they receive a formatted message like "Completed 45 tasks: 42 successful, 3 failed (93.3% success rate)".
3. **Given** a batch operation where all tasks succeed, **When** results are aggregated, **Then** the developer can check that all succeeded and receive a summary message indicating complete success.

---

### User Story 2 - Migration Operations Reporting (Priority: P1)

A developer building a CLI tool needs to perform database migrations or data transformations and report total operations, successes, failures, and collect all errors for detailed reporting. Users need to understand the overall migration outcome and identify specific operations that failed.

**Why this priority**: Migration operations are critical and users need comprehensive reporting to understand what succeeded, what failed, and why. Detailed error collection enables troubleshooting and retry strategies.

**Current limitation**: Developers must manually track migration results, collect errors, and format reports. This is especially important for migrations where partial failures require careful analysis.

**Independent Test**:

- Execute a migration operation that processes 100 records with 5 failures
- Verify that aggregated results show correct counts (100 total, 95 successful, 5 failed)
- Verify that all 5 error messages are collected and accessible
- Verify that success rate is calculated correctly (95%)
- Verify that error formatting provides clear, readable output for users

**Acceptance Scenarios**:

1. **Given** a migration operation processes 100 records with 5 failures, **When** results are aggregated, **Then** the developer receives complete statistics and can format all errors for user display.
2. **Given** a migration operation completes, **When** the developer generates a summary, **Then** users see both high-level statistics and detailed error information for troubleshooting.
3. **Given** multiple migration operations run sequentially, **When** results are merged, **Then** the developer receives combined statistics and all errors from all operations.

---

### User Story 3 - Bulk API Operations Tracking (Priority: P2)

A developer building a CLI tool needs to perform bulk API operations (updates, creates, deletes) and track success/failure rates to provide detailed error reporting. Users need to understand which specific API operations failed and why, especially when dealing with rate limits or partial failures.

**Why this priority**: Bulk API operations often have partial failures due to rate limits, validation errors, or network issues. Users need clear reporting to understand what succeeded and what needs to be retried.

**Current limitation**: Developers must manually track API call results, collect errors, and calculate success rates. This is repetitive and error-prone, especially when dealing with large batches.

**Independent Test**:

- Execute 200 API operations with 10 failures
- Verify that aggregated results correctly show success rate (95%)
- Verify that all errors are collected and can be formatted for display
- Verify that developers can check if all operations succeeded
- Verify that error formatting provides actionable information for users

**Acceptance Scenarios**:

1. **Given** 200 API operations are executed with 10 failures, **When** results are aggregated, **Then** the developer receives statistics showing 95% success rate and can access all 10 error messages.
2. **Given** API operations complete, **When** the developer formats errors for display, **Then** users see a clear list of which operations failed and why, enabling retry strategies.
3. **Given** multiple batches of API operations, **When** results are merged, **Then** the developer receives combined statistics across all batches.

---

### Edge Cases

1. **Empty Results**: When no tasks are executed, aggregation returns empty statistics (0 total, 0 successful, 0 failed, 0 cancelled) and indicates no tasks were executed.
2. **All Success**: When all tasks succeed, aggregation correctly shows 100% success rate and indicates all tasks completed successfully.
3. **All Failure**: When all tasks fail, aggregation correctly shows 0% success rate and collects all errors for reporting.
4. **Cancelled Tasks**: When tasks are cancelled, aggregation tracks cancelled count separately from failed count, and cancelled tasks are excluded from success rate calculations.
4. **Very Large Batches**: Aggregation handles large batches (1000+ tasks) correctly, calculating statistics and collecting errors without performance issues.
5. **Error Formatting**: Error formatting handles various error types and provides readable output even when errors contain technical details. Each formatted error includes task identification (provided identifier or positional index) for clear context.
6. **Success Rate Calculation**: Success rate calculation handles edge cases: zero total (returns 0%), all success (returns 100%), all failure (returns 0%), all cancelled (returns 0% when no successful or failed tasks). Cancelled tasks are excluded from success rate calculations.
7. **Merging Results**: Merging multiple batch results correctly combines statistics and error collections without duplicates or data loss.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST automatically aggregate task results into summary statistics including total count, successful count, failed count, cancelled count, and success rate percentage.

- **FR-002**: The system MUST collect all errors from failed tasks and make them accessible for detailed reporting. By default, all errors are collected regardless of batch size. Applications can opt-in to error collection limits for memory-efficient operation with very large batches.

- **FR-003**: The system MUST provide a mechanism to check if all tasks succeeded (boolean check) without manual comparison. This check returns true only when there are no failed tasks and no cancelled tasks.

- **FR-004**: The system MUST calculate success rate as a percentage (0.0 to 100.0) based on successful vs (successful + failed) tasks, excluding cancelled tasks from the calculation. Handles edge cases: zero total (returns 0%), all success (returns 100%), all failure (returns 0%), all cancelled (returns 0% when no successful or failed tasks).

- **FR-005**: The system MUST generate formatted summary messages that include total count, success/failure/cancelled counts, and success rate percentage.

- **FR-006**: The system MUST format errors for display in a user-friendly format that lists all errors with clear numbering and context. Each error line MUST include the task identifier (using provided identifier when available, or positional index as fallback) to enable users to identify which specific task failed.

- **FR-007**: The system MUST support custom summary messages that applications can provide to override auto-generated summaries.

- **FR-008**: The system MUST support filtering errors during aggregation based on custom criteria, allowing applications to exclude certain errors from failure counts. Filtered errors are excluded from failure counts but still collected in a separate collection for reference and auditability.

- **FR-009**: The system MUST support merging results from multiple batch operations into a single aggregated result with combined statistics and error collections.

- **FR-010**: The system MUST handle empty result collections gracefully, returning appropriate statistics (0 total, 0 successful, 0 failed, 0 cancelled) and indicating no tasks were executed.

- **FR-011**: The system MUST preserve error context and details when collecting errors, ensuring applications can identify which specific task failed and why.

- **FR-012**: The system MUST provide aggregation utilities that work seamlessly with existing batch task management functionality.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Developers can aggregate batch results with 80% less code compared to manual aggregation (measured by lines of code reduction in example implementations).

- **SC-002**: Aggregation correctly calculates statistics for batches ranging from 1 to 1,000,000 tasks with 100% accuracy in count calculations.

- **SC-003**: Success rate calculations are accurate to within 0.1% for all batch sizes and success/failure combinations.

- **SC-004**: Error collection preserves 100% of error messages from failed tasks without data loss or truncation when no collection limit is specified. When limits are applied, the system collects up to the specified limit and indicates if additional errors were truncated.

- **SC-005**: Summary message generation completes in under 10 milliseconds for batches up to 10,000 tasks.

- **SC-006**: Error formatting produces readable output that enables users to identify and address 100% of failures without requiring technical debugging.

- **SC-007**: Result merging correctly combines statistics and errors from up to 100 separate batch operations without duplicates or data loss.

- **SC-008**: Aggregation utilities can be integrated into existing CLI applications without breaking changes to existing functionality.

## Key Entities *(include if feature involves data)*

### Aggregated Result
Represents the outcome of batch task execution, including:
- Total number of tasks executed
- Number of successful tasks
- Number of failed tasks
- Number of cancelled tasks
- Collection of errors from failed tasks (included in failure count)
- Collection of filtered errors (excluded from failure count but preserved for reference)
- Calculated success rate percentage (0.0 to 100.0) based on successful vs (successful + failed), excluding cancelled tasks and filtered errors
- Optional custom summary message

### Error Collection
A collection of error messages from failed tasks, each preserving:
- Error message and context
- Task identification (always available: either provided identifier or positional index)
- Error details sufficient for troubleshooting

### Summary Message
A formatted text message that describes batch operation outcomes, including:
- Total task count
- Success and failure counts
- Success rate percentage
- Optional custom message override

## Assumptions

1. **Error Preservation**: All errors from failed tasks should be preserved for detailed reporting, though very large batches may require memory considerations.

2. **Summary Generation**: Auto-generated summary messages should be clear and user-friendly, with support for custom overrides when needed.

3. **Error Formatting**: Error formatting should prioritize readability and user-friendliness while preserving technical details for debugging.

4. **Statistics Accuracy**: Statistics calculations (counts, percentages) must be accurate and handle edge cases correctly.

5. **Integration**: Aggregation utilities work seamlessly with existing batch task management functionality without requiring changes to existing code.

6. **Memory Considerations**: By default, all errors are collected regardless of batch size. For very large batches (100,000+ tasks), applications can opt-in to error collection limits (e.g., first N errors) to prevent memory issues when needed.

## Dependencies

- Existing batch task management functionality that provides task results
- Existing error handling mechanisms that produce error messages
- Integration with CLI output utilities for formatting summaries and errors

## Out of Scope

- Progress reporting during batch execution (covered by separate feature: 004_progress_reporting.md)
- Automatic retry logic for failed tasks (individual tasks can use existing retry mechanisms)
- Error categorization or grouping beyond basic collection
- Persistence of aggregated results across application restarts
- Real-time aggregation updates during batch execution (aggregation occurs after completion)

## Testing Requirements

### Test Scenarios

1. **All Success**: Aggregate results where all tasks succeed, verify correct counts and 100% success rate.

2. **All Failure**: Aggregate results where all tasks fail, verify correct counts, 0% success rate, and all errors collected.

3. **Mixed Results**: Aggregate results with some successes and some failures, verify correct counts, accurate success rate, and all errors collected.

4. **Empty Results**: Handle empty result collections, verify appropriate statistics and summary message.

5. **Error Formatting**: Test error formatting with various error types, verify readable output.

6. **Summary Generation**: Test summary generation with different scenarios (all success, all failure, mixed), verify appropriate messages.

7. **Result Merging**: Test merging multiple batch results, verify combined statistics and error collections.

8. **Error Filtering**: Test aggregation with custom error filters, verify filtered errors are excluded from failure counts but still collected in a separate collection for reference.

9. **Large Batches**: Test aggregation with large batches (1000+ tasks), verify performance and accuracy.

10. **Success Rate Edge Cases**: Test success rate calculation with zero total, all success, all failure, all cancelled, and mixed scenarios.
11. **Cancelled Tasks**: Test aggregation with cancelled tasks, verify cancelled count is tracked separately and excluded from success rate calculations.

## Backward Compatibility

This feature enhances existing batch task management functionality with aggregation utilities. All new functionality is additive and does not modify existing behavior. Applications can adopt aggregation utilities incrementally without affecting existing code.

## Related Components

- Batch task management (003_batch_task_management.md) - provides task results for aggregation
- CLI output utilities (006_cli_output_utilities.md) - for formatting summaries and errors
- Background task manager - provides task execution and result collection

