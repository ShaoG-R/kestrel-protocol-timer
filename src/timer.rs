use crate::task::{CallbackWrapper, TaskId, TimerCallback, TimerTask};
use crate::wheel::Wheel;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// 定时器句柄，用于管理定时器的生命周期
#[derive(Clone)]
pub struct TimerHandle {
    task_id: TaskId,
    wheel: Arc<Mutex<Wheel>>,
}

impl TimerHandle {
    fn new(task_id: TaskId, wheel: Arc<Mutex<Wheel>>) -> Self {
        Self { task_id, wheel }
    }

    /// 取消定时器
    ///
    /// # 返回
    /// 如果任务存在且成功取消返回 true，否则返回 false
    pub fn cancel(&self) -> bool {
        self.wheel.lock().cancel(self.task_id)
    }

    /// 获取任务 ID
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }
}

/// 时间轮定时器管理器
pub struct TimerWheel {
    /// 时间轮（使用 Arc<Mutex> 保证线程安全）
    wheel: Arc<Mutex<Wheel>>,
    
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
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::new(Duration::from_millis(10), 512);
    /// }
    /// ```
    pub fn new(tick_duration: Duration, slot_count: usize) -> Self {
        let wheel = Arc::new(Mutex::new(Wheel::new(tick_duration, slot_count)));
        let wheel_clone = Arc::clone(&wheel);

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
    pub fn with_defaults() -> Self {
        Self::new(Duration::from_millis(10), 512)
    }

    /// 调度一次性定时器
    ///
    /// # 参数
    /// - `delay`: 延迟时间
    /// - `callback`: 实现了 TimerCallback trait 的回调对象
    ///
    /// # 返回
    /// 定时器句柄，可用于取消定时器
    ///
    /// # 示例
    /// ```no_run
    /// use timer::TimerWheel;
    /// use std::time::Duration;
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let timer = TimerWheel::with_defaults();
    ///     
    ///     let handle = timer.schedule_once(Duration::from_secs(1), || async {
    ///         println!("Timer fired!");
    ///     });
    ///     
    ///     tokio::time::sleep(Duration::from_secs(2)).await;
    /// }
    /// ```
    pub fn schedule_once<C>(&self, delay: Duration, callback: C) -> TimerHandle
    where
        C: TimerCallback,
    {
        let mut wheel = self.wheel.lock();
        let callback_wrapper = Arc::new(callback) as CallbackWrapper;
        let task = TimerTask::once(0, 0, callback_wrapper);
        let task_id = wheel.insert(delay, task);
        TimerHandle::new(task_id, Arc::clone(&self.wheel))
    }

    /// 调度周期性定时器
    ///
    /// # 参数
    /// - `interval`: 周期间隔
    /// - `callback`: 实现了 TimerCallback trait 的回调对象（会在每个周期被调用）
    ///
    /// # 返回
    /// 定时器句柄，可用于取消定时器
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
    ///     let timer = TimerWheel::with_defaults();
    ///     let counter = Arc::new(AtomicU32::new(0));
    ///     let counter_clone = Arc::clone(&counter);
    ///     
    ///     let handle = timer.schedule_repeat(Duration::from_secs(1), move || {
    ///         let counter = Arc::clone(&counter_clone);
    ///         async move {
    ///             let count = counter.fetch_add(1, Ordering::SeqCst);
    ///             println!("Periodic timer fired! Count: {}", count + 1);
    ///         }
    ///     });
    ///     
    ///     tokio::time::sleep(Duration::from_secs(5)).await;
    /// }
    /// ```
    pub fn schedule_repeat<C>(&self, interval: Duration, callback: C) -> TimerHandle
    where
        C: TimerCallback,
    {
        let mut wheel = self.wheel.lock();
        let callback_wrapper = Arc::new(callback) as CallbackWrapper;
        let task = TimerTask::repeat(0, 0, interval, callback_wrapper);
        let task_id = wheel.insert(interval, task);
        TimerHandle::new(task_id, Arc::clone(&self.wheel))
    }

    /// 取消定时器
    ///
    /// # 参数
    /// - `task_id`: 任务 ID
    ///
    /// # 返回
    /// 如果任务存在且成功取消返回 true，否则返回 false
    pub fn cancel(&self, task_id: TaskId) -> bool {
        self.wheel.lock().cancel(task_id)
    }

    /// 获取当前活跃的定时器数量
    pub fn task_count(&self) -> usize {
        self.wheel.lock().task_count()
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
                let is_repeat = task.is_repeat();
                let interval = task.interval();
                
                // 在独立的 tokio 任务中执行回调，避免阻塞时间轮
                tokio::spawn(async move {
                    let future = callback.call();
                    future.await;
                });

                // 如果是周期性任务，重新调度
                if is_repeat {
                    if let Some(interval) = interval {
                        let mut wheel_guard = wheel.lock();
                        let new_task = task.clone_for_repeat(0, 0);
                        wheel_guard.insert(interval, new_task);
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
        let timer = TimerWheel::with_defaults();
        assert_eq!(timer.task_count(), 0);
    }

    #[tokio::test]
    async fn test_schedule_once() {
        let timer = TimerWheel::with_defaults();
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
        );

        // 等待定时器触发
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cancel_timer() {
        let timer = TimerWheel::with_defaults();
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
        );

        // 立即取消
        assert!(handle.cancel());

        // 等待足够长时间确保定时器不会触发
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}

