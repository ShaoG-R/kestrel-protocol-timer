use crate::error::TimerError;
use crate::task::{CallbackWrapper, CompletionNotifier, TaskId, TimerCallback, TimerTask, TimerWheelId};
use crate::wheel::Wheel;
use crossbeam::channel::{Sender, Receiver};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 完成通知接收器，用于接收定时器完成通知
pub struct CompletionReceiver(pub oneshot::Receiver<()>);

/// 时间轮操作类型
pub(crate) enum WheelOperation {
    /// 插入定时器任务
    Insert {
        delay: Duration,
        task: TimerTask,
        result_tx: oneshot::Sender<TaskId>,
    },
    /// 批量插入定时器任务
    InsertBatch {
        tasks: Vec<(Duration, TimerTask)>,
        result_tx: oneshot::Sender<Vec<TaskId>>,
    },
    /// 取消定时器任务
    Cancel {
        task_id: TaskId,
        result_tx: Option<oneshot::Sender<bool>>,
    },
    /// 批量取消定时器任务
    CancelBatch {
        task_ids: Vec<TaskId>,
        result_tx: Option<oneshot::Sender<usize>>,
    },
}

/// 定时器句柄，用于管理定时器的生命周期
/// 
/// 注意：此类型不实现 Clone，以防止重复取消同一个定时器。
/// 每个定时器只应有一个所有者。
pub struct TimerHandle {
    pub(crate) task_id: TaskId,
    pub(crate) timer_wheel_id: TimerWheelId,
    pub(crate) op_sender: Sender<WheelOperation>,
    pub(crate) completion_rx: CompletionReceiver,
}

impl TimerHandle {
    fn new(task_id: TaskId, timer_wheel_id: TimerWheelId, op_sender: Sender<WheelOperation>, completion_rx: oneshot::Receiver<()>) -> Self {
        Self { task_id, timer_wheel_id, op_sender, completion_rx: CompletionReceiver(completion_rx) }
    }

    /// 取消定时器（异步获取结果）
    ///
    /// # 返回
    /// oneshot::Receiver<bool>，可通过 await 获取取消结果
    /// 如果任务存在且成功取消返回 true，否则返回 false
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let handle = timer.schedule_once(Duration::from_secs(1), || async {}).await.unwrap();
    /// 
    /// // 等待取消结果
    /// let success = handle.cancel().await.unwrap();
    /// println!("取消成功: {}", success);
    /// # }
    /// ```
    pub fn cancel(&self) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = self.op_sender.send(WheelOperation::Cancel {
            task_id: self.task_id,
            result_tx: Some(tx),
        });
        rx
    }

    /// 取消定时器（无需等待结果）
    ///
    /// 立即发送取消请求并返回，不等待取消结果。
    /// 适用于不关心取消是否成功的场景，性能更好。
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let handle = timer.schedule_once(Duration::from_secs(1), || async {}).await.unwrap();
    /// 
    /// // 立即发送取消请求，不等待结果
    /// handle.cancel_no_wait();
    /// # }
    /// ```
    pub fn cancel_no_wait(&self) {
        let _ = self.op_sender.send(WheelOperation::Cancel {
            task_id: self.task_id,
            result_tx: None,
        });
    }

    /// 获取任务 ID
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// 获取时间轮 ID
    pub(crate) fn timer_wheel_id(&self) -> TimerWheelId {
        self.timer_wheel_id
    }

    /// 获取完成通知接收器的可变引用
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let handle = timer.schedule_once(Duration::from_secs(1), || async {
    ///     println!("Timer fired!");
    /// }).await.unwrap();
    /// 
    /// // 等待定时器完成（使用 into_completion_receiver 消耗句柄）
    /// handle.into_completion_receiver().0.await.ok();
    /// println!("Timer completed!");
    /// # }
    /// ```
    pub fn completion_receiver(&mut self) -> &mut CompletionReceiver {
        &mut self.completion_rx
    }

    /// 消耗句柄，返回完成通知接收器
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let handle = timer.schedule_once(Duration::from_secs(1), || async {
    ///     println!("Timer fired!");
    /// }).await.unwrap();
    /// 
    /// // 等待定时器完成
    /// handle.into_completion_receiver().0.await.ok();
    /// println!("Timer completed!");
    /// # }
    /// ```
    pub fn into_completion_receiver(self) -> CompletionReceiver {
        self.completion_rx
    }
}

/// 批量定时器句柄，用于管理批量调度的定时器
/// 
/// 通过共用单个 Sender 减少内存开销，同时提供批量操作和迭代器访问能力。
/// 
/// 注意：此类型不实现 Clone，以防止重复取消同一批定时器。
/// 如需访问单个定时器句柄，请使用 `into_iter()` 或 `into_handles()` 进行转换。
pub struct BatchHandle {
    pub(crate) task_ids: Vec<TaskId>,
    pub(crate) timer_wheel_id: TimerWheelId,
    pub(crate) op_sender: Sender<WheelOperation>,
    pub(crate) completion_rxs: Vec<oneshot::Receiver<()>>,
}

impl BatchHandle {
    fn new(task_ids: Vec<TaskId>, timer_wheel_id: TimerWheelId, op_sender: Sender<WheelOperation>, completion_rxs: Vec<oneshot::Receiver<()>>) -> Self {
        Self { task_ids, timer_wheel_id, op_sender, completion_rxs }
    }

    /// 批量取消所有定时器（异步获取结果）
    ///
    /// # 返回
    /// - `Ok(usize)`: 成功取消的任务数量
    /// - `Err(TimerError)`: 内部通信通道已关闭
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let callbacks: Vec<_> = (0..10)
    ///     .map(|_| (Duration::from_secs(1), || async {}))
    ///     .collect();
    /// let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    /// 
    /// let cancelled = batch.cancel_all().await.unwrap();
    /// println!("取消了 {} 个定时器", cancelled);
    /// # }
    /// ```
    pub async fn cancel_all(self) -> Result<usize, TimerError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.op_sender.send(WheelOperation::CancelBatch {
            task_ids: self.task_ids,
            result_tx: Some(tx),
        });
        rx.await.map_err(|_| TimerError::ChannelClosed)
    }

    /// 批量取消所有定时器（无需等待结果）
    ///
    /// 立即发送批量取消请求并返回，不等待取消结果。
    /// 适用于不关心取消是否成功的场景，性能更好。
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let callbacks: Vec<_> = (0..10)
    ///     .map(|_| (Duration::from_secs(1), || async {}))
    ///     .collect();
    /// let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    /// 
    /// batch.cancel_all_no_wait();
    /// # }
    /// ```
    pub fn cancel_all_no_wait(self) {
        let _ = self.op_sender.send(WheelOperation::CancelBatch {
            task_ids: self.task_ids,
            result_tx: None,
        });
    }

    /// 将批量句柄转换为单个定时器句柄的 Vec
    ///
    /// 消耗 BatchHandle，为每个任务创建独立的 TimerHandle。
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|_| (Duration::from_secs(1), || async {}))
    ///     .collect();
    /// let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    /// 
    /// // 转换为独立的句柄
    /// let handles = batch.into_handles();
    /// for handle in handles {
    ///     // 可以单独操作每个句柄
    /// }
    /// # }
    /// ```
    pub fn into_handles(self) -> Vec<TimerHandle> {
        let timer_wheel_id = self.timer_wheel_id;
        self.task_ids
            .into_iter()
            .zip(self.completion_rxs.into_iter())
            .map(|(task_id, rx)| {
                TimerHandle::new(task_id, timer_wheel_id, self.op_sender.clone(), rx)
            })
            .collect()
    }

    /// 获取批量任务的数量
    pub fn len(&self) -> usize {
        self.task_ids.len()
    }

    /// 检查批量任务是否为空
    pub fn is_empty(&self) -> bool {
        self.task_ids.is_empty()
    }

    /// 获取所有任务 ID 的引用
    pub fn task_ids(&self) -> &[TaskId] {
        &self.task_ids
    }

    /// 获取时间轮 ID
    pub fn timer_wheel_id(&self) -> TimerWheelId {
        self.timer_wheel_id
    }

    /// 获取所有完成通知接收器的引用
    ///
    /// # 返回
    /// 所有任务的完成通知接收器列表引用
    pub fn completion_receivers(&mut self) -> &mut Vec<oneshot::Receiver<()>> {
        &mut self.completion_rxs
    }

    /// 消耗句柄，返回所有完成通知接收器
    ///
    /// # 返回
    /// 所有任务的完成通知接收器列表
    ///
    /// # 示例
    /// ```no_run
    /// # use timer::TimerWheel;
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults().unwrap();
    /// let callbacks: Vec<_> = (0..3)
    ///     .map(|_| (Duration::from_secs(1), || async {}))
    ///     .collect();
    /// let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    /// 
    /// // 获取所有完成通知接收器
    /// let receivers = batch.into_completion_receivers();
    /// for rx in receivers {
    ///     tokio::spawn(async move {
    ///         if rx.await.is_ok() {
    ///             println!("A timer completed!");
    ///         }
    ///     });
    /// }
    /// # }
    /// ```
    pub fn into_completion_receivers(self) -> Vec<oneshot::Receiver<()>> {
        self.completion_rxs
    }
}

/// 实现 IntoIterator，允许直接迭代 BatchHandle
/// 
/// # 示例
/// ```no_run
/// # use timer::TimerWheel;
/// # use std::time::Duration;
/// # #[tokio::main]
/// # async fn main() {
/// let timer = TimerWheel::with_defaults().unwrap();
/// let callbacks: Vec<_> = (0..3)
///     .map(|_| (Duration::from_secs(1), || async {}))
///     .collect();
/// let batch = timer.schedule_once_batch(callbacks).await.unwrap();
/// 
/// // 直接迭代，每个元素都是独立的 TimerHandle
/// for handle in batch {
///     // 可以单独操作每个句柄
/// }
/// # }
/// ```
impl IntoIterator for BatchHandle {
    type Item = TimerHandle;
    type IntoIter = BatchHandleIter;

    fn into_iter(self) -> Self::IntoIter {
        BatchHandleIter {
            task_ids: self.task_ids.into_iter(),
            completion_rxs: self.completion_rxs.into_iter(),
            timer_wheel_id: self.timer_wheel_id,
            op_sender: self.op_sender,
        }
    }
}

/// BatchHandle 的迭代器
pub struct BatchHandleIter {
    task_ids: std::vec::IntoIter<TaskId>,
    completion_rxs: std::vec::IntoIter<oneshot::Receiver<()>>,
    timer_wheel_id: TimerWheelId,
    op_sender: Sender<WheelOperation>,
}

impl Iterator for BatchHandleIter {
    type Item = TimerHandle;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.task_ids.next(), self.completion_rxs.next()) {
            (Some(task_id), Some(rx)) => {
                Some(TimerHandle::new(task_id, self.timer_wheel_id, self.op_sender.clone(), rx))
            }
            _ => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.task_ids.size_hint()
    }
}

impl ExactSizeIterator for BatchHandleIter {
    fn len(&self) -> usize {
        self.task_ids.len()
    }
}

/// 时间轮定时器管理器
pub struct TimerWheel {
    /// 时间轮唯一标识符
    wheel_id: TimerWheelId,
    
    /// 操作队列发送端
    op_sender: Sender<WheelOperation>,
    
    /// 后台 tick 循环任务句柄
    tick_handle: Option<JoinHandle<()>>,
}

impl TimerWheel {
    /// 创建新的定时器管理器
    ///
    /// # 参数
    /// - `tick_duration`: 每个 tick 的时间长度（建议 10ms）
    /// - `slot_count`: 槽位数量（必须是 2 的幂次方，建议 512 或 1024）
    ///
    /// # 返回
    /// - `Ok(Self)`: 成功创建定时器管理器
    /// - `Err(TimerError)`: 槽位数量无效
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::new(Duration::from_millis(10), 512).unwrap();
    /// }
    /// ```
    pub fn new(tick_duration: Duration, slot_count: usize) -> Result<Self, TimerError> {
        let wheel = Wheel::new(tick_duration, slot_count)?;
        let (op_sender, op_receiver) = crossbeam::channel::unbounded();

        // 启动后台 tick 循环
        let tick_handle = tokio::spawn(async move {
            Self::tick_loop(wheel, op_receiver, tick_duration).await;
        });

        Ok(Self {
            wheel_id: TimerWheelId::new(),
            op_sender,
            tick_handle: Some(tick_handle),
        })
    }

    /// 创建带默认配置的定时器管理器
    /// - tick 时长: 10ms
    /// - 槽位数量: 512
    ///
    /// # 返回
    /// - `Ok(Self)`: 成功创建定时器管理器
    /// - `Err(TimerError)`: 创建失败（不太可能，因为使用的是有效的默认值）
    pub fn with_defaults() -> Result<Self, TimerError> {
        Self::new(Duration::from_millis(10), 512)
    }

    /// 调度一次性定时器
    ///
    /// # 参数
    /// - `delay`: 延迟时间
    /// - `callback`: 实现了 TimerCallback trait 的回调对象
    ///
    /// # 返回
    /// - `Ok(TimerHandle)`: 成功调度，返回定时器句柄，可用于取消定时器
    /// - `Err(TimerError)`: 内部通信通道已关闭
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     
    ///     let handle = timer.schedule_once(Duration::from_secs(1), || async {
    ///         println!("Timer fired!");
    ///     }).await.unwrap();
    ///     
    ///     tokio::time::sleep(Duration::from_secs(2)).await;
    /// }
    /// ```
    pub async fn schedule_once<C>(&self, delay: Duration, callback: C) -> Result<TimerHandle, TimerError>
    where
        C: TimerCallback,
    {
        use std::sync::Arc;
        let (tx, rx) = oneshot::channel();
        let callback_wrapper = Arc::new(callback) as CallbackWrapper;
        
        // 创建完成通知 channel
        let (completion_tx, completion_rx) = oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        
        let task = TimerTask::once(0, 0, Some(callback_wrapper), notifier);
        
        let _ = self.op_sender.send(WheelOperation::Insert {
            delay,
            task,
            result_tx: tx,
        });
        
        let task_id = rx.await.map_err(|_| TimerError::ChannelClosed)?;
        Ok(TimerHandle::new(task_id, self.wheel_id, self.op_sender.clone(), completion_rx))
    }

    /// 批量调度一次性定时器
    ///
    /// # 参数
    /// - `tasks`: (延迟时间, 回调) 的元组列表
    ///
    /// # 返回
    /// - `Ok(BatchHandle)`: 成功调度，返回批量定时器句柄
    /// - `Err(TimerError)`: 内部通信通道已关闭
    ///
    /// # 性能优势
    /// - 批量处理减少通道通信开销
    /// - 内部优化批量插入操作
    /// - 共用单个 Sender 减少内存开销
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicU32, Ordering};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     let counter = Arc::new(AtomicU32::new(0));
    ///     
    ///     // 动态生成批量回调
    ///     let callbacks: Vec<(Duration, _)> = (0..3)
    ///         .map(|i| {
    ///             let counter = Arc::clone(&counter);
    ///             let delay = Duration::from_millis(100 + i * 100);
    ///             let callback = move || {
    ///                 let counter = Arc::clone(&counter);
    ///                 async move {
    ///                     counter.fetch_add(1, Ordering::SeqCst);
    ///                 }
    ///             };
    ///             (delay, callback)
    ///         })
    ///         .collect();
    ///     
    ///     let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    ///     println!("Scheduled {} timers", batch.len());
    ///     
    ///     // 批量取消所有定时器
    ///     let cancelled = batch.cancel_all().await.unwrap();
    ///     println!("Cancelled {} timers", cancelled);
    ///     
    ///     // 或者获取单个句柄
    ///     // if let Some(handle) = batch.get_handle(0) {
    ///     //     handle.cancel().await.unwrap();
    ///     // }
    /// }
    /// ```
    pub async fn schedule_once_batch<C>(&self, callbacks: Vec<(Duration, C)>) -> Result<BatchHandle, TimerError>
    where
        C: TimerCallback,
    {
        use std::sync::Arc;
        let (tx, rx) = oneshot::channel();
        
        // 为每个任务创建完成通知 channel
        let mut completion_rxs = Vec::with_capacity(callbacks.len());
        
        // 将回调转换为 TimerTask
        let tasks: Vec<(Duration, TimerTask)> = callbacks
            .into_iter()
            .map(|(delay, callback)| {
                let callback_wrapper = Arc::new(callback) as CallbackWrapper;
                
                // 创建完成通知 channel
                let (completion_tx, completion_rx) = oneshot::channel();
                completion_rxs.push(completion_rx);
                let notifier = CompletionNotifier(completion_tx);
                
                let task = TimerTask::once(0, 0, Some(callback_wrapper), notifier);
                (delay, task)
            })
            .collect();
        
        let _ = self.op_sender.send(WheelOperation::InsertBatch {
            tasks,
            result_tx: tx,
        });
        
        let task_ids = rx.await.map_err(|_| TimerError::ChannelClosed)?;
        Ok(BatchHandle::new(task_ids, self.wheel_id, self.op_sender.clone(), completion_rxs))
    }


    /// 调度一次性通知定时器（无回调，仅通知）
    ///
    /// # 参数
    /// - `delay`: 延迟时间
    ///
    /// # 返回
    /// - `Ok(TimerHandle)`: 成功调度，返回定时器句柄，可通过 `into_completion_receiver()` 获取通知接收器
    /// - `Err(TimerError)`: 内部通信通道已关闭
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     
    ///     let handle = timer.schedule_once_notify(Duration::from_secs(1)).await.unwrap();
    ///     
    ///     // 获取完成通知接收器
    ///     handle.into_completion_receiver().0.await.ok();
    ///     println!("Timer completed!");
    /// }
    /// ```
    pub async fn schedule_once_notify(&self, delay: Duration) -> Result<TimerHandle, TimerError> {
        let (tx, rx) = oneshot::channel();
        
        // 创建完成通知 channel
        let (completion_tx, completion_rx) = oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        
        // 无回调的任务
        let task = TimerTask::once(0, 0, None, notifier);
        
        let _ = self.op_sender.send(WheelOperation::Insert {
            delay,
            task,
            result_tx: tx,
        });
        
        let task_id = rx.await.map_err(|_| TimerError::ChannelClosed)?;
        Ok(TimerHandle::new(task_id, self.wheel_id, self.op_sender.clone(), completion_rx))
    }

    /// 取消定时器（异步获取结果）
    ///
    /// # 参数
    /// - `task_id`: 任务 ID
    ///
    /// # 返回
    /// oneshot::Receiver<bool>，可通过 await 获取取消结果
    /// 如果任务存在且成功取消返回 true，否则返回 false
    pub fn cancel(&self, task_id: TaskId) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = self.op_sender.send(WheelOperation::Cancel {
            task_id,
            result_tx: Some(tx),
        });
        rx
    }

    /// 取消定时器（无需等待结果）
    ///
    /// # 参数
    /// - `task_id`: 任务 ID
    ///
    /// 立即发送取消请求并返回，不等待取消结果。
    /// 适用于不关心取消是否成功的场景，性能更好。
    pub fn cancel_no_wait(&self, task_id: TaskId) {
        let _ = self.op_sender.send(WheelOperation::Cancel {
            task_id,
            result_tx: None,
        });
    }

    /// 批量取消定时器（异步获取结果）
    ///
    /// # 参数
    /// - `task_ids`: 要取消的任务 ID 列表
    ///
    /// # 返回
    /// - `Ok(usize)`: 成功取消的任务数量
    /// - `Err(TimerError)`: 内部通信通道已关闭
    ///
    /// # 性能优势
    /// - 批量处理减少通道通信开销
    /// - 内部优化批量取消操作
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     
    ///     // 创建多个定时器
    ///     let handle1 = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    ///     let handle2 = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    ///     let handle3 = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    ///     
    ///     // 批量取消
    ///     let task_ids = vec![handle1.task_id(), handle2.task_id(), handle3.task_id()];
    ///     let cancelled = timer.cancel_batch(&task_ids).await.unwrap();
    ///     println!("已取消 {} 个定时器", cancelled);
    /// }
    /// ```
    pub async fn cancel_batch(&self, task_ids: &[TaskId]) -> Result<usize, TimerError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.op_sender.send(WheelOperation::CancelBatch {
            task_ids: task_ids.to_vec(),
            result_tx: Some(tx),
        });
        rx.await.map_err(|_| TimerError::ChannelClosed)
    }

    /// 批量取消定时器（无需等待结果）
    ///
    /// # 参数
    /// - `task_ids`: 要取消的任务 ID 列表
    ///
    /// 立即发送批量取消请求并返回，不等待取消结果。
    /// 适用于不关心取消是否成功的场景，性能更好。
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     
    ///     let handle1 = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    ///     let handle2 = timer.schedule_once(Duration::from_secs(10), || async {}).await.unwrap();
    ///     
    ///     // 立即批量取消，不等待结果
    ///     let task_ids = vec![handle1.task_id(), handle2.task_id()];
    ///     timer.cancel_batch_no_wait(&task_ids);
    /// }
    /// ```
    pub fn cancel_batch_no_wait(&self, task_ids: &[TaskId]) {
        let _ = self.op_sender.send(WheelOperation::CancelBatch {
            task_ids: task_ids.to_vec(),
            result_tx: None,
        });
    }
    
    /// 核心 tick 循环
    async fn tick_loop(mut wheel: Wheel, op_receiver: Receiver<WheelOperation>, tick_duration: Duration) {
        let mut interval = tokio::time::interval(tick_duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            // 1. 处理队列中的所有操作
            while let Ok(op) = op_receiver.try_recv() {
                match op {
                    WheelOperation::Insert { delay, task, result_tx } => {
                        let task_id = wheel.insert(delay, task);
                        let _ = result_tx.send(task_id);
                    }
                    WheelOperation::InsertBatch { tasks, result_tx } => {
                        let task_ids = wheel.insert_batch(tasks);
                        let _ = result_tx.send(task_ids);
                    }
                    WheelOperation::Cancel { task_id, result_tx } => {
                        let success = wheel.cancel(task_id);
                        // 只在需要返回结果时才发送
                        if let Some(tx) = result_tx {
                            let _ = tx.send(success);
                        }
                    }
                    WheelOperation::CancelBatch { task_ids, result_tx } => {
                        let cancelled_count = wheel.cancel_batch(&task_ids);
                        // 只在需要返回结果时才发送
                        if let Some(tx) = result_tx {
                            let _ = tx.send(cancelled_count);
                        }
                    }
                }
            }

            // 2. 推进时间轮并获取到期任务
            let expired_tasks = wheel.advance();

            // 3. 执行到期任务
            for task in expired_tasks {
                let callback = task.get_callback();
                
                // 移动task的所有权来获取completion_notifier
                let notifier = task.completion_notifier;
                
                // 在独立的 tokio 任务中执行回调
                if let Some(callback) = callback {
                    tokio::spawn(async move {
                        let future = callback.call();
                        future.await;
                    });
                }
                
                // 发送完成通知（在回调执行后立即发送，不等待回调完成）
                let _ = notifier.0.send(());
            }
        }
    }

    /// 停止定时器管理器
    pub async fn shutdown(mut self) {
        if let Some(handle) = self.tick_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for TimerWheel {
    fn drop(&mut self) {
        if let Some(handle) = self.tick_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_timer_creation() {
        let _timer = TimerWheel::with_defaults().unwrap();
    }

    #[tokio::test]
    async fn test_schedule_once() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let _handle = timer.schedule_once(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ).await.unwrap();

        // 等待定时器触发
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cancel_timer() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let handle = timer.schedule_once(
            Duration::from_millis(100),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ).await.unwrap();

        // 立即取消（等待结果）
        let cancel_result = handle.cancel().await.unwrap();
        assert!(cancel_result);

        // 等待足够长时间确保定时器不会触发
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_cancel_no_wait() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let handle = timer.schedule_once(
            Duration::from_millis(100),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ).await.unwrap();

        // 立即取消（无需等待结果）
        handle.cancel_no_wait();

        // 等待足够长时间确保定时器不会触发
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}

