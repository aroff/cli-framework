//! Background task management for async operations
//!
//! Provides a system for spawning and managing long-running async operations
//! that update the UI when complete, without blocking the event loop.
//!
//! ## Batch Task Management
//!
//! The framework supports batch task operations for processing multiple tasks concurrently
//! with configurable concurrency limits and comprehensive result aggregation.
//!
//! ### Basic Usage
//!
//! ```rust,no_run
//! use tui_framework::app::background_tasks::{BackgroundTaskManager, task_definition};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let mut manager = BackgroundTaskManager::new();
//!
//! // Create a batch of tasks
//! let tasks = vec![
//!     task_definition(
//!         || async { Ok("result1") },
//!         Some("task1".to_string()),
//!     ),
//!     task_definition(
//!         || async { Ok("result2") },
//!         Some("task2".to_string()),
//!     ),
//! ];
//!
//! // Spawn batch with concurrency limit of 5
//! let result = manager.spawn_batch(tasks, Some(5)).await;
//!
//! println!("Processed {} tasks: {} succeeded, {} failed",
//!     result.total, result.successful, result.failed);
//! # Ok(())
//! # }
//! ```
//!
//! ### Concurrency Control
//!
//! The framework automatically detects CPU cores and uses a default concurrency limit
//! of `CPU cores * 2`. You can override this with a custom limit (maximum 100):
//!
//! ```rust,no_run
//! # use tui_framework::app::background_tasks::{BackgroundTaskManager, task_definition};
//! # async fn example() -> anyhow::Result<()> {
//! # let mut manager = BackgroundTaskManager::new();
//! # let tasks = vec![];
//! // Use default limit (CPU-based)
//! let result = manager.spawn_batch(tasks.clone(), None).await;
//!
//! // Use custom limit
//! let result = manager.spawn_batch(tasks, Some(10)).await;
//! # Ok(())
//! # }
//! ```

use anyhow::Result;
use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

/// Result type for background tasks (legacy single-task API)
pub type TaskResult = Result<()>;

/// Progress update from a background task
///
/// Represents the current state of a long-running operation's progress,
/// including current item count, optional total count, and optional contextual message.
#[derive(Debug, Clone)]
pub struct ProgressReporter {
    /// Current item number (0-indexed or 1-indexed, application choice)
    pub current: usize,
    /// Total number of items (None for indeterminate progress)
    pub total: Option<usize>,
    /// Optional contextual message describing current operation
    pub message: Option<String>,
}

impl ProgressReporter {
    /// Create a new progress reporter with current and total counts
    ///
    /// # Arguments
    ///
    /// * `current` - Current item number
    /// * `total` - Total number of items (must be > 0)
    ///
    /// # Returns
    ///
    /// `ProgressReporter` with `message: None`
    ///
    /// # Example
    ///
    /// ```rust
    /// use tui_framework::app::background_tasks::ProgressReporter;
    ///
    /// let progress = ProgressReporter::new(45, 200);
    /// assert_eq!(progress.current, 45);
    /// assert_eq!(progress.total, Some(200));
    /// ```
    pub fn new(current: usize, total: usize) -> Self {
        Self {
            current,
            total: Some(total),
            message: None,
        }
    }

    /// Create a progress reporter with current, total, and contextual message
    ///
    /// # Arguments
    ///
    /// * `current` - Current item number
    /// * `total` - Total number of items (must be > 0)
    /// * `message` - Contextual message describing current operation
    ///
    /// # Returns
    ///
    /// `ProgressReporter` with message set
    ///
    /// # Example
    ///
    /// ```rust
    /// use tui_framework::app::background_tasks::ProgressReporter;
    ///
    /// let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
    /// assert_eq!(progress.message, Some("Processing file.jpg".to_string()));
    /// ```
    pub fn with_message(current: usize, total: usize, message: impl Into<String>) -> Self {
        Self {
            current,
            total: Some(total),
            message: Some(message.into()),
        }
    }

    /// Calculate completion percentage
    ///
    /// # Returns
    ///
    /// * `0.0` if `total` is `None` or `0`
    /// * `(current as f64 / total as f64) * 100.0` if current <= total
    /// * `100.0` if current > total (capped at 100%)
    ///
    /// # Example
    ///
    /// ```rust
    /// use tui_framework::app::background_tasks::ProgressReporter;
    ///
    /// let progress = ProgressReporter::new(45, 200);
    /// assert_eq!(progress.percentage(), 22.5);
    ///
    /// let progress = ProgressReporter::new(200, 150);  // > 100%
    /// assert_eq!(progress.percentage(), 100.0);  // Capped
    /// ```
    pub fn percentage(&self) -> f64 {
        match self.total {
            None | Some(0) => 0.0,
            Some(total) => {
                if self.current > total {
                    100.0
                } else {
                    (self.current as f64 / total as f64) * 100.0
                }
            }
        }
    }

    /// Check if progress is complete
    ///
    /// # Returns
    ///
    /// * `true` if `current >= total` and `total` is `Some` and `> 0`
    /// * `false` otherwise (including when `total` is `None`)
    ///
    /// # Example
    ///
    /// ```rust
    /// use tui_framework::app::background_tasks::ProgressReporter;
    ///
    /// let progress = ProgressReporter::new(200, 200);
    /// assert!(progress.is_complete());
    ///
    /// let progress = ProgressReporter::new(45, 200);
    /// assert!(!progress.is_complete());
    /// ```
    pub fn is_complete(&self) -> bool {
        match self.total {
            None | Some(0) => false,
            Some(total) => self.current >= total,
        }
    }
}

// ============================================================================
// Batch Task Management Types
// ============================================================================

/// Identifier for a task in batch operations
///
/// Tasks can be identified by an application-provided string or by their
/// positional index in the batch (0-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskIdentifier {
    /// Application-provided identifier (e.g., "file: image.jpg")
    Provided(String),
    /// Positional index in batch (0-based)
    Index(usize),
}

impl TaskIdentifier {
    /// Get display string for identifier
    pub fn display(&self) -> String {
        match self {
            TaskIdentifier::Provided(s) => s.clone(),
            TaskIdentifier::Index(i) => format!("task[{}]", i),
        }
    }
}

/// Execution status of a task
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task completed successfully
    Success,
    /// Task failed with an error
    Failure,
    /// Task was cancelled before completion
    Cancelled,
}

/// Result of a single task execution in a batch
#[derive(Debug)]
pub struct BatchTaskResult {
    /// Task identifier (provided or positional index)
    pub identifier: TaskIdentifier,
    /// Execution status
    pub status: TaskStatus,
    /// Optional return value for successful tasks
    pub value: Option<Box<dyn Any + Send>>,
    /// Error information for failed tasks
    pub error: Option<anyhow::Error>,
}

impl BatchTaskResult {
    /// Check if task succeeded
    pub fn is_success(&self) -> bool {
        matches!(self.status, TaskStatus::Success)
    }

    /// Check if task failed
    pub fn is_failure(&self) -> bool {
        matches!(self.status, TaskStatus::Failure)
    }

    /// Check if task was cancelled
    pub fn is_cancelled(&self) -> bool {
        matches!(self.status, TaskStatus::Cancelled)
    }

    /// Get error if task failed
    pub fn error(&self) -> Option<&anyhow::Error> {
        self.error.as_ref()
    }

    /// Get value if task succeeded (requires type downcast)
    pub fn value<T: 'static>(&self) -> Option<&T> {
        self.value.as_ref().and_then(|v| v.downcast_ref::<T>())
    }
}

/// Aggregated result of a batch operation
#[derive(Debug)]
pub struct BatchResult {
    /// Total number of tasks executed
    pub total: usize,
    /// Number of successful tasks
    pub successful: usize,
    /// Number of failed tasks
    pub failed: usize,
    /// Number of cancelled tasks
    pub cancelled: usize,
    /// Success rate as percentage (successful / (successful + failed))
    pub success_rate: f64,
    /// Collection of errors from failed tasks
    pub errors: Vec<(TaskIdentifier, anyhow::Error)>,
    /// Individual task results in completion order
    pub results: Vec<BatchTaskResult>,
}

impl BatchResult {
    /// Check if all tasks succeeded
    pub fn all_succeeded(&self) -> bool {
        self.failed == 0 && self.cancelled == 0
    }

    /// Check if any tasks failed
    pub fn has_failures(&self) -> bool {
        self.failed > 0
    }

    /// Check if any tasks were cancelled
    pub fn has_cancellations(&self) -> bool {
        self.cancelled > 0
    }

    /// Get errors from failed tasks
    pub fn errors(&self) -> &[(TaskIdentifier, anyhow::Error)] {
        &self.errors
    }

    /// Get individual task results
    pub fn results(&self) -> &[BatchTaskResult] {
        &self.results
    }
}

/// Definition of a task for batch processing
///
/// Contains the task closure and optional identifier for error reporting.
pub struct TaskDefinition {
    /// The async task to execute, returning a result with optional value
    task: Box<
        dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>>> + Send>>
            + Send
            + 'static,
    >,
    /// Optional task identifier for error reporting
    identifier: Option<String>,
}

impl TaskDefinition {
    /// Create a new task definition
    ///
    /// # Arguments
    /// * `task` - Async task function returning `Result<T>` where T: Send + 'static
    /// * `identifier` - Optional task identifier for error reporting
    pub fn new<F, Fut, T>(task: F, identifier: Option<String>) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        Self {
            task: Box::new(move || {
                Box::pin(async move {
                    task()
                        .await
                        .map(|value| Box::new(value) as Box<dyn Any + Send>)
                })
            }),
            identifier,
        }
    }

    /// Get the task identifier, if provided
    pub fn identifier(&self) -> Option<&String> {
        self.identifier.as_ref()
    }

    /// Take the task closure (consumes self)
    pub fn into_task(
        self,
    ) -> Box<
        dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>>> + Send>>
            + Send
            + 'static,
    > {
        self.task
    }
}

/// Create a task definition for batch processing
///
/// # Arguments
/// * `task` - Async task function returning `Result<T>` where T: Send + 'static
/// * `identifier` - Optional task identifier for error reporting
///
/// # Returns
/// `TaskDefinition` ready for batch execution
///
/// # Example
/// ```rust,no_run
/// use tui_framework::app::background_tasks::task_definition;
///
/// let task = task_definition(
///     || async { Ok(42) },
///     Some("task-1".to_string()),
/// );
/// ```
pub fn task_definition<F, Fut, T>(task: F, identifier: Option<String>) -> TaskDefinition
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    TaskDefinition::new(task, identifier)
}

/// Background task manager for spawning and tracking async operations
///
/// Manages the lifecycle of background tasks, including spawning, cancellation,
/// and result collection. Tasks can be cancelled when views switch or when
/// explicit cancellation is requested.
pub struct BackgroundTaskManager {
    /// Channel receiver for task results
    result_receiver: Option<mpsc::Receiver<TaskResult>>,
    /// Channel sender for task results (cloned for each task)
    result_sender: mpsc::Sender<TaskResult>,
    /// Channel receiver for streaming updates
    stream_receiver: Option<mpsc::Receiver<String>>,
    /// Channel sender for streaming updates (cloned for each task)
    stream_sender: mpsc::Sender<String>,
    /// Active task handles for cancellation
    active_tasks: Vec<(JoinHandle<TaskResult>, CancellationToken)>,
}

impl BackgroundTaskManager {
    /// Create a new background task manager
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(100);
        let (stream_sender, stream_receiver) = mpsc::channel(100);
        Self {
            result_receiver: Some(receiver),
            result_sender: sender,
            stream_receiver: Some(stream_receiver),
            stream_sender,
            active_tasks: Vec::new(),
        }
    }

    /// Spawn a background task that will send its result via channel
    ///
    /// The task will be cancelled if the cancellation token is triggered.
    /// Results are sent via the result channel and can be collected by calling `try_recv_result()`.
    pub fn spawn<F>(&mut self, task: F) -> CancellationToken
    where
        F: std::future::Future<Output = TaskResult> + Send + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();
        let sender = self.result_sender.clone();

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = token_clone.cancelled() => {
                    // Task was cancelled, return Ok(()) to indicate graceful cancellation
                    Ok(())
                }
                result = task => {
                    // Task completed, send result
                    let _ = sender.send(result).await;
                    Ok(())
                }
            }
        });

        self.active_tasks.push((handle, cancel_token.clone()));
        cancel_token
    }

    /// Spawn a streaming background task that can emit many updates
    ///
    /// The task receives a sender for streaming updates and a cancellation token.
    /// Updates can be collected via `try_recv_stream_line` or `drain_stream_lines`.
    pub fn spawn_streaming<F>(&mut self, task: F) -> CancellationToken
    where
        F: FnOnce(
                mpsc::Sender<String>,
                CancellationToken,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = TaskResult> + Send>>
            + Send
            + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();
        let result_sender = self.result_sender.clone();
        let stream_sender = self.stream_sender.clone();

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = token_clone.cancelled() => Ok(()),
                result = task(stream_sender, token_clone.clone()) => {
                    let _ = result_sender.send(result).await;
                    Ok(())
                }
            }
        });

        self.active_tasks.push((handle, cancel_token.clone()));
        cancel_token
    }

    /// Spawn a periodic background task that runs at a fixed interval until cancelled.
    ///
    /// Each tick can emit streaming updates and returns a `TaskResult`. Errors are
    /// forwarded through the result channel.
    pub fn spawn_periodic<F>(
        &mut self,
        interval: std::time::Duration,
        mut tick: F,
    ) -> CancellationToken
    where
        F: FnMut(
                mpsc::Sender<String>,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = TaskResult> + Send>>
            + Send
            + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();
        let result_sender = self.result_sender.clone();
        let stream_sender = self.stream_sender.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = token_clone.cancelled() => break,
                    _ = interval.tick() => {
                        let result = tick(stream_sender.clone()).await;
                        if let Err(err) = &result {
                            let _ = result_sender.send(Err(anyhow::anyhow!(err.to_string()))).await;
                        }
                    }
                }
            }
            Ok(())
        });

        self.active_tasks.push((handle, cancel_token.clone()));
        cancel_token
    }

    /// Spawn a background task with progress reporting capability
    ///
    /// The task receives a sender for progress updates and a cancellation token.
    /// Progress updates can be collected via the returned receiver using `try_recv()`.
    ///
    /// The progress sender can be cloned for concurrent operations within the same task.
    /// Each call to `spawn_with_progress()` creates a new, independent progress channel.
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - `CancellationToken` - Token for cancelling the task
    /// - `mpsc::Receiver<ProgressReporter>` - Receiver for non-blocking progress updates
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use tui_framework::app::background_tasks::{BackgroundTaskManager, ProgressReporter};
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let mut manager = BackgroundTaskManager::new();
    /// let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
    ///     Box::pin(async move {
    ///         for i in 1..=10 {
    ///             if cancel_token.is_cancelled() {
    ///                 break;
    ///             }
    ///             let progress = ProgressReporter::with_message(i, 10, format!("Item {}", i));
    ///             let _ = progress_tx.send(progress).await;  // Best-effort, ignore errors
    ///             tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    ///         }
    ///         Ok(())
    ///     })
    /// });
    ///
    /// // Poll progress updates (non-blocking)
    /// while let Ok(progress) = progress_rx.try_recv() {
    ///     println!("Progress: {}%", progress.percentage());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn spawn_with_progress<F>(
        &mut self,
        task: F,
    ) -> (CancellationToken, mpsc::Receiver<ProgressReporter>)
    where
        F: FnOnce(
                mpsc::Sender<ProgressReporter>,
                CancellationToken,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = TaskResult> + Send>>
            + Send
            + 'static,
    {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();
        let result_sender = self.result_sender.clone();

        // Create a new progress channel for this task
        let (progress_sender, progress_receiver) = mpsc::channel(100);
        let progress_sender_clone = progress_sender.clone();

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = token_clone.cancelled() => Ok(()),
                result = task(progress_sender_clone, token_clone.clone()) => {
                    let _ = result_sender.send(result).await;
                    Ok(())
                }
            }
        });

        self.active_tasks.push((handle, cancel_token.clone()));
        (cancel_token, progress_receiver)
    }

    /// Try to receive a task result (non-blocking)
    ///
    /// Returns `Some(result)` if a result is available, `None` if no results are ready.
    pub fn try_recv_result(&mut self) -> Option<TaskResult> {
        if let Some(ref mut receiver) = self.result_receiver {
            receiver.try_recv().ok()
        } else {
            None
        }
    }

    /// Try to receive a single streaming line (non-blocking)
    pub fn try_recv_stream_line(&mut self) -> Option<String> {
        if let Some(ref mut receiver) = self.stream_receiver {
            receiver.try_recv().ok()
        } else {
            None
        }
    }

    /// Drain all available streaming lines (non-blocking)
    pub fn drain_stream_lines(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(line) = self.try_recv_stream_line() {
            lines.push(line);
        }
        lines
    }

    /// Cancel all active tasks
    ///
    /// This is typically called when switching views or shutting down the application.
    pub fn cancel_all(&mut self) {
        for (_, token) in &self.active_tasks {
            token.cancel();
        }
        self.active_tasks.clear();
    }

    /// Cancel a specific task by its cancellation token
    pub fn cancel_task(&mut self, token: &CancellationToken) {
        token.cancel();
        // Remove tasks with matching token (compare by pointer address)
        self.active_tasks.retain(|(_, t)| {
            // Compare the inner Arc pointers
            std::ptr::eq(t as *const _, token as *const _)
        });
    }

    /// Clean up completed tasks
    ///
    /// Removes tasks that have completed from the active tasks list.
    pub fn cleanup_completed(&mut self) {
        self.active_tasks
            .retain(|(handle, _)| !handle.is_finished());
    }

    /// Get the number of active tasks
    pub fn active_task_count(&self) -> usize {
        self.active_tasks.len()
    }

    /// Detect available CPU parallelism
    ///
    /// Returns the number of available CPU cores, or 4 as a fallback if detection fails.
    fn detect_cpu_cores() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }

    /// Calculate default concurrency limit based on CPU cores
    ///
    /// Returns CPU cores * 2, with a minimum of 1.
    fn default_concurrency_limit() -> usize {
        Self::detect_cpu_cores() * 2
    }

    /// Calculate effective concurrency limit
    ///
    /// # Arguments
    /// * `requested` - User-requested limit (None means use default)
    ///
    /// # Returns
    /// Effective limit: min(requested.unwrap_or(default), max_limit)
    fn effective_concurrency_limit(requested: Option<usize>) -> usize {
        const MAX_LIMIT: usize = 100;
        let default = Self::default_concurrency_limit();
        let requested = requested.unwrap_or(default);
        requested.min(MAX_LIMIT).max(1) // Ensure at least 1
    }

    /// Spawn a batch of tasks with optional concurrency limit
    ///
    /// # Arguments
    /// * `tasks` - Collection of task definitions with optional identifiers
    /// * `concurrency_limit` - Optional maximum concurrent tasks (defaults to CPU-based limit, max 100)
    ///
    /// # Returns
    /// `BatchResult` containing aggregated statistics and individual task results
    ///
    /// # Behavior
    /// - Tasks are spawned concurrently (not sequentially)
    /// - Framework waits for all tasks to complete before returning
    /// - Empty task collections return empty `BatchResult` immediately
    /// - Results are returned in completion order
    pub async fn spawn_batch(
        &mut self,
        tasks: Vec<TaskDefinition>,
        concurrency_limit: Option<usize>,
    ) -> BatchResult {
        // Handle empty batch
        if tasks.is_empty() {
            return BatchResult {
                total: 0,
                successful: 0,
                failed: 0,
                cancelled: 0,
                success_rate: 0.0,
                errors: Vec::new(),
                results: Vec::new(),
            };
        }

        // Calculate effective concurrency limit
        let effective_limit = Self::effective_concurrency_limit(concurrency_limit);
        let semaphore = Arc::new(Semaphore::new(effective_limit));
        let mut join_set = JoinSet::new();
        let task_count = tasks.len();

        // Create cancellation token for this batch
        let batch_token = CancellationToken::new();
        let batch_token_clone = batch_token.clone();

        // Spawn all tasks with concurrency control and cancellation support
        for (index, task_def) in tasks.into_iter().enumerate() {
            let identifier = task_def
                .identifier()
                .map(|s| TaskIdentifier::Provided(s.clone()))
                .unwrap_or_else(|| TaskIdentifier::Index(index));
            let task = task_def.into_task();
            let semaphore_clone = semaphore.clone();
            let cancel_token = batch_token_clone.clone();

            join_set.spawn(async move {
                // Acquire permit before executing task
                let _permit = semaphore_clone.acquire().await.expect("Semaphore closed");
                // Permit is held during task execution and released when dropped

                // Check for cancellation and timeout during task execution
                // Note: Timeout handling would be added here if timeout parameter is provided
                // For now, we only handle cancellation
                let result = tokio::select! {
                    _ = cancel_token.cancelled() => {
                        // Task was cancelled
                        Err(anyhow::anyhow!("Task cancelled"))
                    }
                    result = task() => {
                        result
                    }
                };

                (identifier, result)
            });
        }

        // Collect results in completion order
        let mut results = Vec::new();
        let mut successful = 0;
        let mut failed = 0;
        let mut cancelled = 0;
        let mut errors = Vec::new();

        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((identifier, task_result)) => {
                    match task_result {
                        Ok(value) => {
                            // Task succeeded
                            successful += 1;
                            results.push(BatchTaskResult {
                                identifier: identifier.clone(),
                                status: TaskStatus::Success,
                                value: Some(value),
                                error: None,
                            });
                        }
                        Err(err) => {
                            // Check if error is cancellation
                            if err.to_string().contains("cancelled")
                                || batch_token_clone.is_cancelled()
                            {
                                cancelled += 1;
                                results.push(BatchTaskResult {
                                    identifier,
                                    status: TaskStatus::Cancelled,
                                    value: None,
                                    error: None,
                                });
                            } else {
                                // Task failed
                                failed += 1;
                                let error_msg =
                                    format!("Task {} failed: {}", identifier.display(), err);
                                let formatted_error = anyhow::anyhow!(error_msg.clone());
                                errors.push((identifier.clone(), anyhow::anyhow!(error_msg)));
                                results.push(BatchTaskResult {
                                    identifier,
                                    status: TaskStatus::Failure,
                                    value: None,
                                    error: Some(formatted_error),
                                });
                            }
                        }
                    }
                }
                Err(join_err) => {
                    // Join error (task panicked)
                    failed += 1;
                    let identifier = TaskIdentifier::Index(results.len());
                    let error_msg = format!("Task {} panicked: {}", identifier.display(), join_err);
                    let formatted_error = anyhow::anyhow!(error_msg.clone());
                    errors.push((identifier.clone(), anyhow::anyhow!(error_msg)));
                    results.push(BatchTaskResult {
                        identifier,
                        status: TaskStatus::Failure,
                        value: None,
                        error: Some(formatted_error),
                    });
                }
            }
        }

        // Calculate success rate (excluding cancelled tasks)
        let success_rate = if successful + failed > 0 {
            (successful as f64 / (successful + failed) as f64) * 100.0
        } else {
            0.0
        };

        BatchResult {
            total: task_count,
            successful,
            failed,
            cancelled,
            success_rate,
            errors,
            results,
        }
    }

    /// Wait for all currently active tasks to complete
    ///
    /// This includes tasks spawned individually (via `spawn()`, `spawn_streaming()`, `spawn_periodic()`)
    /// and waits for them to finish. Results are returned in completion order.
    ///
    /// # Returns
    /// `BatchResult` containing results from all active tasks
    ///
    /// # Behavior
    /// - Blocks until all tasks finish
    /// - Returns immediately with empty results if no tasks are active
    /// - Results are in completion order (not spawn order)
    pub async fn wait_for_all(&mut self) -> BatchResult {
        if self.active_tasks.is_empty() {
            return BatchResult {
                total: 0,
                successful: 0,
                failed: 0,
                cancelled: 0,
                success_rate: 0.0,
                errors: Vec::new(),
                results: Vec::new(),
            };
        }

        let mut results = Vec::new();
        let mut successful = 0;
        let mut failed = 0;
        let mut cancelled = 0;
        let mut errors = Vec::new();

        // Collect results from all active tasks
        let mut join_set = JoinSet::new();
        let mut task_tokens = Vec::new();

        // Move all active tasks to JoinSet for collection
        for (handle, token) in self.active_tasks.drain(..) {
            task_tokens.push(token.clone());
            join_set.spawn(async move {
                let result = handle.await;
                (token, result)
            });
        }

        // Collect results in completion order
        while let Some(join_result) = join_set.join_next().await {
            match join_result {
                Ok((token, task_result)) => {
                    let identifier = TaskIdentifier::Index(results.len());

                    match task_result {
                        Ok(Ok(())) => {
                            // Task succeeded
                            successful += 1;
                            results.push(BatchTaskResult {
                                identifier: identifier.clone(),
                                status: TaskStatus::Success,
                                value: None, // Individual tasks return ()
                                error: None,
                            });
                        }
                        Ok(Err(err)) => {
                            // Task failed
                            failed += 1;
                            let error_msg =
                                format!("Task {} failed: {}", identifier.display(), err);
                            let formatted_error = anyhow::anyhow!(error_msg.clone());
                            errors.push((identifier.clone(), anyhow::anyhow!(error_msg)));
                            results.push(BatchTaskResult {
                                identifier,
                                status: TaskStatus::Failure,
                                value: None,
                                error: Some(formatted_error),
                            });
                        }
                        Err(join_err) => {
                            // Check if task was cancelled
                            if token.is_cancelled() {
                                cancelled += 1;
                                results.push(BatchTaskResult {
                                    identifier,
                                    status: TaskStatus::Cancelled,
                                    value: None,
                                    error: None,
                                });
                            } else {
                                // Task panicked
                                failed += 1;
                                let error_msg =
                                    format!("Task {} panicked: {}", identifier.display(), join_err);
                                let formatted_error = anyhow::anyhow!(error_msg.clone());
                                errors.push((identifier.clone(), anyhow::anyhow!(error_msg)));
                                results.push(BatchTaskResult {
                                    identifier,
                                    status: TaskStatus::Failure,
                                    value: None,
                                    error: Some(formatted_error),
                                });
                            }
                        }
                    }
                }
                Err(_) => {
                    // JoinSet error (shouldn't happen)
                    failed += 1;
                    let identifier = TaskIdentifier::Index(results.len());
                    let error_msg = format!("Task {} join error", identifier.display());
                    let formatted_error = anyhow::anyhow!(error_msg.clone());
                    errors.push((identifier.clone(), anyhow::anyhow!(error_msg)));
                    results.push(BatchTaskResult {
                        identifier,
                        status: TaskStatus::Failure,
                        value: None,
                        error: Some(formatted_error),
                    });
                }
            }
        }

        // Calculate success rate (excluding cancelled tasks)
        let success_rate = if successful + failed > 0 {
            (successful as f64 / (successful + failed) as f64) * 100.0
        } else {
            0.0
        };

        BatchResult {
            total: results.len(),
            successful,
            failed,
            cancelled,
            success_rate,
            errors,
            results,
        }
    }

    /// Cancel all tasks in a batch
    ///
    /// # Arguments
    /// * `batch_token` - Cancellation token for the batch (returned from spawn_batch)
    ///
    /// # Behavior
    /// - Cancels all tasks associated with the batch token
    /// - Other tasks (not in this batch) continue executing
    /// - Cancelled tasks are tracked separately in results
    ///
    /// # Note
    /// Currently, batch cancellation tokens are not returned from spawn_batch.
    /// This method is provided for future API compatibility. To cancel a batch,
    /// you would need to store the cancellation token returned from a modified
    /// spawn_batch API.
    pub fn cancel_batch(&mut self, batch_token: &CancellationToken) {
        batch_token.cancel();
        // Note: Batch tasks handle cancellation internally via the token
        // Individual tasks spawned via spawn() are tracked in active_tasks
        // and can be cancelled via cancel_task()
    }
}

impl Default for BackgroundTaskManager {
    fn default() -> Self {
        Self::new()
    }
}
