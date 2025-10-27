use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use std::time::Duration;
use kestrel_protocol_timer::TimerWheel;

/// 基准测试：单个任务插入（通过 TimerWheel API）
fn bench_wheel_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_insert");
    
    group.bench_function("insert_single", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer（不计入测量）
                let timer = TimerWheel::with_defaults();
                
                // 测量阶段：只测量插入操作的性能
                let start = std::time::Instant::now();
                
                let task = TimerWheel::create_task(Duration::from_millis(100), None);
                let _handle = black_box(
                    timer.register(task)
                );
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：批量任务插入
fn bench_wheel_insert_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_insert_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和 tasks（不计入测量）
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|i| (Duration::from_millis(100 + i as u64 * 10), None))
                        .collect();
                    
                    // 测量阶段：只测量批量插入的性能
                    let start = std::time::Instant::now();
                    
                    let tasks = TimerWheel::create_batch(callbacks);
                    let _batch = black_box(
                        timer.register_batch(tasks)
                    );
                    
                    total_duration += start.elapsed();
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：单个任务取消
fn bench_wheel_cancel(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_cancel");
    
    group.bench_function("cancel_single", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 和插入任务（不计入测量）
                let timer = TimerWheel::with_defaults();
                let task = TimerWheel::create_task(Duration::from_millis(100), None);
                let task_id = task.get_id();
                let _handle = timer.register(task);
                
                // 测量阶段：只测量取消操作的性能
                let start = std::time::Instant::now();
                
                let result = black_box(
                    timer.cancel(task_id)
                );
                
                total_duration += start.elapsed();
                black_box(result);
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：批量任务取消
fn bench_wheel_cancel_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_cancel_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和插入任务（不计入测量）
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|i| (Duration::from_millis(100 + i as u64 * 10), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 测量阶段：只测量批量取消的性能
                    let start = std::time::Instant::now();
                    
                    let cancelled = black_box(
                        timer.cancel_batch(&task_ids)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(cancelled);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：同槽位批量取消性能
fn bench_wheel_cancel_batch_same_slot(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_cancel_batch_same_slot");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和插入相同延迟的任务（会进入同一槽位）
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_millis(100), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 测量阶段：只测量批量取消的性能
                    let start = std::time::Instant::now();
                    
                    let cancelled = black_box(
                        timer.cancel_batch(&task_ids)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(cancelled);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：空槽位推进
/// 注意：由于 advance 是内部操作，我们通过 sleep 来触发 tick
fn bench_wheel_advance_empty(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_advance_empty");
    
    group.bench_function("advance_empty_slot", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer（不计入测量）
                let _timer = TimerWheel::with_defaults();
                
                // 测量阶段：让 tick 循环运行一次
                let start = std::time::Instant::now();
                
                // 等待一个 tick 周期（10ms）
                tokio::time::sleep(Duration::from_millis(10)).await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：有任务的槽位推进
fn bench_wheel_advance_with_tasks(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_advance_with_tasks");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 并插入任务（不计入测量）
                    let timer = TimerWheel::with_defaults();
                    
                    // 插入任务，延迟设置为 20ms（确保会在推进时到期）
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_millis(20), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let _batch = timer.register_batch(tasks);
                    
                    // 测量阶段：等待任务到期
                    let start = std::time::Instant::now();
                    
                    // 等待任务到期（20ms + 一点余量）
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    
                    total_duration += start.elapsed();
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：混合轮次的推进
fn bench_wheel_advance_mixed_rounds(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_advance_mixed_rounds");
    
    group.bench_function("advance_mixed_rounds", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 并插入不同轮次的任务
                let timer = TimerWheel::with_defaults();
                
                // 插入不同延迟的任务，产生不同的轮次
                let mut callbacks = Vec::new();
                for i in 0..100 {
                    // 延迟从 20ms 到 200ms
                    let delay = Duration::from_millis(20 + (i % 20) * 10);
                    callbacks.push((delay, None));
                }
                let tasks = TimerWheel::create_batch(callbacks);
                let _batch = timer.register_batch(tasks);
                
                // 测量阶段：推进多个 tick
                let start = std::time::Instant::now();
                
                // 等待最长延迟的任务到期
                tokio::time::sleep(Duration::from_millis(210)).await;
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：混合操作（插入、取消、推进）
fn bench_wheel_mixed_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_mixed_operations");
    
    group.bench_function("insert_cancel_advance_mixed", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer
                let timer = TimerWheel::with_defaults();
                
                // 测量阶段：执行混合操作
                let start = std::time::Instant::now();
                
                for round in 0..10 {
                    // 插入 50 个任务
                    let callbacks: Vec<_> = (0..50)
                        .map(|_| (Duration::from_millis(50 + round * 10), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 取消其中一半
                    let to_cancel: Vec<_> = task_ids.iter().step_by(2).copied().collect();
                    let _ = timer.cancel_batch(&to_cancel);
                    
                    // 等待一个 tick
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：小批量取消（≤10）- 测试优化路径
fn bench_wheel_cancel_small_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_cancel_small_batch");
    
    for size in [1, 5, 10].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和插入任务
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|i| (Duration::from_millis(100 + i as u64 * 10), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 测量阶段：只测量批量取消的性能
                    let start = std::time::Instant::now();
                    
                    let cancelled = black_box(
                        timer.cancel_batch(&task_ids)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(cancelled);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：跨多个槽位的批量操作
fn bench_wheel_batch_multiple_slots(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_batch_multiple_slots");
    
    for size in [100, 500, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 并插入跨多个槽位的任务
                    let timer = TimerWheel::with_defaults();
                    
                    // 均匀分布在不同槽位
                    let callbacks: Vec<_> = (0..size)
                        .map(|i| (Duration::from_millis(10 + i as u64), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 测量阶段：批量取消
                    let start = std::time::Instant::now();
                    
                    let cancelled = black_box(
                        timer.cancel_batch(&task_ids)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(cancelled);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：单个任务推迟
fn bench_wheel_postpone(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_postpone");
    
    group.bench_function("postpone_single", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 和插入任务（不计入测量）
                let timer = TimerWheel::with_defaults();
                let task = TimerWheel::create_task(Duration::from_millis(100), None);
                let task_id = task.get_id();
                let _handle = timer.register(task);
                
                // 测量阶段：只测量推迟操作的性能
                let start = std::time::Instant::now();
                
                let result = black_box(
                    timer.postpone(task_id, Duration::from_millis(200), None)
                );
                
                total_duration += start.elapsed();
                black_box(result);
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：批量任务推迟
fn bench_wheel_postpone_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_postpone_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和插入任务（不计入测量）
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|i| (Duration::from_millis(100 + i as u64 * 10), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 准备推迟参数
                    let postpone_updates: Vec<_> = task_ids
                        .iter()
                        .map(|&id| (id, Duration::from_millis(200)))
                        .collect();
                    
                    // 测量阶段：只测量批量推迟的性能
                    let start = std::time::Instant::now();
                    
                    let postponed = black_box(
                        timer.postpone_batch(&postpone_updates)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(postponed);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：推迟并替换回调
fn bench_wheel_postpone_with_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_postpone_with_callback");
    
    group.bench_function("postpone_single_with_callback", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 和插入任务（不计入测量）
                let timer = TimerWheel::with_defaults();
                let task = TimerWheel::create_task(Duration::from_millis(100), None);
                let task_id = task.get_id();
                let _handle = timer.register(task);
                
                // 测量阶段：只测量推迟并替换回调的性能
                let start = std::time::Instant::now();
                
                let result = black_box(
                    timer.postpone(
                        task_id,
                        Duration::from_millis(200),
                        None
                    )
                );
                
                total_duration += start.elapsed();
                black_box(result);
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：批量推迟并替换回调
fn bench_wheel_postpone_batch_with_callbacks(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_postpone_batch_with_callbacks");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和插入任务（不计入测量）
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|i| (Duration::from_millis(100 + i as u64 * 10), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 准备推迟参数（包含新回调）
                    let postpone_updates: Vec<_> = task_ids
                        .into_iter()
                        .map(|id| (id, Duration::from_millis(200), None))
                        .collect();
                    
                    // 测量阶段：只测量批量推迟并替换回调的性能
                    let start = std::time::Instant::now();
                    
                    let postponed = black_box(
                        timer.postpone_batch_with_callbacks(postpone_updates)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(postponed);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：同槽位批量推迟性能
fn bench_wheel_postpone_batch_same_slot(c: &mut Criterion) {
    let mut group = c.benchmark_group("wheel_postpone_batch_same_slot");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和插入相同延迟的任务（会进入同一槽位）
                    let timer = TimerWheel::with_defaults();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_millis(100), None))
                        .collect();
                    let tasks = TimerWheel::create_batch(callbacks);
                    let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
                    let _batch = timer.register_batch(tasks);
                    
                    // 准备推迟参数（同样推迟到同一槽位）
                    let postpone_updates: Vec<_> = task_ids
                        .iter()
                        .map(|&id| (id, Duration::from_millis(200)))
                        .collect();
                    
                    // 测量阶段：只测量批量推迟的性能
                    let start = std::time::Instant::now();
                    
                    let postponed = black_box(
                        timer.postpone_batch(&postpone_updates)
                    );
                    
                    total_duration += start.elapsed();
                    black_box(postponed);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_wheel_insert,
    bench_wheel_insert_batch,
    bench_wheel_cancel,
    bench_wheel_cancel_batch,
    bench_wheel_postpone,
    bench_wheel_postpone_batch,
    bench_wheel_cancel_batch_same_slot,
    bench_wheel_advance_empty,
    bench_wheel_advance_with_tasks,
    bench_wheel_advance_mixed_rounds,
    bench_wheel_mixed_operations,
    bench_wheel_cancel_small_batch,
    bench_wheel_batch_multiple_slots,
    bench_wheel_postpone_with_callback,
    bench_wheel_postpone_batch_with_callbacks,
    bench_wheel_postpone_batch_same_slot,
);

criterion_main!(benches);

