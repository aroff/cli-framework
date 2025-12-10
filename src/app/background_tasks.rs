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
