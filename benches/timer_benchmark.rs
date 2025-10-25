use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use timer::TimerWheel;
use std::hint::black_box;

/// 基准测试：单个定时器调度
fn bench_timer_schedule_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_schedule_single");
    
    group.bench_function("schedule_once", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer（不计入测量）
                let timer = TimerWheel::with_defaults().unwrap();
                
                // 测量阶段：只测量 schedule_once 的性能
                let start = std::time::Instant::now();
                
                let handle = black_box(
                    timer.schedule_once(
                        Duration::from_secs(10),
                        || async {}
                    ).await.unwrap()
                );
                
                total_duration += start.elapsed();
                black_box(handle);
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：批量定时器调度
fn bench_timer_schedule_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_schedule_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和 callbacks（不计入测量）
                    let timer = TimerWheel::with_defaults().unwrap();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    
                    // 测量阶段：只测量 schedule_once_batch 的性能
                    let start = std::time::Instant::now();
                    
                    let batch = black_box(
                        timer.schedule_once_batch(callbacks).await.unwrap()
                    );
                    
                    total_duration += start.elapsed();
                    black_box(batch);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：单个任务取消（通过 TimerWheel.cancel）
fn bench_timer_cancel_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_cancel_single");
    
    group.bench_function("cancel_task", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 和调度任务（不计入测量）
                let timer = TimerWheel::with_defaults().unwrap();
                
                let handle = timer.schedule_once(
                    Duration::from_secs(10),
                    || async {}
                ).await.unwrap();
                
                let task_id = handle.task_id();
                
                // 测量阶段：只测量 cancel 的性能
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

/// 基准测试：批量任务取消（通过 TimerWheel.cancel_batch）
fn bench_timer_cancel_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_cancel_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和调度任务（不计入测量）
                    let timer = TimerWheel::with_defaults().unwrap();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
                    let task_ids: Vec<_> = batch.task_ids().to_vec();
                    
                    // 测量阶段：只测量 cancel_batch 的性能
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

/// 基准测试：TimerHandle.cancel()
fn bench_handle_cancel(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle_cancel");
    
    group.bench_function("cancel", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 和调度任务（不计入测量）
                let timer = TimerWheel::with_defaults().unwrap();
                
                let handle = timer.schedule_once(
                    Duration::from_secs(10),
                    || async {}
                ).await.unwrap();
                
                // 测量阶段：只测量 handle.cancel 的性能
                let start = std::time::Instant::now();
                
                let result = black_box(
                    handle.cancel()
                );
                
                total_duration += start.elapsed();
                black_box(result);
            }
            
            total_duration
        });
    });
    
    group.finish();
}


/// 基准测试：BatchHandle.cancel_all()
fn bench_batch_handle_cancel_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_handle_cancel_all");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer 和调度任务（不计入测量）
                    let timer = TimerWheel::with_defaults().unwrap();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
                    
                    // 测量阶段：只测量 batch.cancel_all 的性能
                    let start = std::time::Instant::now();
                    
                    let cancelled = black_box(
                        batch.cancel_all()
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


/// 基准测试：调度仅通知的定时器
fn bench_timer_schedule_notify(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_schedule_notify");
    
    group.bench_function("schedule_once_notify", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer（不计入测量）
                let timer = TimerWheel::with_defaults().unwrap();
                
                // 测量阶段：只测量 schedule_once_notify 的性能
                let start = std::time::Instant::now();
                
                let handle = black_box(
                    timer.schedule_once_notify(Duration::from_secs(10)).await.unwrap()
                );
                
                total_duration += start.elapsed();
                black_box(handle);
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：批量调度仅通知的定时器
fn bench_timer_schedule_notify_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_schedule_notify_batch");
    
    for size in [100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer（不计入测量）
                    let timer = TimerWheel::with_defaults().unwrap();
                    
                    // 测量阶段：测量批量通知调度的性能
                    let start = std::time::Instant::now();
                    
                    let mut handles = Vec::new();
                    for _ in 0..size {
                        let handle = timer.schedule_once_notify(Duration::from_secs(10)).await.unwrap();
                        handles.push(handle);
                    }
                    
                    total_duration += start.elapsed();
                    black_box(handles);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：并发调度
fn bench_timer_concurrent_schedule(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_concurrent_schedule");
    
    for concurrent_ops in [10, 50].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(concurrent_ops), concurrent_ops, |b, &concurrent_ops| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            
            b.to_async(&runtime).iter_custom(|iters| async move {
                let mut total_duration = Duration::from_secs(0);
                
                for _ in 0..iters {
                    // 准备阶段：创建 timer（不计入测量）
                    let timer = TimerWheel::with_defaults().unwrap();
                    
                    // 测量阶段：只测量并发调度的性能
                    let start = std::time::Instant::now();
                    
                    // 并发执行多个调度操作
                    let mut handles = Vec::new();
                    for _ in 0..concurrent_ops {
                        let callbacks: Vec<_> = (0..10)
                            .map(|_| (Duration::from_secs(10), || async {}))
                            .collect();
                        
                        let fut = timer.schedule_once_batch(callbacks);
                        handles.push(fut);
                    }
                    
                    // 等待所有调度完成
                    let results = futures::future::join_all(handles).await;
                    
                    total_duration += start.elapsed();
                    black_box(results);
                }
                
                total_duration
            });
        });
    }
    
    group.finish();
}

/// 基准测试：混合操作（调度和取消）
fn bench_timer_mixed_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_mixed_operations");
    
    group.bench_function("schedule_and_cancel_interleaved", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer（不计入测量）
                let timer = TimerWheel::with_defaults().unwrap();
                
                // 测量阶段：测量混合操作的性能
                let start = std::time::Instant::now();
                
                // 交替执行调度和取消操作
                for _ in 0..50 {
                    // 调度10个任务
                    let callbacks: Vec<_> = (0..10)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    let batch = timer.schedule_once_batch(callbacks).await.unwrap();
                    
                    // 取消前5个任务
                    let to_cancel: Vec<_> = batch.task_ids().iter().take(5).copied().collect();
                    let cancelled = timer.cancel_batch(&to_cancel);
                    
                    black_box(cancelled);
                }
                
                total_duration += start.elapsed();
            }
            
            total_duration
        });
    });
    
    group.finish();
}

/// 基准测试：带回调的定时器性能
fn bench_timer_schedule_with_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("timer_schedule_with_callback");
    
    group.bench_function("schedule_with_simple_callback", |b| {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = Duration::from_secs(0);
            
            for _ in 0..iters {
                // 准备阶段：创建 timer 和 counter（不计入测量）
                let timer = TimerWheel::with_defaults().unwrap();
                let counter = Arc::new(AtomicU32::new(0));
                
                // 测量阶段：只测量 schedule_once 的性能
                let start = std::time::Instant::now();
                
                let counter_clone = Arc::clone(&counter);
                let handle = black_box(
                    timer.schedule_once(
                        Duration::from_secs(10),
                        move || {
                            let counter = Arc::clone(&counter_clone);
                            async move {
                                counter.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                    ).await.unwrap()
                );
                
                total_duration += start.elapsed();
                black_box(handle);
            }
            
            total_duration
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_timer_schedule_single,
    bench_timer_schedule_batch,
    bench_timer_cancel_single,
    bench_timer_cancel_batch,
    bench_handle_cancel,
    bench_batch_handle_cancel_all,
    bench_timer_schedule_notify,
    bench_timer_schedule_notify_batch,
    bench_timer_concurrent_schedule,
    bench_timer_mixed_operations,
    bench_timer_schedule_with_callback,
);

criterion_main!(benches);

