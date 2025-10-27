use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
use futures::future;

#[tokio::test]
async fn test_large_scale_timers() {
    // 测试大规模并发定时器（10000+ 个）
    let timer = Arc::new(TimerWheel::with_defaults());
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
            let task = TimerWheel::create_task(
                delay,
                Some(CallbackWrapper::new(move || {
                    let counter = Arc::clone(&counter_clone);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })),
            );
            timer_clone.register(task)
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
    let timer = TimerWheel::with_defaults();
    let start_time = Arc::new(parking_lot::Mutex::new(None::<Instant>));
    let end_time = Arc::new(parking_lot::Mutex::new(None::<Instant>));

    *start_time.lock() = Some(Instant::now());

    let end_clone = Arc::clone(&end_time);
    let task = TimerWheel::create_task(
        Duration::from_millis(100),
        Some(CallbackWrapper::new(move || {
            let end_time = Arc::clone(&end_clone);
            async move {
                *end_time.lock() = Some(Instant::now());
            }
        })),
    );
    let handle = timer.register(task);

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
    let timer = Arc::new(TimerWheel::with_defaults());
    let counter = Arc::new(AtomicU32::new(0));

    // 并发创建所有定时器（5个任务 × 1000个定时器 = 5000个）
    let mut all_futures = Vec::new();
    
    for _ in 0..5 {
        for _ in 0..1000 {
            let timer_clone = Arc::clone(&timer);
            let counter_clone = Arc::clone(&counter);
            
            let future = async move {
                let task = TimerWheel::create_task(
                    Duration::from_millis(50),
                    Some(CallbackWrapper::new(move || {
                        let counter = Arc::clone(&counter_clone);
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                        }
                    })),
                );
                timer_clone.register(task)
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
    let timer = TimerWheel::with_defaults();
    let results = Arc::new(parking_lot::Mutex::new(Vec::new()));

    let delays = vec![10, 20, 30, 50, 100, 150, 200];
    let mut handles = Vec::new();
    
    for (idx, &delay_ms) in delays.iter().enumerate() {
        let results_clone = Arc::clone(&results);
        
        let task = TimerWheel::create_task(
            Duration::from_millis(delay_ms),
            Some(CallbackWrapper::new(move || {
                let results = Arc::clone(&results_clone);
                async move {
                    results.lock().push((idx, delay_ms));
                }
            })),
        );
        let handle = timer.register(task);
        
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
    let timer = Arc::new(TimerWheel::with_defaults());

    // 并发创建 5000 个定时器
    let mut create_futures = Vec::new();
    for _ in 0..5000 {
        let timer_clone = Arc::clone(&timer);
        let future = async move {
            let task = TimerWheel::create_task(
                Duration::from_secs(10),
                None,
            );
            timer_clone.register(task)
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
    let timer = TimerWheel::with_defaults();
    let counter = Arc::new(AtomicU32::new(0));
    
    const BATCH_SIZE: usize = 100;
    let start = Instant::now();
    
    // 创建批量回调
    let callbacks: Vec<(Duration, _)> = (0..BATCH_SIZE)
        .map(|i| {
            let counter_clone = Arc::clone(&counter);
            let delay = Duration::from_millis(50 + (i % 10) as u64);
            let callback = Some(CallbackWrapper::new(move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
                }));
            (delay, callback)
        })
        .collect();
    
    // 批量调度
    let tasks = TimerWheel::create_batch_with_callbacks(callbacks);
    let batch = timer.register_batch(tasks);
    
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
    let timer = Arc::new(TimerWheel::with_defaults());
    const TIMER_COUNT: usize = 500;
    
    // 批量创建定时器
    let delays: Vec<Duration> = (0..TIMER_COUNT)
        .map(|_| Duration::from_secs(10))
        .collect();
    
    let tasks = TimerWheel::create_batch(delays);
    let batch = timer.register_batch(tasks);
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
    let timer = TimerWheel::with_defaults();
    
    // 创建 10 个定时器
    let delays: Vec<Duration> = (0..10)
        .map(|_| Duration::from_millis(100))
        .collect();
    
    let tasks = TimerWheel::create_batch(delays);
    let batch = timer.register_batch(tasks);
    
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
    let timer = TimerWheel::with_defaults();
    
    // 批量创建定时器
    let delays: Vec<Duration> = (0..100)
        .map(|_| Duration::from_secs(10))
        .collect();
    
    let tasks = TimerWheel::create_batch(delays);
    let batch = timer.register_batch(tasks);
    
    // 批量取消（使用 BatchHandle 的 cancel_all 方法，现在是同步的）
    let start = Instant::now();
    let _ = batch.cancel_all();
    let elapsed = start.elapsed();
    
    println!("批量取消（无等待）耗时: {:?}", elapsed);
    
    // 不等待结果的操作应该非常快（几微秒）
    assert!(elapsed < Duration::from_millis(10), "无等待的批量取消应该非常快");
}

#[tokio::test]
async fn test_postpone_single_timer() {
    // 测试单个定时器的推迟功能
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
    assert!(postponed, "任务应该成功推迟");

    // 等待原定时间 50ms，任务不应该触发
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "任务不应在原定时间触发");

    // 等待新的触发时间
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        handle.into_completion_receiver().0
    ).await;
    assert!(result.is_ok(), "任务应该在推迟后的时间触发");
    
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "任务应该被执行一次");
}

#[tokio::test]
async fn test_postpone_with_new_callback() {
    // 测试推迟并替换回调函数
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
    assert!(postponed, "任务应该成功推迟并替换回调");

    // 等待任务触发
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        handle.into_completion_receiver().0
    ).await;
    assert!(result.is_ok(), "任务应该触发");
    
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 10, "新回调应该被执行（增加10而不是1）");
}

#[tokio::test]
async fn test_batch_postpone() {
    // 测试批量推迟定时器
    let timer = TimerWheel::with_defaults();
    let counter = Arc::new(AtomicU32::new(0));
    const BATCH_SIZE: usize = 100;

    // 创建批量任务
    let mut task_ids = Vec::new();
    for _ in 0..BATCH_SIZE {
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

    let start = Instant::now();
    let postponed = timer.postpone_batch(task_ids);
    let elapsed = start.elapsed();
    
    println!("批量推迟 {} 个定时器耗时: {:?}", BATCH_SIZE, elapsed);
    assert_eq!(postponed, BATCH_SIZE, "所有任务都应该成功推迟");

    // 等待原定时间 50ms，任务不应该触发
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "任务不应在原定时间触发");

    // 等待新的触发时间
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(counter.load(Ordering::SeqCst), BATCH_SIZE as u32, "所有任务都应该被执行");
}

#[tokio::test]
async fn test_postpone_batch_with_callbacks() {
    // 测试批量推迟并替换回调
    let timer = TimerWheel::with_defaults();
    let counter = Arc::new(AtomicU32::new(0));
    const BATCH_SIZE: usize = 50;

    // 创建批量任务（初始回调为空）
    let mut task_ids = Vec::new();
    for _ in 0..BATCH_SIZE {
        let task = TimerWheel::create_task(
            Duration::from_millis(50),
            None,
        );
        task_ids.push(task.get_id());
        timer.register(task);
    }

    // 批量推迟并替换回调
    let updates: Vec<_> = task_ids
        .into_iter()
        .map(|id| {
            let counter = Arc::clone(&counter);
                (id, Duration::from_millis(150), Some(CallbackWrapper::new(move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                })))
        })
        .collect();

    let start = Instant::now();
    let postponed = timer.postpone_batch_with_callbacks(updates);
    let elapsed = start.elapsed();
    
    println!("批量推迟并替换回调 {} 个定时器耗时: {:?}", BATCH_SIZE, elapsed);
    assert_eq!(postponed, BATCH_SIZE, "所有任务都应该成功推迟");

    // 等待原定时间 50ms，任务不应该触发
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "任务不应在原定时间触发");

    // 等待新的触发时间
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(counter.load(Ordering::SeqCst), BATCH_SIZE as u32, "所有新回调都应该被执行");
}

#[tokio::test]
async fn test_postpone_multiple_times() {
    // 测试多次推迟同一个定时器
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

    // 第一次推迟到 100ms
    assert!(timer.postpone(task_id, Duration::from_millis(100), None));
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "第一次推迟后不应触发");

    // 第二次推迟到 150ms
    assert!(timer.postpone(task_id, Duration::from_millis(150), None));
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "第二次推迟后不应触发");

    // 第三次推迟到 100ms
    assert!(timer.postpone(task_id, Duration::from_millis(100), None));
    
    // 等待最终触发
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        handle.into_completion_receiver().0
    ).await;
    assert!(result.is_ok(), "任务应该最终触发");
    
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "任务应该只执行一次");
}

#[tokio::test]
async fn test_postpone_with_service() {
    // 测试通过 TimerService 推迟定时器
    let timer = TimerWheel::with_defaults();
    let mut service = timer.create_service();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);

    let task = kestrel_protocol_timer::TimerService::create_task(
        Duration::from_millis(50),
        Some(CallbackWrapper::new(move || {
            let counter = Arc::clone(&counter_clone);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        })),
    );
    let task_id = task.get_id();
    service.register(task).unwrap();

    // 推迟任务
    let postponed = service.postpone(task_id, Duration::from_millis(150), None);
    assert!(postponed, "任务应该成功推迟");

    // 等待原定时间，任务不应该触发
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 0, "任务不应在原定时间触发");

    // 接收超时通知
    let mut rx = service.take_receiver().unwrap();
    let result = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
    assert!(result.is_ok(), "应该收到超时通知");
    
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(counter.load(Ordering::SeqCst), 1, "任务应该被执行");
}

