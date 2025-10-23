use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use timer::TimerWheel;

#[tokio::test]
async fn test_large_scale_timers() {
    // 测试大规模并发定时器（10000+ 个）
    let timer = TimerWheel::with_defaults();
    let counter = Arc::new(AtomicU32::new(0));
    const TIMER_COUNT: u32 = 10_000;

    let start = Instant::now();

    // 创建 10000 个定时器
    for i in 0..TIMER_COUNT {
        let counter_clone = Arc::clone(&counter);
        let delay = Duration::from_millis(10 + (i % 100) as u64);
        
        timer.schedule_once(
            delay,
            move || {
                let counter = Arc::clone(&counter_clone);
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
    }

    println!("创建 {} 个定时器耗时: {:?}", TIMER_COUNT, start.elapsed());
    println!("当前活跃定时器数量: {}", timer.task_count());

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
    timer.schedule_once(
        Duration::from_millis(100),
        move || {
            let end_time = Arc::clone(&end_clone);
            async move {
                *end_time.lock() = Some(Instant::now());
            }
        },
    );

    tokio::time::sleep(Duration::from_millis(150)).await;

    let start = start_time.lock().unwrap();
    let end = end_time.lock().unwrap();
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
    let mut handles = Vec::new();

    // 创建多个任务并发地添加定时器
    for _ in 0..5 {
        let timer_clone = Arc::clone(&timer);
        let counter_clone = Arc::clone(&counter);
        
        let handle = tokio::spawn(async move {
            for _ in 0..1000 {
                let counter = Arc::clone(&counter_clone);
                timer_clone.schedule_once(
                    Duration::from_millis(50),
                    move || {
                        let counter = Arc::clone(&counter);
                        async move {
                            counter.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                );
            }
        });
        
        handles.push(handle);
    }

    // 等待所有任务完成
    for handle in handles {
        handle.await.unwrap();
    }

    println!("并发创建后的活跃定时器数量: {}", timer.task_count());

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
    
    for (idx, &delay_ms) in delays.iter().enumerate() {
        let results_clone = Arc::clone(&results);
        
        timer.schedule_once(
            Duration::from_millis(delay_ms),
            move || {
                let results = Arc::clone(&results_clone);
                async move {
                    results.lock().push((idx, delay_ms));
                }
            },
        );
    }

    // 等待所有定时器触发（等待时间需要大于最大延迟）
    tokio::time::sleep(Duration::from_millis(300)).await;

    let final_results = results.lock();
    println!("触发顺序: {:?}", final_results);
    assert_eq!(final_results.len(), delays.len(), "所有定时器都应该被触发");
}

#[tokio::test]
async fn test_memory_efficiency() {
    // 测试内存效率 - 创建大量定时器然后取消
    let timer = TimerWheel::with_defaults();
    let mut handles = Vec::new();

    // 创建 5000 个定时器
    for _ in 0..5000 {
        let handle = timer.schedule_once(
            Duration::from_secs(10),
            || async {},
        );
        handles.push(handle);
    }

    println!("创建后的活跃定时器数量: {}", timer.task_count());
    assert_eq!(timer.task_count(), 5000);

    // 取消所有定时器
    let mut cancelled_count = 0;
    for handle in handles {
        if handle.cancel() {
            cancelled_count += 1;
        }
    }

    println!("取消的定时器数量: {}", cancelled_count);
    println!("取消后的活跃定时器数量: {}", timer.task_count());
    
    assert_eq!(cancelled_count, 5000);
    assert_eq!(timer.task_count(), 0, "所有定时器都应该被取消");
}

#[tokio::test]
async fn test_repeat_timer() {
    // 测试周期性定时器
    let timer = TimerWheel::with_defaults();
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);

    // 创建一个每 50ms 触发一次的周期性定时器
    let handle = timer.schedule_repeat(
        Duration::from_millis(50),
        move || {
            let counter = Arc::clone(&counter_clone);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        },
    );

    // 等待足够时间让定时器触发多次
    tokio::time::sleep(Duration::from_millis(250)).await;

    let count = counter.load(Ordering::SeqCst);
    println!("周期性定时器触发次数: {}", count);
    
    // 250ms 内应该触发大约 4-5 次（考虑到调度延迟）
    assert!(count >= 3 && count <= 6, "周期性定时器应该触发多次，实际: {}", count);

    // 取消定时器
    assert!(handle.cancel());
    
    // 等待一段时间，确保定时器不再触发
    let count_before = counter.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let count_after = counter.load(Ordering::SeqCst);
    
    assert_eq!(count_before, count_after, "取消后定时器不应该再触发");
}

