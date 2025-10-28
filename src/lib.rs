//! # 高性能异步定时器系统
//!
//! 基于时间轮（Timing Wheel）算法实现的高性能异步定时器，支持 tokio 运行时。
//!
//! ## 特性
//!
//! - **高性能**: 使用时间轮算法，插入和删除操作的时间复杂度为 O(1)
//! - **大规模支持**: 能够高效管理 10000+ 并发定时器
//! - **异步支持**: 基于 tokio 异步运行时
//! - **线程安全**: 使用 parking_lot 提供高性能的锁机制
//!
//! ## 快速开始
//!
//! ```no_run
//! use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
//! use std::time::Duration;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 创建定时器管理器
//!     let timer = TimerWheel::with_defaults();
//!     
//!     // 步骤 1: 创建定时器任务（使用回调）
//!     let callback = Some(CallbackWrapper::new(|| async {
//!         println!("Timer fired after 1 second!");
//!     }));
//!     let task = TimerWheel::create_task(Duration::from_secs(1), callback);
//!     let task_id = task.get_id();
//!     
//!     // 步骤 2: 注册定时器任务并获取完成通知
//!     let handle = timer.register(task);
//!     
//!     // 等待定时器完成
//!     handle.into_completion_receiver().0.await?;
//!     Ok(())
//! }
//! ```
//!
//! ## 架构说明
//!
//! ### 时间轮算法
//!
//! 采用分层时间轮（Hierarchical Timing Wheel）算法，包含 L0 和 L1 两层：
//!
//! - **L0 层（底层）**: 处理短延迟任务
//!   - 槽位数量: 默认 512 个（可配置，必须是 2 的幂次方）
//!   - 时间精度: 默认 10ms（可配置）
//!   - 最大时间跨度: 5.12 秒
//!
//! - **L1 层（高层）**: 处理长延迟任务
//!   - 槽位数量: 默认 64 个（可配置，必须是 2 的幂次方）
//!   - 时间精度: 默认 1 秒（可配置）
//!   - 最大时间跨度: 64 秒
//!
//! - **轮次机制**: 超出 L1 层范围的任务使用轮次计数处理
//!
//! ### 性能优化
//!
//! - 使用 `parking_lot::Mutex` 替代标准库的 Mutex，提供更好的性能
//! - 使用 `FxHashMap`（rustc-hash）替代标准 HashMap，减少哈希冲突
//! - 槽位数量为 2 的幂次方，使用位运算优化取模操作
//! - 任务执行在独立的 tokio 任务中，避免阻塞时间轮推进

mod config;
mod error;
mod task;
mod wheel;
mod timer;
mod service;

// 重新导出公共 API
pub use config::{
    BatchConfig,
    HierarchicalWheelConfig,
    ServiceConfig, ServiceConfigBuilder,
    TimerConfig, TimerConfigBuilder,
    WheelConfig, WheelConfigBuilder,
};
pub use error::TimerError;
pub use task::{CallbackWrapper, CompletionNotifier, TaskCompletionReason, TaskId, TimerCallback, TimerTask};
pub use timer::{BatchHandle, BatchHandleIter, CompletionReceiver, TimerHandle, TimerWheel};
pub use service::TimerService;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn test_basic_timer() {
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let task = TimerWheel::create_task(
            Duration::from_millis(50),
            Some(CallbackWrapper::new(move || {
                let counter =  Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            })),
        );
        timer.register(task);

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_multiple_timers() {
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建 10 个定时器
        for i in 0..10 {
            let counter_clone = Arc::clone(&counter);
            let task = TimerWheel::create_task(
                Duration::from_millis(10 * (i + 1)),
                Some(CallbackWrapper::new(move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })),
            );
            timer.register(task);
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[tokio::test]
    async fn test_timer_cancellation() {
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建 5 个定时器
        let mut handles = Vec::new();
        for _ in 0..5 {
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
            handles.push(handle);
        }

        // 取消前 3 个定时器
        for i in 0..3 {
            let cancel_result = handles[i].cancel();
            assert!(cancel_result);
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        // 只有 2 个定时器应该被触发
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_completion_notification_once() {
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
        let handle = timer.register(task);

        // 等待完成通知
        handle.into_completion_receiver().0.await.expect("Should receive completion notification");

        // 验证回调已执行（等待一下以确保回调执行完成）
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_notify_only_timer_once() {
        let timer = TimerWheel::with_defaults();
        
        let task = TimerTask::new(Duration::from_millis(50), None);
        let handle = timer.register(task);

        // 等待完成通知（无回调，仅通知）
        handle.into_completion_receiver().0.await.expect("Should receive completion notification");
    }

    #[tokio::test]
    async fn test_batch_completion_notifications() {
        let timer = TimerWheel::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));

        // 创建批量回调
        let callbacks: Vec<(Duration, Option<CallbackWrapper>)> = (0..5)
            .map(|i| {
                let counter = Arc::clone(&counter);
                let delay = Duration::from_millis(50 + i * 10);
                let callback = CallbackWrapper::new(move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                });
                (delay, Some(callback))
            })
            .collect();

        let tasks = TimerWheel::create_batch_with_callbacks(callbacks);
        let batch = timer.register_batch(tasks);
        let receivers = batch.into_completion_receivers();

        // 等待所有完成通知
        for rx in receivers {
            rx.await.expect("Should receive completion notification");
        }

        // 等待一下确保回调执行完成
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 验证所有回调都已执行
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }

    #[tokio::test]
    async fn test_completion_reason_expired() {
        let timer = TimerWheel::with_defaults();
        
        let task = TimerTask::new(Duration::from_millis(50), None);
        let handle = timer.register(task);

        // 等待完成通知并验证原因是 Expired
        let result = handle.into_completion_receiver().0.await.expect("Should receive completion notification");
        assert_eq!(result, TaskCompletionReason::Expired);
    }

    #[tokio::test]
    async fn test_completion_reason_cancelled() {
        let timer = TimerWheel::with_defaults();
        
        let task = TimerTask::new(Duration::from_secs(10), None);
        let handle = timer.register(task);

        // 取消任务
        let cancelled = handle.cancel();
        assert!(cancelled);

        // 等待完成通知并验证原因是 Cancelled
        let result = handle.into_completion_receiver().0.await.expect("Should receive completion notification");
        assert_eq!(result, TaskCompletionReason::Cancelled);
    }

    #[tokio::test]
    async fn test_batch_completion_reasons() {
        let timer = TimerWheel::with_defaults();
        
        // 创建 5 个任务，延迟 10 秒
        let tasks: Vec<_> = (0..5)
            .map(|_| TimerTask::new(Duration::from_secs(10), None))
            .collect();
        
        let batch = timer.register_batch(tasks);
        let task_ids: Vec<_> = batch.task_ids.clone();
        let mut receivers = batch.into_completion_receivers();

        // 取消前 3 个任务
        timer.cancel_batch(&task_ids[0..3]);

        // 验证前 3 个任务收到 Cancelled 通知
        for rx in receivers.drain(0..3) {
            let result = rx.await.expect("Should receive completion notification");
            assert_eq!(result, TaskCompletionReason::Cancelled);
        }

        // 取消剩余任务并验证
        timer.cancel_batch(&task_ids[3..5]);
        for rx in receivers {
            let result = rx.await.expect("Should receive completion notification");
            assert_eq!(result, TaskCompletionReason::Cancelled);
        }
    }
}
