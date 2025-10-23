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
//! use timer::TimerWheel;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     // 创建定时器管理器
//!     let timer = TimerWheel::with_defaults();
//!     
//!     // 调度一次性定时器
//!     let handle = timer.schedule_once(Duration::from_secs(1), Box::new(|| {
//!         Box::pin(async {
//!             println!("Timer fired after 1 second!");
//!         })
//!     }));
//!     
//!     // 等待定时器触发
//!     tokio::time::sleep(Duration::from_secs(2)).await;
//! }
//! ```
//!
//! ## 架构说明
//!
//! ### 时间轮算法
//!
//! 时间轮是一个环形数组，每个槽位存储一组定时器任务。时间轮以固定的频率（tick）推进，
//! 当指针移动到某个槽位时，该槽位中的所有任务会被检查是否到期。
//!
//! - **槽位数量**: 默认 512 个（可配置，必须是 2 的幂次方）
//! - **时间精度**: 默认 10ms（可配置）
//! - **最大时间跨度**: 槽位数量 × 时间精度（默认 5.12 秒）
//! - **轮次机制**: 超出时间轮范围的任务使用轮次计数处理
//!
//! ### 性能优化
//!
//! - 使用 `parking_lot::Mutex` 替代标准库的 Mutex，提供更好的性能
//! - 使用 `FxHashMap`（rustc-hash）替代标准 HashMap，减少哈希冲突
//! - 槽位数量为 2 的幂次方，使用位运算优化取模操作
//! - 任务执行在独立的 tokio 任务中，避免阻塞时间轮推进

mod task;
mod wheel;
mod timer;

// 重新导出公共 API
pub use task::{CallbackWrapper, TaskId, TimerCallback};
pub use timer::{TimerHandle, TimerWheel};

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

        timer.schedule_once(
            Duration::from_millis(50),
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );

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
            timer.schedule_once(
                Duration::from_millis(10 * (i + 1)),
                move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                },
            );
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
            let handle = timer.schedule_once(
                Duration::from_millis(100),
                move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                },
            );
            handles.push(handle);
        }

        // 取消前 3 个定时器
        for i in 0..3 {
            assert!(handles[i].cancel());
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        // 只有 2 个定时器应该被触发
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }
}
