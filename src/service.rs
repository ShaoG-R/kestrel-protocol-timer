use crate::config::ServiceConfig;
use crate::task::{TaskCompletionReason, TaskId, TimerCallback};
use crate::timer::{BatchHandle, TimerHandle};
use crate::wheel::Wheel;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::future::BoxFuture;
use parking_lot::Mutex;
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
///     let timer = TimerWheel::with_defaults();
///     let mut service = timer.create_service();
///     
///     // 使用两步式 API 通过 service 批量调度定时器
///     let callbacks: Vec<_> = (0..5)
///         .map(|_| (Duration::from_millis(100), || async {}))
///         .collect();
///     let tasks = TimerService::create_batch(callbacks);
///     service.register_batch(tasks).await;
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
    /// - `config`: 服务配置
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
    ///     let timer = TimerWheel::with_defaults();
    ///     let mut service = timer.create_service();
    /// }
    /// ```
    pub(crate) fn new(wheel: Arc<Mutex<Wheel>>, config: ServiceConfig) -> Self {
        let (command_tx, command_rx) = mpsc::channel(config.command_channel_capacity);
        let (timeout_tx, timeout_rx) = mpsc::channel(config.timeout_channel_capacity);

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
    async fn add_batch_handle(&self, batch: BatchHandle) {
        let _ = self.command_tx
            .send(ServiceCommand::AddBatchHandle(batch))
            .await;
    }

    /// 添加单个定时器句柄（内部方法）
    async fn add_timer_handle(&self, handle: TimerHandle) {
        let _ = self.command_tx
            .send(ServiceCommand::AddTimerHandle(handle))
            .await;
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
    /// let timer = TimerWheel::with_defaults();
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
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// // 使用两步式 API 调度定时器
    /// let task = TimerService::create_task(Duration::from_secs(10), || async {});
    /// let task_id = task.get_id();
    /// service.register(task).await;
    /// 
    /// // 取消任务
    /// let cancelled = service.cancel_task(task_id).await;
    /// println!("Task cancelled: {}", cancelled);
    /// # }
    /// ```
    #[inline]
    pub async fn cancel_task(&self, task_id: TaskId) -> bool {
        // 优化：直接取消任务，无需通知 Actor
        // FuturesUnordered 会在任务被取消时自动清理
        let mut wheel = self.wheel.lock();
        wheel.cancel(task_id)
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
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let callbacks: Vec<_> = (0..10)
    ///     .map(|_| (Duration::from_secs(10), || async {}))
    ///     .collect();
    /// let tasks = TimerService::create_batch(callbacks);
    /// let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
    /// service.register_batch(tasks).await;
    /// 
    /// // 批量取消
    /// let cancelled = service.cancel_batch(&task_ids).await;
    /// println!("成功取消 {} 个任务", cancelled);
    /// # }
    /// ```
    #[inline]
    pub async fn cancel_batch(&self, task_ids: &[TaskId]) -> usize {
        if task_ids.is_empty() {
            return 0;
        }

        // 优化：直接使用底层的批量取消，无需通知 Actor
        // FuturesUnordered 会在任务被取消时自动清理
        let mut wheel = self.wheel.lock();
        wheel.cancel_batch(task_ids)
    }

    /// 推迟任务（保持原回调）
    ///
    /// # 参数
    /// - `task_id`: 要推迟的任务 ID
    /// - `new_delay`: 新的延迟时间（从当前时间点重新计算）
    ///
    /// # 返回
    /// - `true`: 任务存在且成功推迟
    /// - `false`: 任务不存在或推迟失败
    ///
    /// # 性能说明
    /// 此方法使用直接推迟优化，不需要等待 Actor 处理，大幅降低延迟
    ///
    /// # 注意
    /// - 推迟后任务 ID 保持不变
    /// - 原有的超时通知仍然有效
    /// - 保持原回调函数不变
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let task = TimerService::create_task(Duration::from_secs(5), || async {});
    /// let task_id = task.get_id();
    /// service.register(task).await;
    /// 
    /// // 推迟到 10 秒后触发
    /// let success = service.postpone_task(task_id, Duration::from_secs(10)).await;
    /// println!("推迟成功: {}", success);
    /// # }
    /// ```
    #[inline]
    pub async fn postpone_task(&self, task_id: TaskId, new_delay: Duration) -> bool {
        // 优化：直接推迟任务，无需通知 Actor
        // FuturesUnordered 会继续监听原有的 completion_receiver
        let mut wheel = self.wheel.lock();
        wheel.postpone(task_id, new_delay, None)
    }

    /// 推迟任务（替换回调）
    ///
    /// # 参数
    /// - `task_id`: 要推迟的任务 ID
    /// - `new_delay`: 新的延迟时间（从当前时间点重新计算）
    /// - `callback`: 新的回调函数
    ///
    /// # 返回
    /// - `true`: 任务存在且成功推迟
    /// - `false`: 任务不存在或推迟失败
    ///
    /// # 注意
    /// - 推迟后任务 ID 保持不变
    /// - 原有的超时通知仍然有效
    /// - 回调函数会被替换为新的回调
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let task = TimerService::create_task(Duration::from_secs(5), || async {
    ///     println!("Original callback");
    /// });
    /// let task_id = task.get_id();
    /// service.register(task).await;
    /// 
    /// // 推迟并替换回调
    /// let success = service.postpone_task_with_callback(
    ///     task_id,
    ///     Duration::from_secs(10),
    ///     || async { println!("New callback!"); }
    /// ).await;
    /// println!("推迟成功: {}", success);
    /// # }
    /// ```
    #[inline]
    pub async fn postpone_task_with_callback<C>(
        &self,
        task_id: TaskId,
        new_delay: Duration,
        callback: C,
    ) -> bool
    where
        C: TimerCallback,
    {
        use std::sync::Arc;
        let callback_wrapper = Arc::new(callback) as Arc<dyn TimerCallback>;
        let mut wheel = self.wheel.lock();
        wheel.postpone(task_id, new_delay, Some(callback_wrapper))
    }

    /// 批量推迟任务（保持原回调）
    ///
    /// # 参数
    /// - `updates`: (任务ID, 新延迟) 的元组列表
    ///
    /// # 返回
    /// 成功推迟的任务数量
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|_| (Duration::from_secs(5), || async {}))
    ///     .collect();
    /// let tasks = TimerService::create_batch(callbacks);
    /// let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
    /// service.register_batch(tasks).await;
    /// 
    /// // 批量推迟
    /// let updates: Vec<_> = task_ids
    ///     .into_iter()
    ///     .map(|id| (id, Duration::from_secs(10)))
    ///     .collect();
    /// let postponed = service.postpone_batch(&updates).await;
    /// println!("成功推迟 {} 个任务", postponed);
    /// # }
    /// ```
    #[inline]
    pub async fn postpone_batch(&self, updates: &[(TaskId, Duration)]) -> usize {
        if updates.is_empty() {
            return 0;
        }

        let updates_vec: Vec<_> = updates
            .iter()
            .map(|(task_id, delay)| (*task_id, *delay, None))
            .collect();
        let mut wheel = self.wheel.lock();
        wheel.postpone_batch(updates_vec)
    }

    /// 批量推迟任务（替换回调）
    ///
    /// # 参数
    /// - `updates`: (任务ID, 新延迟, 新回调) 的元组列表
    ///
    /// # 返回
    /// 成功推迟的任务数量
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|_| (Duration::from_secs(5), || async {}))
    ///     .collect();
    /// let tasks = TimerService::create_batch(callbacks);
    /// let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
    /// service.register_batch(tasks).await;
    /// 
    /// // 批量推迟并替换回调
    /// let updates: Vec<_> = task_ids
    ///     .into_iter()
    ///     .enumerate()
    ///     .map(|(i, id)| {
    ///         (id, Duration::from_secs(10), move || async move {
    ///             println!("New callback {}", i);
    ///         })
    ///     })
    ///     .collect();
    /// let postponed = service.postpone_batch_with_callbacks(updates).await;
    /// println!("成功推迟 {} 个任务", postponed);
    /// # }
    /// ```
    #[inline]
    pub async fn postpone_batch_with_callbacks<C>(
        &self,
        updates: Vec<(TaskId, Duration, C)>,
    ) -> usize
    where
        C: TimerCallback,
    {
        if updates.is_empty() {
            return 0;
        }

        use std::sync::Arc;
        let updates_vec: Vec<_> = updates
            .into_iter()
            .map(|(task_id, delay, callback)| {
                let callback_wrapper = Arc::new(callback) as Arc<dyn TimerCallback>;
                (task_id, delay, Some(callback_wrapper))
            })
            .collect();
        let mut wheel = self.wheel.lock();
        wheel.postpone_batch(updates_vec)
    }

    /// 创建定时器任务（静态方法，申请阶段）
    /// 
    /// # 参数
    /// - `delay`: 延迟时间
    /// - `callback`: 实现了 TimerCallback trait 的回调对象
    /// 
    /// # 返回
    /// 返回 TimerTask，需要通过 `register()` 注册
    /// 
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// // 步骤 1: 创建任务
    /// let task = TimerService::create_task(Duration::from_millis(100), || async {
    ///     println!("Timer fired!");
    /// });
    /// 
    /// let task_id = task.get_id();
    /// println!("Created task: {:?}", task_id);
    /// 
    /// // 步骤 2: 注册任务
    /// service.register(task).await;
    /// # }
    /// ```
    pub fn create_task<C>(delay: Duration, callback: C) -> crate::task::TimerTask
    where
        C: TimerCallback,
    {
        crate::timer::TimerWheel::create_task(delay, callback)
    }
    
    /// 批量创建定时器任务（静态方法，申请阶段）
    /// 
    /// # 参数
    /// - `callbacks`: (延迟时间, 回调) 的元组列表
    /// 
    /// # 返回
    /// 返回 TimerTask 列表，需要通过 `register_batch()` 注册
    /// 
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// // 步骤 1: 批量创建任务
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|i| (Duration::from_millis(100 * (i + 1)), move || async move {
    ///         println!("Timer {} fired!", i);
    ///     }))
    ///     .collect();
    /// 
    /// let tasks = TimerService::create_batch(callbacks);
    /// println!("Created {} tasks", tasks.len());
    /// 
    /// // 步骤 2: 批量注册任务
    /// service.register_batch(tasks).await;
    /// # }
    /// ```
    pub fn create_batch<C>(callbacks: Vec<(Duration, C)>) -> Vec<crate::task::TimerTask>
    where
        C: TimerCallback,
    {
        crate::timer::TimerWheel::create_batch(callbacks)
    }
    
    /// 注册定时器任务到服务（注册阶段）
    /// 
    /// # 参数
    /// - `task`: 通过 `create_task()` 创建的任务
    /// 
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let task = TimerService::create_task(Duration::from_millis(100), || async {
    ///     println!("Timer fired!");
    /// });
    /// let task_id = task.get_id();
    /// 
    /// service.register(task).await;
    /// # }
    /// ```
    #[inline]
    pub async fn register(&self, task: crate::task::TimerTask) {
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
        let notifier = crate::task::CompletionNotifier(completion_tx);
        
        let delay = task.delay;
        let task_id = task.id;
        
        // 单次加锁完成所有操作
        {
            let mut wheel_guard = self.wheel.lock();
            wheel_guard.insert(delay, task, notifier);
        }
        
        // 创建句柄并添加到服务管理
        let handle = TimerHandle::new(task_id, self.wheel.clone(), completion_rx);
        self.add_timer_handle(handle).await;
    }
    
    /// 批量注册定时器任务到服务（注册阶段）
    /// 
    /// # 参数
    /// - `tasks`: 通过 `create_batch()` 创建的任务列表
    /// 
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, TimerService};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let service = timer.create_service();
    /// 
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|_| (Duration::from_secs(1), || async {}))
    ///     .collect();
    /// let tasks = TimerService::create_batch(callbacks);
    /// 
    /// service.register_batch(tasks).await;
    /// # }
    /// ```
    #[inline]
    pub async fn register_batch(&self, tasks: Vec<crate::task::TimerTask>) {
        let task_count = tasks.len();
        let mut completion_rxs = Vec::with_capacity(task_count);
        let mut task_ids = Vec::with_capacity(task_count);
        let mut prepared_tasks = Vec::with_capacity(task_count);
        
        // 步骤1: 准备所有 channels 和 notifiers（无锁）
        // 优化：使用 for 循环代替 map + collect，避免闭包捕获开销
        for task in tasks {
            let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
            let notifier = crate::task::CompletionNotifier(completion_tx);
            
            task_ids.push(task.id);
            completion_rxs.push(completion_rx);
            prepared_tasks.push((task.delay, task, notifier));
        }
        
        // 步骤2: 单次加锁，批量插入
        {
            let mut wheel_guard = self.wheel.lock();
            wheel_guard.insert_batch(prepared_tasks);
        }
        
        // 创建批量句柄并添加到服务管理
        let batch_handle = BatchHandle::new(task_ids, self.wheel.clone(), completion_rxs);
        self.add_batch_handle(batch_handle).await;
    }

    /// 优雅关闭 TimerService
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::TimerWheel;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
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
}

impl ServiceActor {
    fn new(command_rx: mpsc::Receiver<ServiceCommand>, timeout_tx: mpsc::Sender<TaskId>) -> Self {
        Self {
            command_rx,
            timeout_tx,
        }
    }

    async fn run(mut self) {
        // 使用 FuturesUnordered 来监听所有的 completion_rxs
        // 每个 future 返回 (TaskId, Result)
        let mut futures: FuturesUnordered<BoxFuture<'static, (TaskId, Result<TaskCompletionReason, tokio::sync::oneshot::error::RecvError>)>> = FuturesUnordered::new();

        loop {
            tokio::select! {
                // 监听超时事件
                Some((task_id, result)) = futures.next() => {
                    // 检查完成原因，只转发超时（Expired）事件，不转发取消（Cancelled）事件
                    if let Ok(TaskCompletionReason::Expired) = result {
                        let _ = self.timeout_tx.send(task_id).await;
                    }
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
                            
                            // 将所有任务添加到 futures 中
                            for (task_id, rx) in task_ids.into_iter().zip(completion_rxs.into_iter()) {
                                let future: BoxFuture<'static, (TaskId, Result<TaskCompletionReason, tokio::sync::oneshot::error::RecvError>)> = Box::pin(async move {
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
                            
                            // 添加到 futures 中
                            let future: BoxFuture<'static, (TaskId, Result<TaskCompletionReason, tokio::sync::oneshot::error::RecvError>)> = Box::pin(async move {
                                (task_id, completion_rx.0.await)
                            });
                            futures.push(future);
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
        let timer = TimerWheel::with_defaults();
        let _service = timer.create_service();
    }


    #[tokio::test]
    async fn test_add_timer_handle_and_receive_timeout() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();

        // 创建单个定时器
        let task = TimerWheel::create_task(Duration::from_millis(50), || async {});
        let task_id = task.get_id();
        let handle = timer.register(task);

        // 添加到 service
        service.add_timer_handle(handle).await;

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
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();

        // 添加一些定时器
        let task1 = TimerService::create_task(Duration::from_secs(10), || async {});
        let task2 = TimerService::create_task(Duration::from_secs(10), || async {});
        service.register(task1).await;
        service.register(task2).await;

        // 立即关闭（不等待定时器触发）
        service.shutdown().await;
    }



    #[tokio::test]
    async fn test_cancel_task() {
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();

        // 添加一个长时间的定时器
        let task = TimerWheel::create_task(Duration::from_secs(10), || async {});
        let task_id = task.get_id();
        let handle = timer.register(task);
        
        service.add_timer_handle(handle).await;

        // 取消任务
        let cancelled = service.cancel_task(task_id).await;
        assert!(cancelled, "Task should be cancelled successfully");

        // 尝试再次取消同一个任务，应该返回 false
        let cancelled_again = service.cancel_task(task_id).await;
        assert!(!cancelled_again, "Task should not exist anymore");
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_task() {
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();

        // 添加一个定时器以初始化 service
        let task = TimerWheel::create_task(Duration::from_millis(50), || async {});
        let handle = timer.register(task);
        service.add_timer_handle(handle).await;

        // 尝试取消一个不存在的任务（创建一个不会实际注册的任务ID）
        let fake_task = TimerWheel::create_task(Duration::from_millis(50), || async {});
        let fake_task_id = fake_task.get_id();
        // 不注册 fake_task
        let cancelled = service.cancel_task(fake_task_id).await;
        assert!(!cancelled, "Nonexistent task should not be cancelled");
    }


    #[tokio::test]
    async fn test_task_timeout_cleans_up_task_sender() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();

        // 添加一个短时间的定时器
        let task = TimerWheel::create_task(Duration::from_millis(50), || async {});
        let task_id = task.get_id();
        let handle = timer.register(task);
        
        service.add_timer_handle(handle).await;

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
        let cancelled = service.cancel_task(task_id).await;
        assert!(!cancelled, "Timed out task should not exist anymore");
    }

    #[tokio::test]
    async fn test_cancel_task_spawns_background_task() {
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建一个定时器
        let counter_clone = Arc::clone(&counter);
        let task = TimerWheel::create_task(
            Duration::from_secs(10),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let task_id = task.get_id();
        let handle = timer.register(task);
        
        service.add_timer_handle(handle).await;

        // 使用 cancel_task（会等待结果，但在后台协程中处理）
        let cancelled = service.cancel_task(task_id).await;
        assert!(cancelled, "Task should be cancelled successfully");

        // 等待足够长时间确保回调不会被执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Callback should not have been executed");

        // 验证任务已从 active_tasks 中移除
        let cancelled_again = service.cancel_task(task_id).await;
        assert!(!cancelled_again, "Task should have been removed from active_tasks");
    }

    #[tokio::test]
    async fn test_schedule_once_direct() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 直接通过 service 调度定时器
        let counter_clone = Arc::clone(&counter);
        let task = TimerService::create_task(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let task_id = task.get_id();
        service.register(task).await;

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
        let timer = TimerWheel::with_defaults();
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

        let tasks = TimerService::create_batch(callbacks);
        assert_eq!(tasks.len(), 3);
        service.register_batch(tasks).await;

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
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();

        // 直接通过 service 调度仅通知的定时器（无回调）
        let task = crate::task::TimerTask::new(Duration::from_millis(50), None);
        let task_id = task.get_id();
        service.register(task).await;

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
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 直接调度定时器
        let counter_clone = Arc::clone(&counter);
        let task = TimerService::create_task(
            Duration::from_secs(10),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let task_id = task.get_id();
        service.register(task).await;

        // 立即取消
        let cancelled = service.cancel_task(task_id).await;
        assert!(cancelled, "Task should be cancelled successfully");

        // 等待确保回调不会执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "Callback should not have been executed");
    }

    #[tokio::test]
    async fn test_cancel_batch_direct() {
        let timer = TimerWheel::with_defaults();
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

        let tasks = TimerService::create_batch(callbacks);
        let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
        assert_eq!(task_ids.len(), 10);
        service.register_batch(tasks).await;

        // 批量取消所有任务
        let cancelled = service.cancel_batch(&task_ids).await;
        assert_eq!(cancelled, 10, "All 10 tasks should be cancelled");

        // 等待确保回调不会执行
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0, "No callbacks should have been executed");
    }

    #[tokio::test]
    async fn test_cancel_batch_partial() {
        let timer = TimerWheel::with_defaults();
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

        let tasks = TimerService::create_batch(callbacks);
        let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
        service.register_batch(tasks).await;

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
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();

        // 取消空列表
        let empty: Vec<TaskId> = vec![];
        let cancelled = service.cancel_batch(&empty).await;
        assert_eq!(cancelled, 0, "No tasks should be cancelled");
    }

    #[tokio::test]
    async fn test_postpone_task() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 注册一个任务，延迟 50ms
        let counter_clone = Arc::clone(&counter);
        let task = TimerService::create_task(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let task_id = task.get_id();
        service.register(task).await;

        // 推迟任务到 150ms
        let postponed = service.postpone_task(task_id, Duration::from_millis(150)).await;
        assert!(postponed, "Task should be postponed successfully");

        // 等待原定时间 50ms，任务不应该触发
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        // 接收超时通知（从推迟开始算，还需要等待约 150ms）
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id);
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_postpone_task_with_callback() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 注册一个任务，原始回调增加 1
        let counter_clone1 = Arc::clone(&counter);
        let task = TimerService::create_task(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone1);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let task_id = task.get_id();
        service.register(task).await;

        // 推迟任务并替换回调，新回调增加 10
        let counter_clone2 = Arc::clone(&counter);
        let postponed = service.postpone_task_with_callback(
            task_id,
            Duration::from_millis(100),
            move || {
                let counter = Arc::clone(&counter_clone2);
                async move {
                    counter.fetch_add(10, Ordering::SeqCst);
                }
            }
        ).await;
        assert!(postponed, "Task should be postponed successfully");

        // 接收超时通知（推迟后需要等待100ms，加上余量）
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id);
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        
        // 验证新回调被执行（增加了 10 而不是 1）
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn test_postpone_nonexistent_task() {
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();

        // 尝试推迟一个不存在的任务
        let fake_task = TimerService::create_task(Duration::from_millis(50), || async {});
        let fake_task_id = fake_task.get_id();
        // 不注册这个任务
        
        let postponed = service.postpone_task(fake_task_id, Duration::from_millis(100)).await;
        assert!(!postponed, "Nonexistent task should not be postponed");
    }

    #[tokio::test]
    async fn test_postpone_batch() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 注册 3 个任务
        let mut task_ids = Vec::new();
        for _ in 0..3 {
            let counter_clone = Arc::clone(&counter);
            let task = TimerService::create_task(
                Duration::from_millis(50),
                move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                },
            );
            task_ids.push((task.get_id(), Duration::from_millis(150)));
            service.register(task).await;
        }

        // 批量推迟
        let postponed = service.postpone_batch(&task_ids).await;
        assert_eq!(postponed, 3, "All 3 tasks should be postponed");

        // 等待原定时间 50ms，任务不应该触发
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

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
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_postpone_batch_with_callbacks() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 注册 3 个任务
        let mut task_ids = Vec::new();
        for _ in 0..3 {
            let task = TimerService::create_task(
                Duration::from_millis(50),
                || async {},
            );
            task_ids.push(task.get_id());
            service.register(task).await;
        }

        // 批量推迟并替换回调
        let updates: Vec<_> = task_ids
            .into_iter()
            .map(|id| {
                let counter_clone = Arc::clone(&counter);
                (id, Duration::from_millis(150), move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();

        let postponed = service.postpone_batch_with_callbacks(updates).await;
        assert_eq!(postponed, 3, "All 3 tasks should be postponed");

        // 等待原定时间 50ms，任务不应该触发
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

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
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_postpone_batch_empty() {
        let timer = TimerWheel::with_defaults();
        let service = timer.create_service();

        // 推迟空列表
        let empty: Vec<(TaskId, Duration)> = vec![];
        let postponed = service.postpone_batch(&empty).await;
        assert_eq!(postponed, 0, "No tasks should be postponed");
    }

    #[tokio::test]
    async fn test_postpone_keeps_timeout_notification_valid() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();
        let counter = Arc::new(AtomicU32::new(0));

        // 注册一个任务
        let counter_clone = Arc::clone(&counter);
        let task = TimerService::create_task(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let task_id = task.get_id();
        service.register(task).await;

        // 推迟任务
        service.postpone_task(task_id, Duration::from_millis(100)).await;

        // 验证超时通知仍然有效（推迟后需要等待100ms，加上余量）
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        assert_eq!(received_task_id, task_id, "Timeout notification should still work after postpone");
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cancelled_task_not_forwarded_to_timeout_rx() {
        let timer = TimerWheel::with_defaults();
        let mut service = timer.create_service();

        // 注册两个任务：一个会被取消，一个会正常到期
        let task1 = TimerService::create_task(Duration::from_secs(10), || async {});
        let task1_id = task1.get_id();
        service.register(task1).await;

        let task2 = TimerService::create_task(Duration::from_millis(50), || async {});
        let task2_id = task2.get_id();
        service.register(task2).await;

        // 取消第一个任务
        let cancelled = service.cancel_task(task1_id).await;
        assert!(cancelled, "Task should be cancelled");

        // 等待第二个任务到期
        let mut rx = service.take_receiver().unwrap();
        let received_task_id = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive timeout notification")
            .expect("Should receive Some value");

        // 应该只收到第二个任务（到期的）的通知，不应该收到第一个任务（取消的）的通知
        assert_eq!(received_task_id, task2_id, "Should only receive expired task notification");

        // 验证没有其他通知（特别是被取消的任务不应该有通知）
        let no_more = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        assert!(no_more.is_err(), "Should not receive any more notifications");
    }
}

