use crate::error::TimerError;
use crate::task::{CallbackWrapper, TaskId, TimerCallback, TimerTask};
use crate::wheel::Wheel;
use crossbeam::channel::{Sender, Receiver};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 时间轮操作类型
enum WheelOperation {
    /// 插入定时器任务
    Insert {
        delay: Duration,
        task: TimerTask,
        result_tx: oneshot::Sender<TaskId>,
    },
    /// 取消定时器任务
    Cancel {
        task_id: TaskId,
        result_tx: Option<oneshot::Sender<bool>>,
    },
}

/// 定时器句柄，用于管理定时器的生命周期
#[derive(Clone)]
pub struct TimerHandle {
    task_id: TaskId,
    op_sender: Sender<WheelOperation>,
}

impl TimerHandle {
    fn new(task_id: TaskId, op_sender: Sender<WheelOperation>) -> Self {
        Self { task_id, op_sender }
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
}

/// 时间轮定时器管理器
pub struct TimerWheel {
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
        let task = TimerTask::once(0, 0, callback_wrapper);
        
        let _ = self.op_sender.send(WheelOperation::Insert {
            delay,
            task,
            result_tx: tx,
        });
        
        let task_id = rx.await.map_err(|_| TimerError::ChannelClosed)?;
        Ok(TimerHandle::new(task_id, self.op_sender.clone()))
    }

    /// 调度周期性定时器
    ///
    /// # 参数
    /// - `interval`: 周期间隔
    /// - `callback`: 实现了 TimerCallback trait 的回调对象（会在每个周期被调用）
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
    /// use std::sync::atomic::{AtomicU32, Ordering};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults().unwrap();
    ///     let counter = Arc::new(AtomicU32::new(0));
    ///     let counter_clone = Arc::clone(&counter);
    ///     
    ///     let handle = timer.schedule_repeat(Duration::from_secs(1), move || {
    ///         let counter = Arc::clone(&counter_clone);
    ///         async move {
    ///             let count = counter.fetch_add(1, Ordering::SeqCst);
    ///             println!("Periodic timer fired! Count: {}", count + 1);
    ///         }
    ///     }).await.unwrap();
    ///     
    ///     tokio::time::sleep(Duration::from_secs(5)).await;
    /// }
    /// ```
    pub async fn schedule_repeat<C>(&self, interval: Duration, callback: C) -> Result<TimerHandle, TimerError>
    where
        C: TimerCallback,
    {
        use std::sync::Arc;
        let (tx, rx) = oneshot::channel();
        let callback_wrapper = Arc::new(callback) as CallbackWrapper;
        let task = TimerTask::repeat(0, 0, interval, callback_wrapper);
        
        let _ = self.op_sender.send(WheelOperation::Insert {
            delay: interval,
            task,
            result_tx: tx,
        });
        
        let task_id = rx.await.map_err(|_| TimerError::ChannelClosed)?;
        Ok(TimerHandle::new(task_id, self.op_sender.clone()))
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
                    WheelOperation::Cancel { task_id, result_tx } => {
                        let success = wheel.cancel(task_id);
                        // 只在需要返回结果时才发送
                        if let Some(tx) = result_tx {
                            let _ = tx.send(success);
                        }
                    }
                }
            }

            // 2. 推进时间轮并获取到期任务
            let expired_tasks = wheel.advance();

            // 3. 执行到期任务
            for task in expired_tasks {
                let callback = task.get_callback();
                let is_repeat = task.is_repeat();
                let task_interval = task.interval();
                
                // 在独立的 tokio 任务中执行回调，避免阻塞时间轮
                tokio::spawn(async move {
                    let future = callback.call();
                    future.await;
                });

                // 4. 如果是周期性任务，直接重新调度（无需通过队列）
                if is_repeat {
                    if let Some(interval) = task_interval {
                        let new_task = task.clone_for_repeat(0, 0);
                        wheel.insert(interval, new_task);
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

