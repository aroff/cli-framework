//! Background task management for async operations
//!
//! Provides a system for spawning and managing long-running async operations
//! that update the UI when complete, without blocking the event loop.

use anyhow::Result;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Result type for background tasks
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
}

impl Default for BackgroundTaskManager {
    fn default() -> Self {
        Self::new()
    }
}
