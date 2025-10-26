use crate::task::{CallbackWrapper, TaskId, TimerCallback};
use crate::timer::{BatchHandle, TimerHandle};
use crate::error::TimerError;
use crate::wheel::Wheel;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::future::BoxFuture;
use parking_lot::Mutex;
use rustc_hash::FxHashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// TimerService 命令类型
enum ServiceCommand {
    /// 添加批量定时器句柄
    AddBatchHandle(BatchHandle),
    /// 添加单个定时器句柄
    AddTimerHandle(TimerHandle),
    /// 批量从活跃任务集合中移除任务（用于直接取消后的清理）
    RemoveTasks {
        task_ids: Vec<TaskId>,
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
/// use kestrel_protocol_timer::{TimerWheel, TimerService};
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() {
///     let timer = TimerWheel::with_defaults().unwrap();
///     let mut service = timer.create_service();
///     
///     // 直接通过 service 批量调度定时器
///     let callbacks: Vec<_> = (0..5)
///         .map(|_| (Duration::from_millis(100), || async {}))
///         .collect();
///     service.schedule_once_batch(callbacks).await.unwrap();
///     
///     // 接收超时通知
///     let mut rx = service.take_receiver().unwrap();
///     while let Some(task_id) = rx.recv().await {
///         println!("Task {:?} completed", task_id);
///     }
/// }
/// ```
pub struct TimerService {
    /// 命令发送端
    command_tx: mpsc::Sender<ServiceCommand>,
    /// 超时接收端
    timeout_rx: Option<mpsc::Receiver<TaskId>>,
    /// Actor 任务句柄
    actor_handle: Option<JoinHandle<()>>,
    /// 时间轮引用（用于直接调度定时器）
    wheel: Arc<Mutex<Wheel>>,
}

impl TimerService {
    /// 创建新的 TimerService
    ///
    /// # 参数
    /// - `wheel`: 时间轮引用
    ///
    /// # 注意
    /// 通常不直接调用此方法，而是使用 `TimerWheel::create_service()` 来创建。
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::TimerWheel;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     let mut service = timer.create_service();
    /// }
    /// ```
    pub(crate) fn new(wheel: Arc<Mutex<Wheel>>) -> Self {
        // 优化：增加命令通道容量以减少背压，提升添加操作的吞吐量
        let (command_tx, command_rx) = mpsc::channel(512);
        let (timeout_tx, timeout_rx) = mpsc::channel(1000);

        let actor = ServiceActor::new(command_rx, timeout_tx);
        let actor_handle = tokio::spawn(async move {
            actor.run().await;
        });

        Self {
            command_tx,
            timeout_rx: Some(timeout_rx),
            actor_handle: Some(actor_handle),
            wheel,
        }
    }

    /// 添加批量定时器句柄（内部方法）
    async fn add_batch_handle(&self, batch: BatchHandle) -> Result<(), TimerError> {
        self.command_tx
            .send(ServiceCommand::AddBatchHandle(batch))
            .await
            .map_err(|_| TimerError::ChannelClosed)
    }

    /// 添加单个定时器句柄（内部方法）
    async fn add_timer_handle(&self, handle: TimerHandle) -> Result<(), TimerError> {
        self.command_tx
            .send(ServiceCommand::AddTimerHandle(handle))
            .await
            .map_err(|_| TimerError::ChannelClosed)
    }

    /// 获取超时接收器（转移所有权）
    ///
    /// # 返回
    /// 超时通知接收器，如果已经被取走则返回 None
    ///
    /// # 注意
    /// 此方法只能调用一次，因为它会转移接收器的所有权
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = timer.create_service();
    /// 
    /// let mut rx = service.take_receiver().unwrap();
    /// while let Some(task_id) = rx.recv().await {
    ///     println!("Task {:?} timed out", task_id);
    /// }
    /// # }
    /// ```
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<TaskId>> {
        self.timeout_rx.take()
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
    /// # 性能说明
    /// 此方法使用直接取消优化，不需要等待 Actor 处理，大幅降低延迟
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let service = timer.create_service();
    /// 
    /// // 直接通过 service 调度定时器
    /// let task_id = service.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    /// 
    /// // 取消任务
    /// let cancelled = service.cancel_task(task_id).await.unwrap();
    /// println!("Task cancelled: {}", cancelled);
    /// # }
    /// ```
    pub async fn cancel_task(&self, task_id: TaskId) -> Result<bool, String> {
        // 优化：直接取消任务，避免通过 Actor 的异步往返
        // 这将延迟从 "2次异步通信" 减少到 "0次等待"
        let success = {
            let mut wheel = self.wheel.lock();
            wheel.cancel(task_id)
        };
        
        // 异步通知 Actor 清理 active_tasks（无需等待结果）
        if success {
            let _ = self.command_tx
                .send(ServiceCommand::RemoveTasks { 
                    task_ids: vec![task_id] 
                })
                .await;
        }
        
        Ok(success)
    }

    /// 批量取消任务
    ///
    /// 使用底层的批量取消操作一次性取消多个任务，性能优于循环调用 cancel_task。
    ///
    /// # 参数
    /// - `task_ids`: 要取消的任务 ID 列表
    ///
    /// # 返回
    /// 成功取消的任务数量
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let service = timer.create_service();
    /// 
    /// let callbacks: Vec<_> = (0..10)
    ///     .map(|_| (Duration::from_secs(10), || async {}))
    ///     .collect();
    /// let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
    /// 
    /// // 批量取消
    /// let cancelled = service.cancel_batch(&task_ids).await;
    /// println!("成功取消 {} 个任务", cancelled);
    /// # }
    /// ```
    pub async fn cancel_batch(&self, task_ids: &[TaskId]) -> usize {
        if task_ids.is_empty() {
            return 0;
        }

        // 直接使用底层的批量取消
        let cancelled_count = {
            let mut wheel = self.wheel.lock();
            wheel.cancel_batch(task_ids)
        };

        // 使用批量移除命令，一次性发送所有需要移除的任务ID
        let _ = self.command_tx
            .send(ServiceCommand::RemoveTasks { 
                task_ids: task_ids.to_vec() 
            })
            .await;

        cancelled_count
    }

    /// 调度一次性定时器
    ///
    /// 创建定时器并自动添加到服务管理中，无需手动调用 add_timer_handle
    ///
    /// # 参数
    /// - `delay`: 延迟时间
    /// - `callback`: 实现了 TimerCallback trait 的回调对象
    ///
    /// # 返回
    /// - `Ok(TaskId)`: 成功调度，返回任务ID
    /// - `Err(TimerError)`: 调度失败
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = timer.create_service();
    /// 
    /// let task_id = service.schedule_once(Duration::from_millis(100), || async {
    ///     println!("Timer fired!");
    /// }).await.unwrap();
    /// 
    /// println!("Scheduled task: {:?}", task_id);
    /// # }
    /// ```
    pub async fn schedule_once<C>(&self, delay: Duration, callback: C) -> Result<TaskId, TimerError>
    where
        C: TimerCallback,
    {
        // 创建任务并获取句柄
        let handle = self.create_timer_handle(delay, Some(Arc::new(callback)))?;
        let task_id = handle.task_id();
        
        // 自动添加到服务管理
        self.add_timer_handle(handle).await?;
        
        Ok(task_id)
    }

    /// 批量调度一次性定时器
    ///
    /// 批量创建定时器并自动添加到服务管理中
    ///
    /// # 参数
    /// - `callbacks`: (延迟时间, 回调) 的元组列表
    ///
    /// # 返回
    /// - `Ok(Vec<TaskId>)`: 成功调度，返回所有任务ID
    /// - `Err(TimerError)`: 调度失败
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = timer.create_service();
    /// 
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|i| (Duration::from_millis(100 * (i + 1)), move || async move {
    ///         println!("Timer {} fired!", i);
    ///     }))
    ///     .collect();
    /// 
    /// let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
    /// println!("Scheduled {} tasks", task_ids.len());
    /// # }
    /// ```
    pub async fn schedule_once_batch<C>(&self, callbacks: Vec<(Duration, C)>) -> Result<Vec<TaskId>, TimerError>
    where
        C: TimerCallback,
    {
        // 创建批量任务并获取句柄
        let batch_handle = self.create_batch_handle(callbacks)?;
        let task_ids = batch_handle.task_ids().to_vec();
        
        // 自动添加到服务管理
        self.add_batch_handle(batch_handle).await?;
        
        Ok(task_ids)
    }

    /// 调度一次性通知定时器（无回调，仅通知）
    ///
    /// 创建仅通知的定时器并自动添加到服务管理中
    ///
    /// # 参数
    /// - `delay`: 延迟时间
    ///
    /// # 返回
    /// - `Ok(TaskId)`: 成功调度，返回任务ID
    /// - `Err(TimerError)`: 调度失败
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = timer.create_service();
    /// 
    /// let task_id = service.schedule_once_notify(Duration::from_millis(100)).await.unwrap();
    /// println!("Scheduled notify task: {:?}", task_id);
    /// 
    /// // 可以通过 timeout_receiver 接收超时通知
    /// # }
    /// ```
    pub async fn schedule_once_notify(&self, delay: Duration) -> Result<TaskId, TimerError> {
        // 创建无回调任务并获取句柄
        let handle = self.create_timer_handle(delay, None)?;
        let task_id = handle.task_id();
        
        // 自动添加到服务管理
        self.add_timer_handle(handle).await?;
        
        Ok(task_id)
    }

    /// 内部方法：创建定时器句柄
    fn create_timer_handle(
        &self,
        delay: Duration,
        callback: Option<CallbackWrapper>,
    ) -> Result<TimerHandle, TimerError> {
        crate::timer::TimerWheel::create_timer_handle_internal(
            &self.wheel,
            delay,
            callback
        )
    }

    /// 内部方法：创建批量定时器句柄
    fn create_batch_handle<C>(
        &self,
        callbacks: Vec<(Duration, C)>,
    ) -> Result<BatchHandle, TimerError>
    where
        C: TimerCallback,
    {
        crate::timer::TimerWheel::create_batch_handle_internal(
            &self.wheel,
            callbacks
        )
    }

    /// 优雅关闭 TimerService
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::TimerWheel;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let mut service = timer.create_service();
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
    /// 活跃任务ID集合（使用 FxHashSet 提升性能）
    active_tasks: FxHashSet<TaskId>,
}

impl ServiceActor {
    fn new(command_rx: mpsc::Receiver<ServiceCommand>, timeout_tx: mpsc::Sender<TaskId>) -> Self {
        Self {
            command_rx,
            timeout_tx,
            active_tasks: FxHashSet::default(),
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
                    // 从活跃任务集合中移除该任务
                    self.active_tasks.remove(&task_id);
                    // 任务会自动从 FuturesUnordered 中移除
                }
                
                // 监听命令
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        ServiceCommand::AddBatchHandle(batch) => {
                            let BatchHandle {
                                task_ids,
                                completion_rxs,
                                ..
                            } = batch;
                            
                            // 将所有任务添加到 futures 和 active_tasks 中
                            for (task_id, rx) in task_ids.into_iter().zip(completion_rxs.into_iter()) {
                                // 记录到活跃任务集合
                                self.active_tasks.insert(task_id);
                                
                                let future: BoxFuture<'static, (TaskId, Result<(), tokio::sync::oneshot::error::RecvError>)> = Box::pin(async move {
                                    (task_id, rx.await)
                                });
                                futures.push(future);
                            }
                        }
                        ServiceCommand::AddTimerHandle(handle) => {
                            let TimerHandle{
                                task_id,
                                completion_rx,
                                ..
                            } = handle;
                            
                            // 记录到活跃任务集合
                            self.active_tasks.insert(task_id);
                            
                            // 添加到 futures 中
                            let future: BoxFuture<'static, (TaskId, Result<(), tokio::sync::oneshot::error::RecvError>)> = Box::pin(async move {
                                (task_id, completion_rx.0.await)
                            });
                            futures.push(future);
                        }
                        ServiceCommand::RemoveTasks { task_ids } => {
                            // 批量从活跃任务集合中移除任务
                            // 用于直接取消后的清理工作
                            for task_id in task_ids {
                                self.active_tasks.remove(&task_id);
                            }
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
        let timer = TimerWheel::with_defaults().unwrap();
        let _service = timer.create_service();
    }


    #[tokio::test]
    async fn test_add_timer_handle_and_receive_timeout() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = timer.create_service();

        // 创建单个定时器
        let handle = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        let task_id = handle.task_id();

        // 添加到 service
        service.add_timer_handle(handle).await.unwrap();

        // 接收超时通知
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id);
    }


    #[tokio::test]
    async fn test_shutdown() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();

        // 添加一些定时器
        let _task_id1 = service.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
        let _task_id2 = service.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();

        // 立即关闭（不等待定时器触发）
        service.shutdown().await;
    }



    #[tokio::test]
    async fn test_cancel_task() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();

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
        let service = timer.create_service();

        // 添加一个定时器以初始化 service
        let handle = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        service.add_timer_handle(handle).await.unwrap();

        // 尝试取消一个不存在的任务
        let fake_task_id = TaskId::new();
        let cancelled = service.cancel_task(fake_task_id).await.unwrap();
        assert!(!cancelled, "Nonexistent task should not be cancelled");
    }


    #[tokio::test]
    async fn test_task_timeout_cleans_up_task_sender() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = timer.create_service();

        // 添加一个短时间的定时器
        let handle = timer.schedule_once(Duration::from_millis(50), || async {}).await.unwrap();
        let task_id = handle.task_id();
        
        service.add_timer_handle(handle).await.unwrap();

        // 等待任务超时
        let mut rx = service.take_receiver().unwrap();
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

    #[tokio::test]
    async fn test_cancel_task_spawns_background_task() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建一个定时器
        let counter_clone = Arc::clone(&counter);
        let handle = timer.schedule_once(
            Duration::from_secs(10),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ).await.unwrap();
        let task_id = handle.task_id();
        
        service.add_timer_handle(handle).await.unwrap();

        // 使用 cancel_task（会等待结果，但在后台协程中处理）
        let cancelled = service.cancel_task(task_id).await.unwrap();
        assert!(cancelled, "Task should be cancelled successfully");

        // 等待足够长时间确保回调不会被执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Callback should not have been executed");

        // 验证任务已从 active_tasks 中移除
        let cancelled_again = service.cancel_task(task_id).await.unwrap();
        assert!(!cancelled_again, "Task should have been removed from active_tasks");
    }

    #[tokio::test]
    async fn test_schedule_once_direct() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 直接通过 service 调度定时器
        let counter_clone = Arc::clone(&counter);
        let task_id = service.schedule_once(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ).await.unwrap();

        // 等待定时器触发
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id);
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_schedule_once_batch_direct() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 直接通过 service 批量调度定时器
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

        let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
        assert_eq!(task_ids.len(), 3);

        // 接收所有超时通知
        let mut received_count = 0;
        let mut rx = service.take_receiver().unwrap();
        
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
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_schedule_once_notify_direct() {
        let timer = TimerWheel::with_defaults().unwrap();
        let mut service = timer.create_service();

        // 直接通过 service 调度仅通知的定时器
        let task_id = service.schedule_once_notify(Duration::from_millis(50)).await.unwrap();

        // 接收超时通知
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id);
    }

    #[tokio::test]
    async fn test_schedule_and_cancel_direct() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 直接调度定时器
        let counter_clone = Arc::clone(&counter);
        let task_id = service.schedule_once(
            Duration::from_secs(10),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ).await.unwrap();

        // 立即取消
        let cancelled = service.cancel_task(task_id).await.unwrap();
        assert!(cancelled, "Task should be cancelled successfully");

        // 等待确保回调不会执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Callback should not have been executed");
    }

    #[tokio::test]
    async fn test_cancel_batch_direct() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 批量调度定时器
        let callbacks: Vec<_> = (0..10)
            .map(|_| {
                let counter = Arc::clone(&counter);
                (Duration::from_secs(10), move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();

        let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
        assert_eq!(task_ids.len(), 10);

        // 批量取消所有任务
        let cancelled = service.cancel_batch(&task_ids).await;
        assert_eq!(cancelled, 10, "All 10 tasks should be cancelled");

        // 等待确保回调不会执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "No callbacks should have been executed");
    }

    #[tokio::test]
    async fn test_cancel_batch_partial() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 批量调度定时器
        let callbacks: Vec<_> = (0..10)
            .map(|_| {
                let counter = Arc::clone(&counter);
                (Duration::from_secs(10), move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();

        let task_ids = service.schedule_once_batch(callbacks).await.unwrap();

        // 只取消前5个任务
        let to_cancel: Vec<_> = task_ids.iter().take(5).copied().collect();
        let cancelled = service.cancel_batch(&to_cancel).await;
        assert_eq!(cancelled, 5, "5 tasks should be cancelled");

        // 等待确保前5个回调不会执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Cancelled tasks should not execute");
    }

    #[tokio::test]
    async fn test_cancel_batch_empty() {
        let timer = TimerWheel::with_defaults().unwrap();
        let service = timer.create_service();

        // 取消空列表
        let empty: Vec<TaskId> = vec![];
        let cancelled = service.cancel_batch(&empty).await;
        assert_eq!(cancelled, 0, "No tasks should be cancelled");
    }
}

