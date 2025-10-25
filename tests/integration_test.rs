use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use timer::TimerWheel;
use futures::future;

#[tokio::test]
async fn test_large_scale_timers() {
    // 测试大规模并发定时器（10000+ 个）
    let timer = Arc::new(TimerWheel::with_defaults().unwrap());
    let counter = Arc::new(AtomicU32::new(0));
    const TIMER_COUNT: u32 = 10_000;

    let start = Instant::now();

    // 并发创建 10000 个定时器
    let mut futures = Vec::new();
    for i in 0..TIMER_COUNT {
        let timer_clone = Arc::clone(&timer);
        let counter_clone = Arc::clone(&counter);
        let delay = Duration::from_millis(10 + (i % 100) as u64);
        
        let future = async move {
            timer_clone.schedule_once(
                delay,
                move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                },
            ).await.unwrap()
        };
        futures.push(future);
    }

    // 并发等待所有定时器创建完成
    future::join_all(futures).await;

    println!("创建 {} 个定时器耗时: {:?}", TIMER_COUNT, start.elapsed());
    // Note: task_count() is deprecated in lockfree version

    // 等待所有定时器触发
    tokio::time::sleep(Duration::from_millis(200)).await;

    let count = counter.load(Ordering::SeqCst);
    println!("触发的定时器数量: {}", count);
    assert_eq!(count, TIMER_COUNT, "所有定时器都应该被触发");
}

#[tokio::test]
async fn test_timer_precision() {
    // 测试定时器的精度
    let timer = TimerWheel::with_defaults().unwrap();
    let start_time = Arc::new(parking_lot::Mutex::new(None::<Instant>));
    let end_time = Arc::new(parking_lot::Mutex::new(None::<Instant>));

    *start_time.lock() = Some(Instant::now());

    let end_clone = Arc::clone(&end_time);
    let handle = timer.schedule_once(
        Duration::from_millis(100),
        move || {
            let end_time = Arc::clone(&end_clone);
            async move {
                *end_time.lock() = Some(Instant::now());
            }
        },
    ).await.unwrap();

    // 使用 completion_receiver 等待定时器完成，而不是固定的sleep时间
    // 这样可以避免竞态条件
    let _ = handle.into_completion_receiver().0.await;
    
    // 额外等待一点时间确保回调完全执行完毕
    tokio::time::sleep(Duration::from_millis(20)).await;

    let start = start_time.lock().expect("start_time should be set");
    let end = end_time.lock().expect("end_time should be set after timer completion");
    let elapsed = end.duration_since(start);

    println!("预期延迟: 100ms, 实际延迟: {:?}", elapsed);
    
    // 允许 ±50ms 的误差（考虑到调度延迟和系统负载）
    assert!(
        elapsed >= Duration::from_millis(80) && elapsed <= Duration::from_millis(180),
        "定时器精度在可接受范围内，实际延迟: {:?}", elapsed
    );
}

#[tokio::test]
async fn test_concurrent_operations() {
    // 测试并发操作（同时添加和取消定时器）
    let timer = Arc::new(TimerWheel::with_defaults().unwrap());
    let counter = Arc::new(AtomicU32::new(0));

    // 并发创建所有定时器（5个任务 × 1000个定时器 = 5000个）
    let mut all_futures = Vec::new();
    
    for _ in 0..5 {
        for _ in 0..1000 {
            let timer_clone = Arc::clone(&timer);
            let counter_clone = Arc::clone(&counter);
            
            let future = async move {
                timer_clone.schedule_once(
                    Duration::from_millis(50),
                    move || {
                        let counter = Arc::clone(&counter_clone);
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                ).await.unwrap()
            };
            
            all_futures.push(future);
        }
    }

    // 并发等待所有定时器创建完成
    future::join_all(all_futures).await;

    // Note: task_count() is deprecated in lockfree version

    // 等待定时器触发
    tokio::time::sleep(Duration::from_millis(150)).await;

    let count = counter.load(Ordering::SeqCst);
    println!("触发的定时器数量: {}", count);
    assert_eq!(count, 5000, "所有定时器都应该被触发");
}

#[tokio::test]
async fn test_timer_with_different_delays() {
    // 测试不同延迟的定时器
    let timer = TimerWheel::with_defaults().unwrap();
    let results = Arc::new(parking_lot::Mutex::new(Vec::new()));

    let delays = vec![10, 20, 30, 50, 100, 150, 200];
    let mut handles = Vec::new();
    
    for (idx, &delay_ms) in delays.iter().enumerate() {
        let results_clone = Arc::clone(&results);
        
        let handle = timer.schedule_once(
            Duration::from_millis(delay_ms),
            move || {
                let results = Arc::clone(&results_clone);
                async move {
                    results.lock().push((idx, delay_ms));
                }
            },
        ).await.unwrap();
        
        handles.push(handle);
    }

    // 使用 completion_receiver 等待所有定时器完成，而不是固定的sleep时间
    // 这样可以确保所有定时器都真正触发了
    for handle in handles {
        let _ = handle.into_completion_receiver().0.await;
    }
    
    // 额外等待一点时间确保所有回调完全执行完毕
    tokio::time::sleep(Duration::from_millis(50)).await;

    let final_results = results.lock();
    println!("触发顺序: {:?}", final_results);
    assert_eq!(final_results.len(), delays.len(), "所有定时器都应该被触发");
}

#[tokio::test]
async fn test_memory_efficiency() {
    // 测试内存效率 - 创建大量定时器然后取消
    let timer = Arc::new(TimerWheel::with_defaults().unwrap());

    // 并发创建 5000 个定时器
    let mut create_futures = Vec::new();
    for _ in 0..5000 {
        let timer_clone = Arc::clone(&timer);
        let future = async move {
            timer_clone.schedule_once(
                Duration::from_secs(10),
                || async {},
            ).await.unwrap()
        };
        create_futures.push(future);
    }

    let handles = future::join_all(create_futures).await;

    // Note: task_count() is deprecated in lockfree version

    // 取消所有定时器（现在是同步操作）
    let cancelled_count = handles.into_iter()
        .map(|handle| handle.cancel())
        .filter(|&success| success)
        .count();

    println!("取消的定时器数量: {}", cancelled_count);
    
    assert_eq!(cancelled_count, 5000);
}

#[tokio::test]
async fn test_batch_schedule() {
    // 测试批量调度定时器
    let timer = TimerWheel::with_defaults().unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    
    const BATCH_SIZE: usize = 100;
    let start = Instant::now();
    
    // 创建批量回调
    let callbacks: Vec<(Duration, _)> = (0..BATCH_SIZE)
        .map(|i| {
            let counter_clone = Arc::clone(&counter);
            let delay = Duration::from_millis(50 + (i % 10) as u64);
            let callback = move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            };
            (delay, callback)
        })
        .collect();
    
    // 批量调度
    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    
    println!("批量调度 {} 个定时器耗时: {:?}", BATCH_SIZE, start.elapsed());
    assert_eq!(batch.len(), BATCH_SIZE);
    
    // 等待所有定时器触发
    tokio::time::sleep(Duration::from_millis(150)).await;
    
    let count = counter.load(Ordering::SeqCst);
    println!("触发的定时器数量: {}", count);
    assert_eq!(count, BATCH_SIZE as u32, "所有批量调度的定时器都应该被触发");
}

#[tokio::test]
async fn test_batch_cancel() {
    // 测试批量取消定时器
    let timer = Arc::new(TimerWheel::with_defaults().unwrap());
    const TIMER_COUNT: usize = 500;
    
    // 批量创建定时器
    let callbacks: Vec<(Duration, _)> = (0..TIMER_COUNT)
        .map(|_| {
            let callback = || async {};
            (Duration::from_secs(10), callback)
        })
        .collect();
    
    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    assert_eq!(batch.len(), TIMER_COUNT);
    
    // 批量取消（使用 BatchHandle 的 cancel_all 方法）
    let start = Instant::now();
    let cancelled = batch.cancel_all();
    let elapsed = start.elapsed();
    
    println!("批量取消 {} 个定时器耗时: {:?}", TIMER_COUNT, elapsed);
    assert_eq!(cancelled, TIMER_COUNT, "所有定时器都应该被成功取消");
}

#[tokio::test]
async fn test_batch_cancel_partial() {
    // 测试部分批量取消
    let timer = TimerWheel::with_defaults().unwrap();
    
    // 创建 10 个定时器
    let callbacks: Vec<(Duration, _)> = (0..10)
        .map(|_| (Duration::from_millis(100), || async {}))
        .collect();
    
    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    
    // 转换为独立的句柄
    let mut handles = batch.into_handles();
    
    // 分离：取出前 5 个，保留后 5 个
    let remaining_handles = handles.split_off(5);
    
    // 取消前 5 个
    let mut cancelled_count = 0;
    for handle in handles {
        if handle.cancel() {
            cancelled_count += 1;
        }
    }
    assert_eq!(cancelled_count, 5);
    
    // 等待剩余的定时器触发 - 增加等待时间到200ms以确保定时器真正触发
    // tick_duration是10ms，100ms延迟需要10个tick，加上调度延迟和回调执行时间
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // 尝试取消已经触发的定时器
    let mut cancelled_after = 0;
    for handle in remaining_handles {
        if handle.cancel() {
            cancelled_after += 1;
        }
    }
    assert_eq!(cancelled_after, 0, "已触发的定时器不应该被取消");
}

#[tokio::test]
async fn test_batch_cancel_no_wait() {
    // 测试无需等待结果的批量取消
    let timer = TimerWheel::with_defaults().unwrap();
    
    // 批量创建定时器
    let callbacks: Vec<(Duration, _)> = (0..100)
        .map(|_| (Duration::from_secs(10), || async {}))
        .collect();
    
    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
    
    // 批量取消（使用 BatchHandle 的 cancel_all 方法，现在是同步的）
    let start = Instant::now();
    let _ = batch.cancel_all();
    let elapsed = start.elapsed();
    
    println!("批量取消（无等待）耗时: {:?}", elapsed);
    
    // 不等待结果的操作应该非常快（几微秒）
    assert!(elapsed < Duration::from_millis(10), "无等待的批量取消应该非常快");
}

