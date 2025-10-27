use crate::config::{ServiceConfig, WheelConfig};
use crate::task::{CallbackWrapper, TaskId, TaskCompletionReason};
use crate::wheel::Wheel;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 完成通知接收器，用于接收定时器完成通知
pub struct CompletionReceiver(pub oneshot::Receiver<TaskCompletionReason>);

/// 定时器句柄，用于管理定时器的生命周期
/// 
/// 注意：此类型不实现 Clone，以防止重复取消同一个定时器。
/// 每个定时器只应有一个所有者。
pub struct TimerHandle {
    pub(crate) task_id: TaskId,
    pub(crate) wheel: Arc<Mutex<Wheel>>,
    pub(crate) completion_rx: CompletionReceiver,
}

impl TimerHandle {
    pub(crate) fn new(task_id: TaskId, wheel: Arc<Mutex<Wheel>>, completion_rx: oneshot::Receiver<TaskCompletionReason>) -> Self {
        Self { task_id, wheel, completion_rx: CompletionReceiver(completion_rx) }
    }

    /// 取消定时器
    ///
    /// # 返回
    /// 如果任务存在且成功取消返回 true，否则返回 false
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
    /// # use std::time::Duration;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let callback = Some(CallbackWrapper::new(|| async {}));
    /// let task = TimerWheel::create_task(Duration::from_secs(1), callback);
    /// let handle = timer.register(task);
    /// 
    /// // 取消定时器
    /// let success = handle.cancel();
    /// println!("取消成功: {}", success);
    /// # }
    /// ```
    pub fn cancel(&self) -> bool {
        let mut wheel = self.wheel.lock();
        wheel.cancel(self.task_id)
    }

    /// 获取完成通知接收器的可变引用
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
    /// # use std::time::Duration;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let callback = Some(CallbackWrapper::new(|| async {
    ///     println!("Timer fired!");
    /// }));
    /// let task = TimerWheel::create_task(Duration::from_secs(1), callback);
    /// let handle = timer.register(task);
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
    /// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
    /// # use std::time::Duration;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let callback = Some(CallbackWrapper::new(|| async {
    ///     println!("Timer fired!");
    /// }));
    /// let task = TimerWheel::create_task(Duration::from_secs(1), callback);
    /// let handle = timer.register(task);
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
/// 通过共享 Wheel 引用减少内存开销，同时提供批量操作和迭代器访问能力。
/// 
/// 注意：此类型不实现 Clone，以防止重复取消同一批定时器。
/// 如需访问单个定时器句柄，请使用 `into_iter()` 或 `into_handles()` 进行转换。
pub struct BatchHandle {
    pub(crate) task_ids: Vec<TaskId>,
    pub(crate) wheel: Arc<Mutex<Wheel>>,
    pub(crate) completion_rxs: Vec<oneshot::Receiver<TaskCompletionReason>>,
}

impl BatchHandle {
    pub(crate) fn new(task_ids: Vec<TaskId>, wheel: Arc<Mutex<Wheel>>, completion_rxs: Vec<oneshot::Receiver<TaskCompletionReason>>) -> Self {
        Self { task_ids, wheel, completion_rxs }
    }

    /// 批量取消所有定时器
    ///
    /// # 返回
    /// 成功取消的任务数量
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
    /// # use std::time::Duration;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..10)
    ///     .map(|_| (Duration::from_secs(1), Some(CallbackWrapper::new(|| async {}))))
    ///     .collect();
    /// let tasks = TimerWheel::create_batch(callbacks);
    /// let batch = timer.register_batch(tasks);
    /// 
    /// let cancelled = batch.cancel_all();
    /// println!("取消了 {} 个定时器", cancelled);
    /// # }
    /// ```
    pub fn cancel_all(self) -> usize {
        let mut wheel = self.wheel.lock();
        wheel.cancel_batch(&self.task_ids)
    }

    /// 将批量句柄转换为单个定时器句柄的 Vec
    ///
    /// 消耗 BatchHandle，为每个任务创建独立的 TimerHandle。
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
    /// # use std::time::Duration;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..3)
    ///     .map(|_| (Duration::from_secs(1), Some(CallbackWrapper::new(|| async {}))))
    ///     .collect();
    /// let tasks = TimerWheel::create_batch(callbacks);
    /// let batch = timer.register_batch(tasks);
    /// 
    /// // 转换为独立的句柄
    /// let handles = batch.into_handles();
    /// for handle in handles {
    ///     // 可以单独操作每个句柄
    /// }
    /// # }
    /// ```
    pub fn into_handles(self) -> Vec<TimerHandle> {
        self.task_ids
            .into_iter()
            .zip(self.completion_rxs.into_iter())
            .map(|(task_id, rx)| {
                TimerHandle::new(task_id, self.wheel.clone(), rx)
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

    /// 获取所有完成通知接收器的引用
    ///
    /// # 返回
    /// 所有任务的完成通知接收器列表引用
    pub fn completion_receivers(&mut self) -> &mut Vec<oneshot::Receiver<TaskCompletionReason>> {
        &mut self.completion_rxs
    }

    /// 消耗句柄，返回所有完成通知接收器
    ///
    /// # 返回
    /// 所有任务的完成通知接收器列表
    ///
    /// # 示例
    /// ```no_run
    /// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
    /// # use std::time::Duration;
    /// # 
    /// # #[tokio::main]
    /// # async fn main() {
    /// let timer = TimerWheel::with_defaults();
    /// let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..3)
    ///     .map(|_| (Duration::from_secs(1), Some(CallbackWrapper::new(|| async {}))))
    ///     .collect();
    /// let tasks = TimerWheel::create_batch(callbacks);
    /// let batch = timer.register_batch(tasks);
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
    pub fn into_completion_receivers(self) -> Vec<oneshot::Receiver<TaskCompletionReason>> {
        self.completion_rxs
    }
}

/// 实现 IntoIterator，允许直接迭代 BatchHandle
/// 
/// # 示例
/// ```no_run
/// # use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
/// # use std::time::Duration;
/// # 
/// # #[tokio::main]
/// # async fn main() {
/// let timer = TimerWheel::with_defaults();
/// let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..3)
///     .map(|_| (Duration::from_secs(1), Some(CallbackWrapper::new(|| async {}))))
///     .collect();
/// let tasks = TimerWheel::create_batch(callbacks);
/// let batch = timer.register_batch(tasks);
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
            wheel: self.wheel,
        }
    }
}

/// BatchHandle 的迭代器
pub struct BatchHandleIter {
    task_ids: std::vec::IntoIter<TaskId>,
    completion_rxs: std::vec::IntoIter<oneshot::Receiver<TaskCompletionReason>>,
    wheel: Arc<Mutex<Wheel>>,
}

impl Iterator for BatchHandleIter {
    type Item = TimerHandle;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.task_ids.next(), self.completion_rxs.next()) {
            (Some(task_id), Some(rx)) => {
                Some(TimerHandle::new(task_id, self.wheel.clone(), rx))
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
    
    /// 时间轮实例（使用 Arc<Mutex> 包装以支持多线程访问）
    wheel: Arc<Mutex<Wheel>>,
    
    /// 后台 tick 循环任务句柄
    tick_handle: Option<JoinHandle<()>>,
}

impl TimerWheel {
    /// 创建新的定时器管理器
    ///
    /// # 参数
    /// - `config`: 时间轮配置（已经过验证）
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, WheelConfig, TimerTask};
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let config = WheelConfig::builder()
    ///         .tick_duration(Duration::from_millis(10))
    ///         .slot_count(512)
    ///         .build()
    ///         .unwrap();
    ///     let timer = TimerWheel::new(config);
    ///     
    ///     // 使用两步式 API
    ///     let task = TimerWheel::create_task(Duration::from_secs(1), None);
    ///     let handle = timer.register(task);
    /// }
    /// ```
    pub fn new(config: WheelConfig) -> Self {
        let tick_duration = config.tick_duration;
        let wheel = Wheel::new(config);
        let wheel = Arc::new(Mutex::new(wheel));
        let wheel_clone = wheel.clone();

        // 启动后台 tick 循环
        let tick_handle = tokio::spawn(async move {
            Self::tick_loop(wheel_clone, tick_duration).await;
        });

        Self {
            wheel,
            tick_handle: Some(tick_handle),
        }
    }

    /// 创建带默认配置的定时器管理器
    /// - tick 时长: 10ms
    /// - 槽位数量: 512
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::TimerWheel;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    /// }
    /// ```
    pub fn with_defaults() -> Self {
        Self::new(WheelConfig::default())
    }

    /// 创建与此时间轮绑定的 TimerService（使用默认配置）
    ///
    /// # 返回
    /// 绑定到此时间轮的 TimerService 实例
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerService, CallbackWrapper};
    /// use std::time::Duration;
    /// 
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     let mut service = timer.create_service();
    ///     
    ///     // 使用两步式 API 通过 service 批量调度定时器
    ///     let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..5)
    ///         .map(|_| (Duration::from_millis(100), Some(CallbackWrapper::new(|| async {}))))
    ///         .collect();
    ///     let tasks = TimerService::create_batch(callbacks);
    ///     service.register_batch(tasks).unwrap();
    ///     
    ///     // 接收超时通知
    ///     let mut rx = service.take_receiver().unwrap();
    ///     while let Some(task_id) = rx.recv().await {
    ///         println!("Task {:?} completed", task_id);
    ///     }
    /// }
    /// ```
    pub fn create_service(&self) -> crate::service::TimerService {
        crate::service::TimerService::new(self.wheel.clone(), ServiceConfig::default())
    }
    
    /// 创建与此时间轮绑定的 TimerService（使用自定义配置）
    ///
    /// # 参数
    /// - `config`: 服务配置
    ///
    /// # 返回
    /// 绑定到此时间轮的 TimerService 实例
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, ServiceConfig};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     let config = ServiceConfig::builder()
    ///         .command_channel_capacity(1024)
    ///         .timeout_channel_capacity(2000)
    ///         .build()
    ///         .unwrap();
    ///     let service = timer.create_service_with_config(config);
    /// }
    /// ```
    pub fn create_service_with_config(&self, config: ServiceConfig) -> crate::service::TimerService {
        crate::service::TimerService::new(self.wheel.clone(), config)
    }

    /// 创建定时器任务（申请阶段）
    /// 
    /// # 参数
    /// - `delay`: 延迟时间
    /// - `callback`: 实现了 TimerCallback trait 的回调对象
    /// 
    /// # 返回
    /// 返回 TimerTask，需要通过 `register()` 注册到时间轮
    /// 
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// use std::time::Duration;
    /// 
    /// 
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     // 步骤 1: 创建任务
    ///     let task = TimerWheel::create_task(Duration::from_secs(1), Some(CallbackWrapper::new(|| async {
    ///         println!("Timer fired!");
    ///     })));
    ///     
    ///     // 获取任务 ID
    ///     let task_id = task.get_id();
    ///     println!("Created task: {:?}", task_id);
    ///     
    ///     // 步骤 2: 注册任务
    ///     let handle = timer.register(task);
    /// }
    /// ```
    #[inline]
    pub fn create_task(delay: Duration, callback: Option<CallbackWrapper>) -> crate::task::TimerTask {
        crate::task::TimerTask::new(delay, callback)
    }
    
    /// 批量创建定时器任务（申请阶段）
    /// 
    /// # 参数
    /// - `callbacks`: (延迟时间, 回调) 的元组列表
    /// 
    /// # 返回
    /// 返回 TimerTask 列表，需要通过 `register_batch()` 注册到时间轮
    /// 
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// use std::time::Duration;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicU32, Ordering};
    /// 
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     let counter = Arc::new(AtomicU32::new(0));
    ///     
    ///     // 步骤 1: 批量创建任务
    ///     let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..3)
    ///         .map(|i| {
    ///             let counter = Arc::clone(&counter);
    ///             let delay = Duration::from_millis(100 + i * 100);
    ///             let callback = Some(CallbackWrapper::new(move || {
    ///                 let counter = Arc::clone(&counter);
    ///                 async move {
    ///                     counter.fetch_add(1, Ordering::SeqCst);
    ///                 }
    ///             }));
    ///             (delay, callback)
    ///         })
    ///         .collect();
    ///     
    ///     let tasks = TimerWheel::create_batch(callbacks);
    ///     println!("Created {} tasks", tasks.len());
    ///     
    ///     // 步骤 2: 批量注册任务
    ///     let batch = timer.register_batch(tasks);
    /// }
    /// ```
    #[inline]
    pub fn create_batch(callbacks: Vec<(Duration, Option<CallbackWrapper>)>) -> Vec<crate::task::TimerTask>
    {
        callbacks
            .into_iter()
            .map(|(delay, callback)| crate::task::TimerTask::new(delay, callback))
            .collect()
    }
    
    /// 注册定时器任务到时间轮（注册阶段）
    /// 
    /// # 参数
    /// - `task`: 通过 `create_task()` 创建的任务
    /// 
    /// # 返回
    /// 返回定时器句柄，可用于取消定时器和接收完成通知
    /// 
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// 
    /// use std::time::Duration;
    /// 
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     let task = TimerWheel::create_task(Duration::from_secs(1), Some(CallbackWrapper::new(|| async {
    ///         println!("Timer fired!");
    ///     })));
    ///     let task_id = task.get_id();
    ///     
    ///     let handle = timer.register(task);
    ///     
    ///     // 等待定时器完成
    ///     handle.into_completion_receiver().0.await.ok();
    /// }
    /// ```
    #[inline]
    pub fn register(&self, task: crate::task::TimerTask) -> TimerHandle {
        let (completion_tx, completion_rx) = oneshot::channel();
        let notifier = crate::task::CompletionNotifier(completion_tx);
        
        let delay = task.delay;
        let task_id = task.id;
        
        // 单次加锁完成所有操作
        {
            let mut wheel_guard = self.wheel.lock();
            wheel_guard.insert(delay, task, notifier);
        }
        
        TimerHandle::new(task_id, self.wheel.clone(), completion_rx)
    }
    
    /// 批量注册定时器任务到时间轮（注册阶段）
    /// 
    /// # 参数
    /// - `tasks`: 通过 `create_batch()` 创建的任务列表
    /// 
    /// # 返回
    /// 返回批量定时器句柄
    /// 
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask};
    /// use std::time::Duration;
    /// 
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     let callbacks: Vec<_> = (0..3)
    ///         .map(|_| (Duration::from_secs(1), None))
    ///         .collect();
    ///     let tasks = TimerWheel::create_batch(callbacks);
    ///     
    ///     let batch = timer.register_batch(tasks);
    ///     println!("Registered {} timers", batch.len());
    /// }
    /// ```
    #[inline]
    pub fn register_batch(&self, tasks: Vec<crate::task::TimerTask>) -> BatchHandle {
        let task_count = tasks.len();
        let mut completion_rxs = Vec::with_capacity(task_count);
        let mut task_ids = Vec::with_capacity(task_count);
        let mut prepared_tasks = Vec::with_capacity(task_count);
        
        // 步骤1: 准备所有 channels 和 notifiers（无锁）
        // 优化：使用 for 循环代替 map + collect，避免闭包捕获开销
        for task in tasks {
            let (completion_tx, completion_rx) = oneshot::channel();
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
        
        BatchHandle::new(task_ids, self.wheel.clone(), completion_rxs)
    }

    /// 取消定时器
    ///
    /// # 参数
    /// - `task_id`: 任务 ID
    ///
    /// # 返回
    /// 如果任务存在且成功取消返回 true，否则返回 false
    /// 
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// 
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     let task = TimerWheel::create_task(Duration::from_secs(10), Some(CallbackWrapper::new(|| async {
    ///         println!("Timer fired!");
    ///     })));
    ///     let task_id = task.get_id();
    ///     let _handle = timer.register(task);
    ///     
    ///     // 使用任务 ID 取消
    ///     let cancelled = timer.cancel(task_id);
    ///     println!("取消成功: {}", cancelled);
    /// }
    /// ```
    #[inline]
    pub fn cancel(&self, task_id: TaskId) -> bool {
        let mut wheel = self.wheel.lock();
        wheel.cancel(task_id)
    }

    /// 批量取消定时器
    ///
    /// # 参数
    /// - `task_ids`: 要取消的任务 ID 列表
    ///
    /// # 返回
    /// 成功取消的任务数量
    ///
    /// # 性能优势
    /// - 批量处理减少锁竞争
    /// - 内部优化批量取消操作
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask};
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     // 创建多个定时器
    ///     let task1 = TimerWheel::create_task(Duration::from_secs(10), None);
    ///     let task2 = TimerWheel::create_task(Duration::from_secs(10), None);
    ///     let task3 = TimerWheel::create_task(Duration::from_secs(10), None);
    ///     
    ///     let task_ids = vec![task1.get_id(), task2.get_id(), task3.get_id()];
    ///     
    ///     let _h1 = timer.register(task1);
    ///     let _h2 = timer.register(task2);
    ///     let _h3 = timer.register(task3);
    ///     
    ///     // 批量取消
    ///     let cancelled = timer.cancel_batch(&task_ids);
    ///     println!("已取消 {} 个定时器", cancelled);
    /// }
    /// ```
    #[inline]
    pub fn cancel_batch(&self, task_ids: &[TaskId]) -> usize {
        let mut wheel = self.wheel.lock();
        wheel.cancel_batch(task_ids)
    }

    /// 推迟定时器
    ///
    /// # 参数
    /// - `task_id`: 要推迟的任务 ID
    /// - `new_delay`: 新的延迟时间（从当前时间点重新计算）
    /// - `callback`: 新的回调函数，传入 `None` 保持原回调不变，传入 `Some` 替换为新回调
    ///
    /// # 返回
    /// 如果任务存在且成功推迟返回 true，否则返回 false
    ///
    /// # 注意
    /// - 推迟后任务 ID 保持不变
    /// - 原有的 completion_receiver 仍然有效
    ///
    /// # 示例
    ///
    /// ## 保持原回调
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// use std::time::Duration;
    /// 
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     let task = TimerWheel::create_task(Duration::from_secs(5), Some(CallbackWrapper::new(|| async {
    ///         println!("Timer fired!");
    ///     })));
    ///     let task_id = task.get_id();
    ///     let _handle = timer.register(task);
    ///     
    ///     // 推迟到 10 秒后触发（保持原回调）
    ///     let success = timer.postpone(task_id, Duration::from_secs(10), None);
    ///     println!("推迟成功: {}", success);
    /// }
    /// ```
    ///
    /// ## 替换为新回调
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     let task = TimerWheel::create_task(Duration::from_secs(5), Some(CallbackWrapper::new(|| async {
    ///         println!("Original callback!");
    ///     })));
    ///     let task_id = task.get_id();
    ///     let _handle = timer.register(task);
    ///     
    ///     // 推迟到 10 秒后触发（并替换为新回调）
    ///     let success = timer.postpone(task_id, Duration::from_secs(10), Some(CallbackWrapper::new(|| async {
    ///         println!("New callback!");
    ///     })));
    ///     println!("推迟成功: {}", success);
    /// }
    /// ```
    #[inline]
    pub fn postpone(
        &self,
        task_id: TaskId,
        new_delay: Duration,
        callback: Option<CallbackWrapper>,
    ) -> bool {
        let mut wheel = self.wheel.lock();
        wheel.postpone(task_id, new_delay, callback)
    }

    /// 批量推迟定时器（保持原回调）
    ///
    /// # 参数
    /// - `updates`: (任务ID, 新延迟) 的元组列表
    ///
    /// # 返回
    /// 成功推迟的任务数量
    ///
    /// # 注意
    /// - 此方法会保持所有任务的原回调不变
    /// - 如需替换回调，请使用 `postpone_batch_with_callbacks`
    ///
    /// # 性能优势
    /// - 批量处理减少锁竞争
    /// - 内部优化批量推迟操作
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     // 创建多个带回调的定时器
    ///     let task1 = TimerWheel::create_task(Duration::from_secs(5), Some(CallbackWrapper::new(|| async {
    ///         println!("Task 1 fired!");
    ///     })));
    ///     let task2 = TimerWheel::create_task(Duration::from_secs(5), Some(CallbackWrapper::new(|| async {
    ///         println!("Task 2 fired!");
    ///     })));
    ///     let task3 = TimerWheel::create_task(Duration::from_secs(5), Some(CallbackWrapper::new(|| async {
    ///         println!("Task 3 fired!");
    ///     })));
    ///     
    ///     let task_ids = vec![
    ///         (task1.get_id(), Duration::from_secs(10)),
    ///         (task2.get_id(), Duration::from_secs(15)),
    ///         (task3.get_id(), Duration::from_secs(20)),
    ///     ];
    ///     
    ///     timer.register(task1);
    ///     timer.register(task2);
    ///     timer.register(task3);
    ///     
    ///     // 批量推迟（保持原回调）
    ///     let postponed = timer.postpone_batch(&task_ids);
    ///     println!("已推迟 {} 个定时器", postponed);
    /// }
    /// ```
    #[inline]
    pub fn postpone_batch(&self, updates: &[(TaskId, Duration)]) -> usize {
        let mut wheel = self.wheel.lock();
        wheel.postpone_batch(updates.to_vec())
    }

    /// 批量推迟定时器（替换回调）
    ///
    /// # 参数
    /// - `updates`: (任务ID, 新延迟, 新回调) 的元组列表
    ///
    /// # 返回
    /// 成功推迟的任务数量
    ///
    /// # 性能优势
    /// - 批量处理减少锁竞争
    /// - 内部优化批量推迟操作
    ///
    /// # 示例
    /// ```no_run
    /// use kestrel_protocol_timer::{TimerWheel, TimerTask, CallbackWrapper};
    /// use std::time::Duration;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicU32, Ordering};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     let counter = Arc::new(AtomicU32::new(0));
    ///     
    ///     // 创建多个定时器
    ///     let task1 = TimerWheel::create_task(Duration::from_secs(5), None);
    ///     let task2 = TimerWheel::create_task(Duration::from_secs(5), None);
    ///     
    ///     let id1 = task1.get_id();
    ///     let id2 = task2.get_id();
    ///     
    ///     timer.register(task1);
    ///     timer.register(task2);
    ///     
    ///     // 批量推迟并替换回调
    ///     let updates: Vec<_> = vec![id1, id2]
    ///         .into_iter()
    ///         .map(|id| {
    ///             let counter = Arc::clone(&counter);
    ///             (id, Duration::from_secs(10), Some(CallbackWrapper::new(move || {
    ///                 let counter = Arc::clone(&counter);
    ///                 async move { counter.fetch_add(1, Ordering::SeqCst); }
    ///             })))
    ///         })
    ///         .collect();
    ///     let postponed = timer.postpone_batch_with_callbacks(updates);
    ///     println!("已推迟 {} 个定时器", postponed);
    /// }
    /// ```
    #[inline]
    pub fn postpone_batch_with_callbacks(
        &self,
        updates: Vec<(TaskId, Duration, Option<CallbackWrapper>)>,
    ) -> usize {
        let mut wheel = self.wheel.lock();
        wheel.postpone_batch_with_callbacks(updates.to_vec())
    }
    
    /// 核心 tick 循环
    async fn tick_loop(wheel: Arc<Mutex<Wheel>>, tick_duration: Duration) {
        let mut interval = tokio::time::interval(tick_duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            // 推进时间轮并获取到期任务
            let expired_tasks = {
                let mut wheel_guard = wheel.lock();
                wheel_guard.advance()
            };

            // 执行到期任务
            for task in expired_tasks {
                let callback = task.get_callback();
                
                // 移动task的所有权来获取completion_notifier
                let notifier = task.completion_notifier;
                
                // 只有注册过的任务才有 notifier
                if let Some(notifier) = notifier {
                    // 在独立的 tokio 任务中执行回调，并在回调完成后发送通知
                    if let Some(callback) = callback {
                        tokio::spawn(async move {
                            // 执行回调
                            let future = callback.call();
                            future.await;
                            
                            // 回调执行完成后发送通知
                            let _ = notifier.0.send(TaskCompletionReason::Expired);
                        });
                    } else {
                        // 如果没有回调，立即发送完成通知
                        let _ = notifier.0.send(TaskCompletionReason::Expired);
                    }
                }
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
        let _timer = TimerWheel::with_defaults();
    }

    #[tokio::test]
    async fn test_schedule_once() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let task = TimerWheel::create_task(
            Duration::from_millis(50),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        let _handle = timer.register(task);

        // 等待定时器触发
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cancel_timer() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let task = TimerWheel::create_task(
            Duration::from_millis(100),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        let handle = timer.register(task);

        // 立即取消
        let cancel_result = handle.cancel();
        assert!(cancel_result);

        // 等待足够长时间确保定时器不会触发
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_cancel_immediate() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let task = TimerWheel::create_task(
            Duration::from_millis(100),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        let handle = timer.register(task);

        // 立即取消
        let cancel_result = handle.cancel();
        assert!(cancel_result);

        // 等待足够长时间确保定时器不会触发
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_postpone_timer() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let task = TimerWheel::create_task(
            Duration::from_millis(50),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        let task_id = task.get_id();
        let handle = timer.register(task);

        // 推迟任务到 150ms
        let postponed = timer.postpone(task_id, Duration::from_millis(150), None);
        assert!(postponed);

        // 等待原定时间 50ms，任务不应该触发
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        // 等待新的触发时间（从推迟开始算，还需要等待约 150ms）
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            handle.into_completion_receiver().0
        ).await;
        assert!(result.is_ok());
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_postpone_with_callback() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone1 = Arc::clone(&counter);
        let counter_clone2 = Arc::clone(&counter);

        // 创建任务，原始回调增加 1
        let task = TimerWheel::create_task(
            Duration::from_millis(50),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone1);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        let task_id = task.get_id();
        let handle = timer.register(task);

        // 推迟任务并替换回调，新回调增加 10
        let postponed = timer.postpone(
            task_id,
            Duration::from_millis(100),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone2);
                async move {
                    counter.fetch_add(10, Ordering::SeqCst);
                }
            })),
        );
        assert!(postponed);

        // 等待任务触发（推迟后需要等待100ms，加上余量）
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            handle.into_completion_receiver().0
        ).await;
        assert!(result.is_ok());
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        
        // 验证新回调被执行（增加了 10 而不是 1）
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn test_postpone_nonexistent_timer() {
        let timer = TimerWheel::with_defaults();
        
        // 尝试推迟不存在的任务
        let fake_task = TimerWheel::create_task(Duration::from_millis(50), None);
        let fake_task_id = fake_task.get_id();
        // 不注册这个任务
        
        let postponed = timer.postpone(fake_task_id, Duration::from_millis(100), None);
        assert!(!postponed);
    }

    #[tokio::test]
    async fn test_postpone_batch() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建 3 个任务
        let mut task_ids = Vec::new();
        for _ in 0..3 {
            let counter_clone = Arc::clone(&counter);
            let task = TimerWheel::create_task(
                Duration::from_millis(50),
                Some(CallbackWrapper::new(move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })),
            );
            task_ids.push((task.get_id(), Duration::from_millis(150)));
            timer.register(task);
        }

        // 批量推迟
        let postponed = timer.postpone_batch(&task_ids);
        assert_eq!(postponed, 3);

        // 等待原定时间 50ms，任务不应该触发
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        // 等待新的触发时间（从推迟开始算，还需要等待约 150ms）
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_postpone_batch_with_callbacks() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建 3 个任务
        let mut task_ids = Vec::new();
        for _ in 0..3 {
            let task = TimerWheel::create_task(
                Duration::from_millis(50),
                None
            );
            task_ids.push(task.get_id());
            timer.register(task);
        }

        // 批量推迟并替换回调
        let updates: Vec<_> = task_ids
            .into_iter()
            .map(|id| {
                let counter_clone = Arc::clone(&counter);
                (id, Duration::from_millis(150), Some(CallbackWrapper::new(move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })))
            })
            .collect();

        let postponed = timer.postpone_batch_with_callbacks(updates);
        assert_eq!(postponed, 3);

        // 等待原定时间 50ms，任务不应该触发
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        // 等待新的触发时间（从推迟开始算，还需要等待约 150ms）
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_postpone_keeps_completion_receiver_valid() {
        use std::sync::Arc;
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let task = TimerWheel::create_task(
            Duration::from_millis(50),
            Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        let task_id = task.get_id();
        let handle = timer.register(task);

        // 推迟任务
        timer.postpone(task_id, Duration::from_millis(100), None);

        // 验证原 completion_receiver 仍然有效（推迟后需要等待100ms，加上余量）
        let result = tokio::time::timeout(
            Duration::from_millis(200),
            handle.into_completion_receiver().0
        ).await;
        assert!(result.is_ok(), "Completion receiver should still work after postpone");
        
        // 等待回调执行
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}

