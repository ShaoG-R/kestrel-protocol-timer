use crate::task::{TaskId, TimerWheelId};
use crate::timer::{BatchHandle, TimerHandle, WheelOperation};
use crate::error::TimerError;
use crossbeam::channel::Sender;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// TimerService 命令类型
enum ServiceCommand {
    /// 添加批量定时器句柄
    AddBatchHandle(BatchHandle),
    /// 添加单个定时器句柄
    AddTimerHandle(TimerHandle),
    /// 取消单个任务
    CancelTask {
        task_id: TaskId,
        result_tx: oneshot::Sender<bool>,
    },
    /// 关闭 Service
    Shutdown,
}

/// TimerService - 基于 Actor 模式的定时器服务
///
/// 管理多个定时器句柄，监听所有超时事件，并将 TaskId 聚合转发给用户。
///
/// # 特性
/// - 自动监听所有添加的定时器句柄的超时事件
/// - 超时后自动从内部管理中移除该任务
/// - 将超时的 TaskId 转发到统一的通道供用户接收
/// - 支持动态添加 BatchHandle 和 TimerHandle
///
/// # 示例
/// ```no_run
/// use timer::{TimerWheel, TimerService};
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() {
///     let timer = TimerWheel::with_defaults().unwrap();
///     let mut service = TimerService::new();
///     
///     // 创建批量定时器并添加到 service
///     let callbacks: Vec<_> = (0..5)
///         .map(|_| (Duration::from_millis(100), || async {}))
///         .collect();
///     let batch = timer.schedule_once_batch(callbacks).await.unwrap();
///     service.add_batch_handle(batch).await.unwrap();
///     
///     // 接收超时通知
///     let mut rx = service.timeout_receiver();
///     while let Some(task_id) = rx.recv().await {
///         println!("Task {:?} completed", task_id);
///     }
/// }
/// ```
pub struct TimerService {
    /// 命令发送端
    command_tx: mpsc::Sender<ServiceCommand>,
    /// 超时接收端
    timeout_rx: mpsc::Receiver<TaskId>,
    /// Actor 任务句柄
    actor_handle: Option<JoinHandle<()>>,
    /// 时间轮 ID（用于验证所有句柄来自同一个时间轮）
    timer_wheel_id: Option<TimerWheelId>,
}

impl TimerService {
    /// 创建新的 TimerService
    ///
    /// # 示例
    /// ```
    /// use timer::TimerService;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut service = TimerService::new();
    /// }
    /// ```
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::channel(100);
        let (timeout_tx, timeout_rx) = mpsc::channel(1000);

        let actor = ServiceActor::new(command_rx, timeout_tx);
        let actor_handle = tokio::spawn(async move {
            actor.run().await;
        });

        Self {
            command_tx,
            timeout_rx,
            actor_handle: Some(actor_handle),
            timer_wheel_id: None,
        }
    }

    /// 添加批量定时器句柄
    ///
    /// # 参数
    /// - `batch`: 批量定时器句柄
    ///
    /// # 返回
    /// - `Ok(())`: 成功添加
    /// - `Err(TimerError)`: 添加失败（例如：TimerWheel ID 不匹配）
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = TimerService::new();
    /// 
    /// let callbacks: Vec<_> = (0..5)
    ///     .map(|_| (Duration::from_millis(100), || async {}))
    ///     .collect();
    /// let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    /// 
    /// service.add_batch_handle(batch).await.unwrap();
    /// # }
    /// ```
    pub async fn add_batch_handle(&mut self, batch: BatchHandle) -> Result<(), TimerError> {
        // 验证 timer_wheel_id
        let batch_wheel_id = batch.timer_wheel_id();
        if let Some(expected_id) = self.timer_wheel_id {
            if expected_id != batch_wheel_id {
                return Err(TimerError::TimerWheelIdMismatch {
                    expected: expected_id,
                    actual: batch_wheel_id,
                });
            }
        } else {
            // 第一次添加，记录 timer_wheel_id
            self.timer_wheel_id = Some(batch_wheel_id);
        }
        
        self.command_tx
            .send(ServiceCommand::AddBatchHandle(batch))
            .await
            .map_err(|_| TimerError::ChannelClosed)
    }

    /// 添加单个定时器句柄
    ///
    /// # 参数
    /// - `handle`: 定时器句柄
    ///
    /// # 返回
    /// - `Ok(())`: 成功添加
    /// - `Err(TimerError)`: 添加失败（例如：TimerWheel ID 不匹配）
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = TimerService::new();
    /// 
    /// let handle = timer.schedule_once(Duration::from_millis(100), || async {}).await.unwrap();
    /// 
    /// service.add_timer_handle(handle).await.unwrap();
    /// # }
    /// ```
    pub async fn add_timer_handle(&mut self, handle: TimerHandle) -> Result<(), TimerError> {
        // 验证 timer_wheel_id
        let handle_wheel_id = handle.timer_wheel_id();
        if let Some(expected_id) = self.timer_wheel_id {
            if expected_id != handle_wheel_id {
                return Err(TimerError::TimerWheelIdMismatch {
                    expected: expected_id,
                    actual: handle_wheel_id,
                });
            }
        } else {
            // 第一次添加，记录 timer_wheel_id
            self.timer_wheel_id = Some(handle_wheel_id);
        }
        
        self.command_tx
            .send(ServiceCommand::AddTimerHandle(handle))
            .await
            .map_err(|_| TimerError::ChannelClosed)
    }

    /// 获取超时接收器的可变引用
    ///
    /// # 返回
    /// 超时通知接收器的可变引用
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut service = TimerService::new();
    /// 
    /// let mut rx = service.timeout_receiver();
    /// while let Some(task_id) = rx.recv().await {
    ///     println!("Task {:?} timed out", task_id);
    /// }
    /// # }
    /// ```
    pub fn timeout_receiver(&mut self) -> &mut mpsc::Receiver<TaskId> {
        &mut self.timeout_rx
    }

    /// 取消指定的任务
    ///
    /// # 参数
    /// - `task_id`: 要取消的任务 ID
    ///
    /// # 返回
    /// - `Ok(true)`: 任务存在且成功取消
    /// - `Ok(false)`: 任务不存在或取消失败
    /// - `Err(String)`: 发送命令失败
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = TimerService::new();
    /// 
    /// let handle = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    /// let task_id = handle.task_id();
    /// 
    /// service.add_timer_handle(handle).await.unwrap();
    /// 
    /// // 取消任务
    /// let cancelled = service.cancel_task(task_id).await.unwrap();
    /// println!("Task cancelled: {}", cancelled);
    /// # }
    /// ```
    pub async fn cancel_task(&self, task_id: TaskId) -> Result<bool, String> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(ServiceCommand::CancelTask { task_id, result_tx: tx })
            .await
            .map_err(|e| format!("Failed to send CancelTask command: {}", e))?;
        
        rx.await.map_err(|e| format!("Failed to receive cancel result: {}", e))
    }

    /// 优雅关闭 TimerService
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerService;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let mut service = TimerService::new();
    /// 
    /// // 使用 service...
    /// 
    /// service.shutdown().await;
    /// # }
    /// ```
    pub async fn shutdown(mut self) {
        let _ = self.command_tx.send(ServiceCommand::Shutdown).await;
        if let Some(handle) = self.actor_handle.take() {
            let _ = handle.await;
        }
    }
}

impl Default for TimerService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TimerService {
    fn drop(&mut self) {
        if let Some(handle) = self.actor_handle.take() {
            handle.abort();
        }
    }
}

/// ServiceActor - 内部 Actor 实现
struct ServiceActor {
    /// 命令接收端
    command_rx: mpsc::Receiver<ServiceCommand>,
    /// 超时发送端
    timeout_tx: mpsc::Sender<TaskId>,
    /// 操作发送端（用于取消操作等）
    #[allow(dead_code)]
    op_sender: Option<Sender<WheelOperation>>,
    /// 任务ID到操作发送端的映射（用于取消操作）
    task_senders: std::collections::HashMap<TaskId, Sender<WheelOperation>>,
}

impl ServiceActor {
    fn new(command_rx: mpsc::Receiver<ServiceCommand>, timeout_tx: mpsc::Sender<TaskId>) -> Self {
        Self {
            command_rx,
            timeout_tx,
            op_sender: None,
            task_senders: std::collections::HashMap::new(),
        }
    }

    async fn run(mut self) {
        // 使用 FuturesUnordered 来监听所有的 completion_rxs
        // 每个 future 返回 (TaskId, Result)
        let mut futures: FuturesUnordered<BoxFuture<'static, (TaskId, Result<(), tokio::sync::oneshot::error::RecvError>)>> = FuturesUnordered::new();

        loop {
            tokio::select! {
                // 监听超时事件
                Some((task_id, _result)) = futures.next() => {
                    // 任务超时，转发 TaskId
                    let _ = self.timeout_tx.send(task_id).await;
                    // 从 task_senders 中移除该任务
                    self.task_senders.remove(&task_id);
                    // 任务会自动从 FuturesUnordered 中移除
                }
                
                // 监听命令
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        ServiceCommand::AddBatchHandle(batch) => {
                            let BatchHandle {
                                task_ids,
                                op_sender,
                                completion_rxs,
                                ..
                            } = batch;
                            
                            // 将所有任务添加到 futures 和 task_senders 中
                            for (task_id, rx) in task_ids.into_iter().zip(completion_rxs.into_iter()) {
                                // 记录到 task_senders
                                self.task_senders.insert(task_id, op_sender.clone());
                                
                                let future: BoxFuture<'static, (TaskId, Result<(), tokio::sync::oneshot::error::RecvError>)> = Box::pin(async move {
                                    (task_id, rx.await)
                                });
                                futures.push(future);
                            }
                        }
                        ServiceCommand::AddTimerHandle(handle) => {
                            let TimerHandle{
                                task_id,
                                op_sender,
                                completion_rx,
                                ..
                            } = handle;
                            
                            // 记录到 task_senders
                            self.task_senders.insert(task_id, op_sender);
                            
                            // 添加到 futures 中
                            let future: BoxFuture<'static, (TaskId, Result<(), tokio::sync::oneshot::error::RecvError>)> = Box::pin(async move {
                                (task_id, completion_rx.0.await)
                            });
                            futures.push(future);
                        }
                        ServiceCommand::CancelTask { task_id, result_tx } => {
                            // 从 task_senders 中查找对应的 op_sender
                            let cancel_result = if let Some(op_sender) = self.task_senders.get(&task_id) {
                                // 发送取消操作到时间轮
                                let (tx, rx) = oneshot::channel();
                                let send_result = op_sender.send(WheelOperation::Cancel {
                                    task_id,
                                    result_tx: Some(tx),
                                });
                                
                                if send_result.is_ok() {
                                    // 等待取消结果
                                    match rx.await {
                                        Ok(success) => {
                                            if success {
                                                // 取消成功，从 task_senders 中移除
                                                self.task_senders.remove(&task_id);
                                            }
                                            success
                                        }
                                        Err(_) => false,
                                    }
                                } else {
                                    false
                                }
                            } else {
                                // 任务不存在
                                false
                            };
                            
                            // 发送取消结果
                            let _ = result_tx.send(cancel_result);
                        }
                        ServiceCommand::Shutdown => {
                            break;
                        }
                    }
                }
                
                // 如果没有任何 future 且命令通道已关闭，退出循环
                else => {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimerWheel;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn test_service_creation() {
        let _service = TimerService::new();
    }

    #[tokio::test]
    async fn test_add_batch_handle_and_receive_timeouts() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 创建批量定时器
        let callbacks: Vec<_> = (0..3)
            .map(|_| (Duration::from_millis(50), || async {}))
            .collect();
        let batch = timer.schedule_once_batch(callbacks).await.unwrap();
        let expected_count = batch.len();

        // 添加到 service
        service.add_batch_handle(batch).await.unwrap();

        // 接收超时通知
        let mut received_count = 0;
        let rx = service.timeout_receiver();
        
        // 使用 timeout 避免测试挂起
        while received_count < expected_count {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(_task_id)) => {
                    received_count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert_eq!(received_count, expected_count);
    }

    #[tokio::test]
    async fn test_add_timer_handle_and_receive_timeout() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 创建单个定时器
        let handle = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        let task_id = handle.task_id();

        // 添加到 service
        service.add_timer_handle(handle).await.unwrap();

        // 接收超时通知
        let rx = service.timeout_receiver();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id);
    }

    #[tokio::test]
    async fn test_mixed_handles() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加单个定时器
        let handle1 = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        service.add_timer_handle(handle1).await.unwrap();

        // 添加批量定时器
        let callbacks: Vec<_> = (0..2)
            .map(|_| (Duration::from_millis(50), || async {}))
            .collect();
        let batch = timer.schedule_once_batch(callbacks).await.unwrap();
        service.add_batch_handle(batch).await.unwrap();

        // 应该接收到 3 个超时通知
        let mut received_count = 0;
        let rx = service.timeout_receiver();
        
        while received_count < 3 {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(_task_id)) => {
                    received_count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert_eq!(received_count, 3);
    }

    #[tokio::test]
    async fn test_shutdown() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加一些定时器
        let callbacks: Vec<_> = (0..5)
            .map(|_| (Duration::from_secs(10), || async {}))
            .collect();
        let batch = timer.schedule_once_batch(callbacks).await.unwrap();
        service.add_batch_handle(batch).await.unwrap();

        // 立即关闭（不等待定时器触发）
        service.shutdown().await;
    }

    #[tokio::test]
    async fn test_callback_execution_with_service() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建带回调的定时器
        let callbacks: Vec<_> = (0..3)
            .map(|_| {
                let counter = Arc::clone(&counter);
                (Duration::from_millis(50), move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();

        let batch = timer.schedule_once_batch(callbacks).await.unwrap();
        service.add_batch_handle(batch).await.unwrap();

        // 接收所有超时通知
        let mut received_count = 0;
        let rx = service.timeout_receiver();
        
        while received_count < 3 {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(_task_id)) => {
                    received_count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // 等待一下确保回调执行完成
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 验证所有回调都已执行
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_timer_wheel_id_mismatch() {
        let timer1 = TimerWheel::with_defaults().unwrap();
        let timer2 = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加来自 timer1 的句柄
        let handle1 = timer1.schedule_once(Duration::from_millis(100), || async {}).await.unwrap();
        service.add_timer_handle(handle1).await.unwrap();

        // 尝试添加来自 timer2 的句柄，应该失败
        let handle2 = timer2.schedule_once(Duration::from_millis(100), || async {}).await.unwrap();
        let result = service.add_timer_handle(handle2).await;
        
        assert!(result.is_err());
        match result {
            Err(TimerError::TimerWheelIdMismatch { .. }) => {
                // 预期的错误类型
            }
            _ => panic!("Expected TimerWheelIdMismatch error"),
        }
    }

    #[tokio::test]
    async fn test_timer_wheel_id_mismatch_batch() {
        let timer1 = TimerWheel::with_defaults().unwrap();
        let timer2 = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加来自 timer1 的批量句柄
        let callbacks1: Vec<_> = (0..2)
            .map(|_| (Duration::from_millis(100), || async {}))
            .collect();
        let batch1 = timer1.schedule_once_batch(callbacks1).await.unwrap();
        service.add_batch_handle(batch1).await.unwrap();

        // 尝试添加来自 timer2 的批量句柄，应该失败
        let callbacks2: Vec<_> = (0..2)
            .map(|_| (Duration::from_millis(100), || async {}))
            .collect();
        let batch2 = timer2.schedule_once_batch(callbacks2).await.unwrap();
        let result = service.add_batch_handle(batch2).await;
        
        assert!(result.is_err());
        match result {
            Err(TimerError::TimerWheelIdMismatch { .. }) => {
                // 预期的错误类型
            }
            _ => panic!("Expected TimerWheelIdMismatch error"),
        }
    }

    #[tokio::test]
    async fn test_cancel_task() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加一个长时间的定时器
        let handle = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
        let task_id = handle.task_id();
        
        service.add_timer_handle(handle).await.unwrap();

        // 取消任务
        let cancelled = service.cancel_task(task_id).await.unwrap();
        assert!(cancelled, "Task should be cancelled successfully");

        // 尝试再次取消同一个任务，应该返回 false
        let cancelled_again = service.cancel_task(task_id).await.unwrap();
        assert!(!cancelled_again, "Task should not exist anymore");
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_task() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加一个定时器以初始化 service
        let handle = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        service.add_timer_handle(handle).await.unwrap();

        // 尝试取消一个不存在的任务
        let fake_task_id = TaskId::new();
        let cancelled = service.cancel_task(fake_task_id).await.unwrap();
        assert!(!cancelled, "Nonexistent task should not be cancelled");
    }

    #[tokio::test]
    async fn test_cancel_task_from_batch() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 创建批量定时器
        let callbacks: Vec<_> = (0..3)
            .map(|_| (Duration::from_secs(10), || async {}))
            .collect();
        let batch = timer.schedule_once_batch(callbacks).await.unwrap();
        let task_ids = batch.task_ids().to_vec();
        
        service.add_batch_handle(batch).await.unwrap();

        // 取消第一个任务
        let cancelled = service.cancel_task(task_ids[0]).await.unwrap();
        assert!(cancelled, "First task should be cancelled successfully");

        // 取消第二个任务
        let cancelled = service.cancel_task(task_ids[1]).await.unwrap();
        assert!(cancelled, "Second task should be cancelled successfully");

        // 再次取消第一个任务，应该返回 false
        let cancelled_again = service.cancel_task(task_ids[0]).await.unwrap();
        assert!(!cancelled_again, "First task should not exist anymore");
    }

    #[tokio::test]
    async fn test_task_timeout_cleans_up_task_sender() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = TimerService::new();

        // 添加一个短时间的定时器
        let handle = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        let task_id = handle.task_id();
        
        service.add_timer_handle(handle).await.unwrap();

        // 等待任务超时
        let rx = service.timeout_receiver();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");
        
        assert_eq!(received_task_id, task_id);

        // 等待一下确保内部清理完成
        tokio::time::sleep(Duration::from_millis(10)).await;

        // 尝试取消已经超时的任务，应该返回 false
        let cancelled = service.cancel_task(task_id).await.unwrap();
        assert!(!cancelled, "Timed out task should not exist anymore");
    }
}

