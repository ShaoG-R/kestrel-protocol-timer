use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use timer::TimerWheel;
use std::hint::black_box;

/// 基准测试：单个定时器调度
fn bench_schedule_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("schedule_single");
    
    group.bench_function("schedule_once", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let timer = TimerWheel::with_defaults().unwrap();
                let service = timer.create_service();
                
                let task_id = black_box(
                    service.schedule_once(
                        Duration::from_secs(10),
                        || async {}
                    ).await.unwrap()
                );
                
                black_box(task_id);
            });
    });
    
    group.finish();
}

/// 基准测试：批量定时器调度（不同规模）
fn bench_schedule_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("schedule_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| async move {
                    let timer = TimerWheel::with_defaults().unwrap();
                    let service = timer.create_service();
                    
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    
                    let task_ids = black_box(
                        service.schedule_once_batch(callbacks).await.unwrap()
                    );
                    
                    black_box(task_ids);
                });
        });
    }
    
    group.finish();
}

/// 基准测试：单个任务取消
fn bench_cancel_single(c: &mut Criterion) {
    let mut group = c.benchmark_group("cancel_single");
    
    group.bench_function("cancel_task", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let timer = TimerWheel::with_defaults().unwrap();
                let service = timer.create_service();
                
                // 先调度一个任务
                let task_id = service.schedule_once(
                    Duration::from_secs(10),
                    || async {}
                ).await.unwrap();
                
                // 测量取消操作的性能
                let result = black_box(
                    service.cancel_task(task_id).await.unwrap()
                );
                
                black_box(result);
            });
    });
    
    group.finish();
}

/// 基准测试：批量任务取消（使用优化的批量 API）
fn bench_cancel_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("cancel_batch");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| async move {
                    let timer = TimerWheel::with_defaults().unwrap();
                    let service = timer.create_service();
                    
                    // 先批量调度任务
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
                    
                    // 测量批量取消操作的性能（使用优化的批量 API）
                    let cancelled = black_box(
                        service.cancel_batch(&task_ids).await.unwrap()
                    );
                    
                    black_box(cancelled);
                });
        });
    }
    
    group.finish();
}

/// 基准测试：并发调度
fn bench_concurrent_schedule(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_schedule");
    
    for concurrent_ops in [10, 50].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(concurrent_ops), concurrent_ops, |b, &concurrent_ops| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| async move {
                    let timer = TimerWheel::with_defaults().unwrap();
                    let service = timer.create_service();
                    
                    // 并发执行多个调度操作
                    let mut handles = Vec::new();
                    for _ in 0..concurrent_ops {
                        let callbacks: Vec<_> = (0..10)
                            .map(|_| (Duration::from_secs(10), || async {}))
                            .collect();
                        
                        let fut = service.schedule_once_batch(callbacks);
                        handles.push(fut);
                    }
                    
                    // 等待所有调度完成
                    let results = futures::future::join_all(handles).await;
                    black_box(results);
                });
        });
    }
    
    group.finish();
}

/// 基准测试：高频取消（使用优化的批量 API）
fn bench_high_frequency_cancel(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_frequency_cancel");
    
    group.bench_function("cancel_1000_tasks_batch", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let timer = TimerWheel::with_defaults().unwrap();
                let service = timer.create_service();
                
                // 先调度1000个任务
                let callbacks: Vec<_> = (0..1000)
                    .map(|_| (Duration::from_secs(10), || async {}))
                    .collect();
                let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
                
                // 批量取消所有任务
                let cancelled = black_box(
                    service.cancel_batch(&task_ids).await.unwrap()
                );
                
                black_box(cancelled);
            });
    });
    
    group.finish();
}

/// 基准测试：混合操作（调度和取消，使用优化的批量 API）
fn bench_mixed_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_operations");
    
    group.bench_function("schedule_and_cancel_interleaved", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let timer = TimerWheel::with_defaults().unwrap();
                let service = timer.create_service();
                
                // 交替执行调度和取消操作
                for _ in 0..50 {
                    // 调度10个任务
                    let callbacks: Vec<_> = (0..10)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
                    
                    // 使用批量取消前5个任务
                    let to_cancel: Vec<_> = task_ids.iter().take(5).copied().collect();
                    let cancelled = service.cancel_batch(&to_cancel).await.unwrap();
                    
                    black_box(cancelled);
                }
            });
    });
    
    group.finish();
}

/// 基准测试：调度仅通知的定时器
fn bench_schedule_notify(c: &mut Criterion) {
    let mut group = c.benchmark_group("schedule_notify");
    
    group.bench_function("schedule_once_notify", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let timer = TimerWheel::with_defaults().unwrap();
                let service = timer.create_service();
                
                let task_id = black_box(
                    service.schedule_once_notify(Duration::from_secs(10)).await.unwrap()
                );
                
                black_box(task_id);
            });
    });
    
    for size in [100, 1000].iter() {
        group.bench_with_input(BenchmarkId::new("batch_notify", size), size, |b, &size| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| async move {
                    let timer = TimerWheel::with_defaults().unwrap();
                    let service = timer.create_service();
                    
                    let mut task_ids = Vec::new();
                    for _ in 0..size {
                        let task_id = service.schedule_once_notify(Duration::from_secs(10)).await.unwrap();
                        task_ids.push(task_id);
                    }
                    
                    black_box(task_ids);
                });
        });
    }
    
    group.finish();
}

/// 基准测试：带回调的定时器性能
fn bench_schedule_with_callback(c: &mut Criterion) {
    let mut group = c.benchmark_group("schedule_with_callback");
    
    group.bench_function("schedule_with_simple_callback", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let timer = TimerWheel::with_defaults().unwrap();
                let service = timer.create_service();
                let counter = Arc::new(AtomicU32::new(0));
                
                let counter_clone = Arc::clone(&counter);
                let task_id = black_box(
                    service.schedule_once(
                        Duration::from_secs(10),
                        move || {
                            let counter = Arc::clone(&counter_clone);
                            async move {
                                counter.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                    ).await.unwrap()
                );
                
                black_box(task_id);
            });
    });
    
    group.finish();
}

/// 基准测试：无等待取消
fn bench_cancel_no_wait(c: &mut Criterion) {
    let mut group = c.benchmark_group("cancel_no_wait");
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.to_async(tokio::runtime::Runtime::new().unwrap())
                .iter(|| async move {
                    let timer = TimerWheel::with_defaults().unwrap();
                    let service = timer.create_service();
                    
                    // 先批量调度任务
                    let callbacks: Vec<_> = (0..size)
                        .map(|_| (Duration::from_secs(10), || async {}))
                        .collect();
                    let task_ids = service.schedule_once_batch(callbacks).await.unwrap();
                    
                    // 测量无等待取消操作的性能
                    for task_id in task_ids {
                        service.cancel_task_no_wait(task_id).await.unwrap();
                    }
                });
        });
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_schedule_single,
    bench_schedule_batch,
    bench_cancel_single,
    bench_cancel_batch,
    bench_concurrent_schedule,
    bench_high_frequency_cancel,
    bench_mixed_operations,
    bench_schedule_notify,
    bench_schedule_with_callback,
    bench_cancel_no_wait,
);

criterion_main!(benches);

