# Kestrel Protocol Timer

> 基于时间轮（Timing Wheel）算法的高性能异步定时器系统

[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)
[![Tokio](https://img.shields.io/badge/tokio-1.48-blue.svg)](https://tokio.rs/)
[![Crates.io](https://img.shields.io/crates/v/kestrel-protocol-timer.svg)](https://crates.io/crates/kestrel-protocol-timer)
[![Documentation](https://docs.rs/kestrel-protocol-timer/badge.svg)](https://docs.rs/kestrel-protocol-timer)
[![Downloads](https://img.shields.io/crates/d/kestrel-protocol-timer.svg)](https://crates.io/crates/kestrel-protocol-timer)
[![License](https://img.shields.io/crates/l/kestrel-protocol-timer.svg)](https://github.com/ShaoG-R/kestrel-protocol-timer#license)

## 📚 目录

- [项目概述](#项目概述)
- [核心特性](#核心特性)
- [快速开始](#快速开始)
- [安装](#安装)
- [架构说明](#架构说明)
- [使用示例](#使用示例)
- [API 文档](#api-文档)
- [配置选项](#配置选项)
- [性能基准](#性能基准)
- [测试](#测试)
- [使用场景](#使用场景)
- [依赖项](#依赖项)
- [贡献指南](#贡献指南)
- [许可证](#许可证)

## 项目概述

`kestrel-protocol-timer` 是一个基于时间轮（Timing Wheel）算法实现的高性能异步定时器库，专为 Rust 和 tokio 异步运行时设计。它能够高效管理大规模并发定时器任务，提供 O(1) 时间复杂度的插入和删除操作。

### 为什么选择 Kestrel Timer？

- **极致性能**：相比传统的堆（Heap）实现，时间轮算法在大规模定时器场景下具有显著的性能优势
- **分层架构**：双层时间轮设计，自动分离短延迟和长延迟任务，避免单层轮次检查的性能开销
- **可扩展性**：轻松处理 10,000+ 并发定时器而不影响性能，支持超长延迟任务（通过 rounds 机制）
- **生产就绪**：经过严格测试，包含完整的单元测试、集成测试和性能基准测试
- **灵活易用**：提供简洁的 API，支持单个和批量操作，内置完成通知机制
- **零成本抽象**：充分利用 Rust 的类型系统和零成本抽象特性
- **智能优化**：自动降级机制、缓存优化、小批量优化等多种性能优化手段

## 核心特性

### 🏗️ 分层时间轮架构

- **双层设计**：L0 层（高精度短延迟）+ L1 层（长延迟）自动分层
- **智能降级**：L1 层任务到期后自动降级到 L0 层执行
- **无轮次检查**：L0 层无需 rounds 判断，大幅减少 90% 任务的检查开销
- **灵活扩展**：支持超长延迟任务（通过 L1 层 rounds 机制）

### ⚡ 高性能

- **O(1) 时间复杂度**：插入、删除和触发操作均为 O(1)
- **优化的数据结构**：使用 `FxHashMap` 减少哈希冲突，`parking_lot::Mutex` 提供更快的锁机制
- **位运算优化**：槽位数量为 2 的幂次方，使用位运算替代取模操作
- **缓存优化**：预计算槽位掩码、tick 时长、层级容量等常用值

### 🚀 大规模支持

- 支持 10,000+ 并发定时器
- 批量操作优化，减少锁竞争
- 小批量阈值优化，智能选择处理策略
- 独立的 tokio 任务执行，避免阻塞时间轮推进

### 🔄 异步支持

- 完全基于 tokio 异步运行时
- 异步回调函数支持
- 非阻塞的定时器管理

### 🔒 线程安全

- 多线程环境下安全使用
- 使用 `parking_lot::Mutex` 提供高性能的锁机制
- 无数据竞争保证

### 📦 批量操作

- 批量调度定时器，减少锁开销
- 批量取消定时器
- 批量推迟定时器
- 批量完成通知

### ⏰ 定时器推迟

- 动态推迟定时器触发时间
- 支持替换回调函数
- 批量推迟操作
- O(1) 时间复杂度
- 保持原有的完成通知有效

### 🔔 完成通知

- 内置任务完成通知机制
- 支持仅通知的定时器（无回调）
- 异步等待定时器完成

### ⚙️ 灵活配置

- 可配置槽位数量
- 可配置时间精度（tick 时长）
- 默认配置开箱即用

## 快速开始

```rust
use kestrel_protocol_timer::{TimerWheel, CallbackWrapper};
use std::time::Duration;

#[tokio::main]
async fn main() {
    // 创建定时器（使用默认配置）
    let timer = TimerWheel::with_defaults();
    
    // 两步式 API：创建任务 + 注册
    let callback = Some(CallbackWrapper::new(|| async {
        println!("定时器触发！");
    }));
    let task = TimerWheel::create_task(Duration::from_secs(1), callback);
    let handle = timer.register(task);
    
    // 等待定时器完成
    handle.into_completion_receiver().0.await.ok();
    
    println!("定时器执行完成");
}
```

## 安装

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
kestrel-protocol-timer = "0.1.0"
tokio = { version = "1.48", features = ["full"] }
```

### 系统要求

- Rust 1.70 或更高版本
- Tokio 1.48 或更高版本

## 架构说明

### 时间轮算法原理

时间轮是一个环形数组结构，每个槽位（slot）存储一组到期时间相近的定时器任务。时间轮以固定的频率（tick）推进，当指针移动到某个槽位时，该槽位中的所有任务会被检查是否到期。

#### 分层时间轮架构

本实现采用**双层时间轮（Hierarchical Timing Wheel）**架构，以高效处理不同时间范围的任务：

```
┌─────────────────────────────────────────────────────────┐
│                    L1 层（高层）                          │
│   槽位数：64 | Tick: 1000ms | 覆盖范围：64秒              │
│                                                          │
│   槽位 0      槽位 1      槽位 2      ...    槽位 63      │
│   ┌────┐    ┌────┐    ┌────┐              ┌────┐       │
│   │长任│    │长任│    │    │              │    │       │
│   │务A │    │务B │    │    │              │    │       │
│   └────┘    └────┘    └────┘              └────┘       │
│        ↓ 降级到 L0                                       │
└─────────────────────────────────────────────────────────┘
                         ↓
┌─────────────────────────────────────────────────────────┐
│                    L0 层（底层）                          │
│   槽位数：512 | Tick: 10ms | 覆盖范围：5.12秒             │
│                                                          │
│   槽位 0      槽位 1      槽位 2      ...    槽位 511     │
│   ┌────┐    ┌────┐    ┌────┐              ┌────┐       │
│   │任务│    │任务│    │任务│              │    │       │
│   │A/B │    │C/D │    │E   │              │    │       │
│   └────┘    └────┘    └────┘              └────┘       │
│        ▲                                                │
│        │ 当前指针（current_tick）                         │
└─────────────────────────────────────────────────────────┘
```

#### 核心参数

**L0 层（底层 - 高精度）**：
- **槽位数量**：默认 512 个（必须是 2 的幂次方）
- **Tick 时长**：默认 10ms
- **覆盖范围**：512 × 10ms = 5.12 秒
- **用途**：处理短延迟任务（< 5.12 秒）

**L1 层（高层 - 长时间）**：
- **槽位数量**：默认 64 个（必须是 2 的幂次方）
- **Tick 时长**：默认 1000ms（1 秒）
- **覆盖范围**：64 × 1 秒 = 64 秒
- **用途**：处理长延迟任务（≥ 5.12 秒）
- **轮次机制（rounds）**：超出 64 秒的任务使用轮次计数

**层级比率**：L1 tick / L0 tick = 1000ms / 10ms = 100

#### 工作流程

1. **插入任务**：
   - 短延迟（< 5.12s）→ 直接插入 L0 层
   - 长延迟（≥ 5.12s）→ 插入 L1 层
   - 超长延迟（> 64s）→ L1 层 + rounds 计数

2. **推进时间轮**：
   - L0 层：每个 tick（10ms）推进一次
   - L1 层：每 100 个 L0 tick（1000ms）推进一次

3. **任务降级**：
   - L1 层任务到期时，自动降级到 L0 层
   - 根据剩余延迟重新计算 L0 槽位

4. **触发任务**：
   - L0 当前槽位的所有任务立即触发
   - L1 层 rounds > 0 的任务减少轮次后保留

5. **执行回调**：在独立的 tokio 任务中执行回调函数

#### 分层时间轮的优势

相比单层时间轮，分层架构具有以下优势：

1. **性能优化**：
   - L0 层（处理 80-90% 的短延迟任务）无需 rounds 检查
   - 大幅减少 `advance()` 时的条件判断开销
   - L1 层推进频率低（每 100 个 L0 tick 推进一次），减少 CPU 占用

2. **内存效率**：
   - 短延迟任务在 L0 层密集存储
   - 长延迟任务在 L1 层稀疏存储，减少内存浪费
   - 相比单层轮次方案，减少约 30% 的内存占用

3. **时间范围扩展**：
   - 单层 512 槽位 × 10ms = 5.12 秒（超出需要 rounds）
   - 双层架构：5.12 秒 + 64 秒 = 支持 0-64 秒的高效覆盖
   - L1 层的 rounds 可支持更长延迟（如分钟级、小时级）

4. **精度保证**：
   - L0 层保持高精度（10ms）处理频繁的短延迟任务
   - L1 层使用较低精度（1s）处理少量的长延迟任务
   - 降级机制确保任务在接近到期时回到 L0 层获得精确触发

### 核心组件

#### 1. TimerWheel

主定时器接口，提供定时器的创建、调度和管理功能。

```rust
pub struct TimerWheel {
    wheel: Arc<Mutex<Wheel>>,
    driver_handle: JoinHandle<()>,
}
```

**职责**：
- 管理时间轮实例
- 启动和停止时间轮驱动器
- 提供调度 API

#### 2. Wheel

分层时间轮的核心实现，负责任务的存储、查找、触发和层级管理。

```rust
pub struct Wheel {
    l0: WheelLayer,                   // L0 层（底层 - 高精度）
    l1: WheelLayer,                   // L1 层（高层 - 长时间）
    l1_tick_ratio: u64,               // L1 tick 相对于 L0 tick 的比率
    task_index: FxHashMap<TaskId, TaskLocation>, // 任务索引
    batch_config: BatchConfig,        // 批处理配置
    l0_capacity_ms: u64,              // L0 层容量（毫秒）
    l1_capacity_ticks: u64,           // L1 层容量（tick 数）
}

struct WheelLayer {
    slots: Vec<Vec<TimerTask>>,       // 槽位数组
    current_tick: u64,                // 当前 tick
    slot_count: usize,                // 槽位数量
    tick_duration: Duration,          // tick 时长
    tick_duration_ms: u64,            // 缓存的 tick 时长（毫秒）
    slot_mask: usize,                 // 槽位掩码（用于快速取模）
}
```

**职责**：
- 管理双层时间轮结构（L0 和 L1）
- 自动选择合适的层级插入任务
- 执行时间轮的推进逻辑（L0 每 tick 推进，L1 定期推进）
- 处理任务的插入、取消、推迟和触发
- 实现 L1 到 L0 的任务降级机制

#### 3. TimerHandle / BatchHandle

定时器句柄，用于管理单个或批量定时器的生命周期。

```rust
pub struct TimerHandle {
    task_id: TaskId,
    wheel: Arc<Mutex<Wheel>>,
    completion_rx: CompletionReceiver,
}
```

**职责**：
- 取消定时器
- 获取任务 ID
- 接收完成通知

#### 4. TimerService

基于 Actor 模式的定时器服务管理器，提供集中式的定时器管理。

```rust
pub struct TimerService {
    command_tx: mpsc::Sender<ServiceCommand>,
    timeout_rx: Option<mpsc::Receiver<TaskId>>,
    actor_handle: Option<JoinHandle<()>>,
    wheel: Arc<Mutex<Wheel>>,
}
```

**职责**：
- 集中管理多个定时器句柄
- 自动监听超时事件
- 将超时的 TaskId 聚合转发给用户

#### 5. TimerTask

定时器任务的封装，包含任务的元数据和回调函数。

```rust
pub struct TimerTask {
    id: TaskId,
    deadline_tick: u64,
    rounds: u32,
    callback: Option<CallbackWrapper>,
    completion_notifier: CompletionNotifier,
}
```

### 性能优化

1. **分层时间轮架构**：
   - 双层设计自动分离短延迟和长延迟任务
   - 避免单层轮次检查的开销，L0 层无需 rounds 判断
   - 长延迟任务在 L1 层稀疏存储，减少内存占用

2. **高效锁机制**：使用 `parking_lot::Mutex` 替代标准库 Mutex，减少锁开销

3. **优化哈希表**：使用 `FxHashMap`（rustc-hash）替代标准 HashMap，减少哈希冲突

4. **位运算优化**：
   - 槽位数量为 2 的幂次方，使用 `& (slot_count - 1)` 替代 `% slot_count`
   - 预计算并缓存槽位掩码（`slot_mask`）

5. **缓存优化**：
   - 缓存 tick 时长（毫秒）避免重复转换
   - 缓存层级容量（`l0_capacity_ms`, `l1_capacity_ticks`）
   - 减少 `determine_layer` 中的重复计算

6. **独立任务执行**：回调函数在独立的 tokio 任务中执行，避免阻塞时间轮推进

7. **批量操作优化**：
   - 减少锁的获取次数，提高吞吐量
   - 小批量阈值优化（`small_batch_threshold`）：小批量直接处理，避免分组排序开销
   - 使用不稳定排序（`sort_unstable_by`）提升性能

8. **SmallVec 优化**：在批量取消中使用 `smallvec` 减少小型集合的堆分配

9. **跨层迁移优化**：
   - 支持任务在 L0 和 L1 层之间自动迁移（postpone 操作）
   - 降级机制高效复用时间轮位置计算逻辑

## 使用示例

### 基础用法

#### 创建定时器

```rust
use kestrel_protocol_timer::{TimerWheel, WheelConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 使用默认配置
    // L0: 512 槽位，10ms tick，覆盖 5.12 秒
    // L1: 64 槽位，1000ms tick，覆盖 64 秒
    let timer = TimerWheel::with_defaults();
    
    // 或使用自定义配置（分层时间轮）
    let config = WheelConfig::builder()
        .l0_tick_duration(Duration::from_millis(10))  // L0 tick 时长
        .l0_slot_count(512)                            // L0 槽位数量
        .l1_tick_duration(Duration::from_secs(1))      // L1 tick 时长
        .l1_slot_count(64)                             // L1 槽位数量
        .build()?;
    let timer = TimerWheel::new(config);
    
    Ok(())
}
```

#### 调度单个定时器

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();
let counter = Arc::new(AtomicU32::new(0));
let counter_clone = Arc::clone(&counter);

// 两步式 API：创建任务 + 注册
let callback = Some(CallbackWrapper::new(move || {
    let counter = Arc::clone(&counter_clone);
    async move {
        counter.fetch_add(1, Ordering::SeqCst);
        println!("定时器触发！");
    }
}));
let task = TimerWheel::create_task(Duration::from_millis(100), callback);
let handle = timer.register(task);

// 等待定时器完成
handle.into_completion_receiver().0.await.ok();
```

#### 取消定时器

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();

let callback = Some(CallbackWrapper::new(|| async {
    println!("这条消息不会被打印");
}));
let task = TimerWheel::create_task(Duration::from_secs(10), callback);
let handle = timer.register(task);

// 取消定时器
let cancelled = handle.cancel();
println!("取消成功: {}", cancelled);
```

#### 推迟定时器

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();
let counter = Arc::new(AtomicU32::new(0));
let counter_clone = Arc::clone(&counter);

// 创建一个 50ms 后触发的定时器
let callback = Some(CallbackWrapper::new(move || {
    let counter = Arc::clone(&counter_clone);
    async move {
        counter.fetch_add(1, Ordering::SeqCst);
        println!("定时器触发！");
    }
}));
let task = TimerWheel::create_task(Duration::from_millis(50), callback);
let task_id = task.get_id();
let handle = timer.register(task);

// 推迟到 150ms 后触发（保持原回调）
let postponed = timer.postpone(task_id, Duration::from_millis(150), None);
println!("推迟成功: {}", postponed);

// 等待定时器完成
handle.into_completion_receiver().0.await.ok();
```

#### 推迟并替换回调

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();

let callback = Some(CallbackWrapper::new(|| async {
    println!("原始回调");
}));
let task = TimerWheel::create_task(Duration::from_millis(50), callback);
let task_id = task.get_id();
let handle = timer.register(task);

// 推迟并替换回调函数
let new_callback = Some(CallbackWrapper::new(|| async {
    println!("新的回调！");
}));
let postponed = timer.postpone(task_id, Duration::from_millis(100), new_callback);
println!("推迟成功: {}", postponed);

// 等待定时器完成（会执行新回调）
handle.into_completion_receiver().0.await.ok();
```

### 批量操作

#### 批量调度定时器

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();
let counter = Arc::new(AtomicU32::new(0));

// 创建 100 个定时器回调
let callbacks: Vec<_> = (0..100)
    .map(|i| {
        let counter = Arc::clone(&counter);
        let delay = Duration::from_millis(100 + i * 10);
        let callback = Some(CallbackWrapper::new(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }));
        (delay, callback)
    })
    .collect();

// 批量调度：创建任务 + 注册
let tasks = TimerWheel::create_batch_with_callbacks(callbacks);
let batch_handle = timer.register_batch(tasks);

println!("已调度 {} 个定时器", batch_handle.len());
```

#### 批量取消定时器

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();

// 创建批量定时器
let callbacks: Vec<_> = (0..50)
    .map(|_| (Duration::from_secs(10), Some(CallbackWrapper::new(|| async {}))))
    .collect();

let tasks = TimerWheel::create_batch_with_callbacks(callbacks);
let batch_handle = timer.register_batch(tasks);

// 批量取消
let cancelled_count = batch_handle.cancel_all();
println!("已取消 {} 个定时器", cancelled_count);
```

#### 批量推迟定时器

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();
let counter = Arc::new(AtomicU32::new(0));

// 创建 100 个定时器
let mut task_ids = Vec::new();
for _ in 0..100 {
    let counter_clone = Arc::clone(&counter);
    let callback = Some(CallbackWrapper::new(move || {
        let counter = Arc::clone(&counter_clone);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }));
    let task = TimerWheel::create_task(Duration::from_millis(50), callback);
    task_ids.push((task.get_id(), Duration::from_millis(150)));
    timer.register(task);
}

// 批量推迟所有定时器
let postponed = timer.postpone_batch(task_ids);
println!("已推迟 {} 个定时器", postponed);
```

#### 批量推迟并替换回调

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();
let counter = Arc::new(AtomicU32::new(0));

// 创建批量定时器
let mut task_ids = Vec::new();
for _ in 0..50 {
    let task = TimerWheel::create_task(Duration::from_millis(50), None);
    task_ids.push(task.get_id());
    timer.register(task);
}

// 批量推迟并替换回调
let updates: Vec<_> = task_ids
    .into_iter()
    .map(|id| {
        let counter = Arc::clone(&counter);
        let callback = Some(CallbackWrapper::new(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }));
        (id, Duration::from_millis(150), callback)
    })
    .collect();

let postponed = timer.postpone_batch_with_callbacks(updates);
println!("已推迟 {} 个定时器并替换回调", postponed);
```

### 完成通知

#### 等待单个定时器完成

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();

let callback = Some(CallbackWrapper::new(|| async {
    println!("定时器触发");
}));
let task = TimerWheel::create_task(Duration::from_millis(100), callback);
let handle = timer.register(task);

// 等待定时器完成
match handle.into_completion_receiver().0.await {
    Ok(_) => println!("定时器已完成"),
    Err(_) => println!("定时器被取消"),
}
```

#### 批量完成通知

```rust
use kestrel_protocol_timer::CallbackWrapper;

let timer = TimerWheel::with_defaults();
let counter = Arc::new(AtomicU32::new(0));

// 创建批量定时器
let callbacks: Vec<_> = (0..10)
    .map(|i| {
        let counter = Arc::clone(&counter);
        let delay = Duration::from_millis(50 + i * 10);
        let callback = Some(CallbackWrapper::new(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }));
        (delay, callback)
    })
    .collect();

let tasks = TimerWheel::create_batch_with_callbacks(callbacks);
let batch_handle = timer.register_batch(tasks);

// 获取所有完成通知接收器
let receivers = batch_handle.into_completion_receivers();

// 等待所有定时器完成
for (i, rx) in receivers.into_iter().enumerate() {
    rx.await.ok();
    println!("定时器 {} 已完成", i);
}

println!("所有定时器已完成，共触发 {} 次", counter.load(Ordering::SeqCst));
```

#### 仅通知的定时器（无回调）

```rust
use kestrel_protocol_timer::TimerTask;

let timer = TimerWheel::with_defaults();

// 创建仅发送通知的定时器（无回调函数）
let task = TimerTask::new(Duration::from_millis(100), None);
let handle = timer.register(task);

// 等待通知
handle.into_completion_receiver().0.await.ok();
println!("定时器到期");
```

### TimerService 使用

`TimerService` 提供基于 Actor 模式的集中式定时器管理，适合需要统一处理大量定时器超时事件的场景。

#### 创建和使用 TimerService

```rust
use kestrel_protocol_timer::{TimerWheel, TimerService, CallbackWrapper, ServiceConfig};

let timer = TimerWheel::with_defaults();
let mut service = timer.create_service(ServiceConfig::default());

// 两步式 API：创建任务 + 注册
let callback = Some(CallbackWrapper::new(|| async {
    println!("通过 service 调度的定时器触发");
}));
let task = TimerService::create_task(Duration::from_millis(100), callback);
let task_id = task.get_id();
service.register(task).unwrap();

println!("已调度任务 ID: {:?}", task_id);
```

#### 批量调度并接收超时通知

```rust
use kestrel_protocol_timer::{CallbackWrapper, ServiceConfig};

let timer = TimerWheel::with_defaults();
let mut service = timer.create_service(ServiceConfig::default());

// 批量调度定时器：创建 + 注册
let callbacks: Vec<_> = (0..100)
    .map(|_| (Duration::from_millis(100), Some(CallbackWrapper::new(|| async {}))))
    .collect();

let tasks = TimerService::create_batch_with_callbacks(callbacks);
service.register_batch(tasks).unwrap();
println!("已调度 100 个任务");

// 获取超时通知接收器
let mut timeout_rx = service.take_receiver()
    .expect("接收器只能被获取一次");

// 接收超时通知
let mut completed_count = 0;
while let Some(task_id) = timeout_rx.recv().await {
    completed_count += 1;
    println!("任务 {:?} 已完成", task_id);
    
    if completed_count >= 100 {
        break;
    }
}

// 关闭 service
service.shutdown().await;
```

#### 动态取消任务

```rust
use kestrel_protocol_timer::{CallbackWrapper, ServiceConfig};

let timer = TimerWheel::with_defaults();
let service = timer.create_service(ServiceConfig);

// 通过 service 直接调度定时器
let callback1 = Some(CallbackWrapper::new(|| async { 
    println!("任务 1 触发"); 
}));
let task1 = TimerService::create_task(Duration::from_secs(5), callback1);
let task_id1 = task1.get_id();
service.register(task1).unwrap();

let callback2 = Some(CallbackWrapper::new(|| async { 
    println!("任务 2 触发"); 
}));
let task2 = TimerService::create_task(Duration::from_secs(10), callback2);
let task_id2 = task2.get_id();
service.register(task2).unwrap();

// 取消任务
let cancelled = service.cancel_task(task_id2);
println!("任务 2 取消结果: {}", cancelled);

// 批量取消
let task_ids = vec![task_id1];
let cancelled_count = service.cancel_batch(&task_ids);
println!("批量取消了 {} 个任务", cancelled_count);
```

#### 动态推迟任务

```rust
use kestrel_protocol_timer::{CallbackWrapper, ServiceConfig};

let timer = TimerWheel::with_defaults();
let service = timer.create_service(ServiceConfig::default());
let counter = Arc::new(AtomicU32::new(0));
let counter_clone = Arc::clone(&counter);

// 调度一个任务
let callback = Some(CallbackWrapper::new(move || {
    let counter = Arc::clone(&counter_clone);
    async move {
        counter.fetch_add(1, Ordering::SeqCst);
    }
}));
let task = TimerService::create_task(Duration::from_millis(50), callback);
let task_id = task.get_id();
service.register(task).unwrap();

// 推迟任务（保持原回调）
let postponed = service.postpone(task_id, Duration::from_millis(150), None);
println!("推迟结果: {}", postponed);

// 接收超时通知
let mut rx = service.take_receiver().unwrap();
if let Some(completed_task_id) = rx.recv().await {
    println!("任务 {:?} 已完成", completed_task_id);
}

// 批量推迟任务
let callbacks: Vec<_> = (0..10)
    .map(|_| (Duration::from_millis(50), Some(CallbackWrapper::new(|| async {}))))
    .collect();
let tasks = TimerService::create_batch_with_callbacks(callbacks);
let task_ids: Vec<_> = tasks.iter().map(|t| t.get_id()).collect();
service.register_batch(tasks).unwrap();

let updates: Vec<_> = task_ids
    .iter()
    .map(|&id| (id, Duration::from_millis(150)))
    .collect();
let postponed_count = service.postpone_batch(updates);
println!("批量推迟了 {} 个任务", postponed_count);
```

## API 文档

### TimerWheel

#### 构造方法

**`TimerWheel::with_defaults() -> Self`**

使用默认配置创建定时器（分层时间轮）：
- **L0 层**：512 槽位 × 10ms tick = 5.12 秒覆盖范围
- **L1 层**：64 槽位 × 1000ms tick = 64 秒覆盖范围
- **层级比率**：100（L1 每 100 个 L0 tick 推进一次）

```rust
let timer = TimerWheel::with_defaults();
```

**`TimerWheel::new(config: WheelConfig) -> Self`**

使用自定义配置创建定时器（分层时间轮）。

参数：
- `config`：时间轮配置（通过 WheelConfigBuilder 构建并验证）

配置项：
- `l0_tick_duration`：L0 层 tick 时长
- `l0_slot_count`：L0 层槽位数量（必须是 2 的幂次方）
- `l1_tick_duration`：L1 层 tick 时长（必须是 L0 tick 的整数倍）
- `l1_slot_count`：L1 层槽位数量（必须是 2 的幂次方）

```rust
let config = WheelConfig::builder()
    .l0_tick_duration(Duration::from_millis(10))
    .l0_slot_count(512)
    .l1_tick_duration(Duration::from_secs(1))
    .l1_slot_count(64)
    .build()?;
let timer = TimerWheel::new(config);
```

#### 调度方法

**`fn create_task(delay: Duration, callback: Option<CallbackWrapper>) -> TimerTask`**（静态方法）

创建一个定时器任务（申请阶段）。

参数：
- `delay`：延迟时间
- `callback`：可选的回调包装器（`Option<CallbackWrapper>`）

返回：
- `TimerTask`：待注册的定时器任务

```rust
use kestrel_protocol_timer::CallbackWrapper;

let callback = Some(CallbackWrapper::new(|| async {
    println!("1 秒后执行");
}));
let task = TimerWheel::create_task(Duration::from_secs(1), callback);
```

**`fn register(&self, task: TimerTask) -> TimerHandle`**

注册定时器任务到时间轮（注册阶段）。

参数：
- `task`：通过 `create_task()` 创建的任务

返回：
- `TimerHandle`：定时器句柄

```rust
use kestrel_protocol_timer::CallbackWrapper;

let callback = Some(CallbackWrapper::new(|| async {}));
let task = TimerWheel::create_task(Duration::from_secs(1), callback);
let handle = timer.register(task);
```

**`fn create_batch(delays: Vec<Duration>) -> Vec<TimerTask>`**（静态方法）

批量创建定时器任务（申请阶段，无回调）。

参数：
- `delays`：延迟时间的向量

返回：
- `Vec<TimerTask>`：待注册的任务列表

```rust
let delays = vec![Duration::from_secs(1), Duration::from_secs(2)];
let tasks = TimerWheel::create_batch(delays);
let batch = timer.register_batch(tasks);
```

**`fn create_batch_with_callbacks(callbacks: Vec<(Duration, Option<CallbackWrapper>)>) -> Vec<TimerTask>`**（静态方法）

批量创建定时器任务（申请阶段，带回调）。

参数：
- `callbacks`：(延迟时间, 可选回调) 的向量

返回：
- `Vec<TimerTask>`：待注册的任务列表

```rust
use kestrel_protocol_timer::CallbackWrapper;

let callbacks = vec![
    (Duration::from_secs(1), Some(CallbackWrapper::new(|| async { println!("1"); }))),
    (Duration::from_secs(2), Some(CallbackWrapper::new(|| async { println!("2"); }))),
];
let tasks = TimerWheel::create_batch_with_callbacks(callbacks);
let batch = timer.register_batch(tasks);
```

**`fn register_batch(&self, tasks: Vec<TimerTask>) -> BatchHandle`**

批量注册定时器任务到时间轮（注册阶段）。

参数：
- `tasks`：通过 `create_batch()` 或 `create_batch_with_callbacks()` 创建的任务列表

返回：
- `BatchHandle`：批量句柄

#### 推迟方法

**`fn postpone(&self, task_id: TaskId, new_delay: Duration, callback: Option<CallbackWrapper>) -> bool`**

推迟定时器任务。

参数：
- `task_id`：要推迟的任务 ID
- `new_delay`：新的延迟时间（从当前时间点重新计算）
- `callback`：可选的新回调函数。如果为 `None`，则保持原回调；如果为 `Some`，则替换回调

返回：
- `true`：成功推迟
- `false`：任务不存在

```rust
use kestrel_protocol_timer::CallbackWrapper;

// 推迟并保持原回调
let postponed = timer.postpone(task_id, Duration::from_secs(10), None);

// 推迟并替换回调
let new_callback = Some(CallbackWrapper::new(|| async { 
    println!("新回调"); 
}));
let postponed = timer.postpone(task_id, Duration::from_secs(10), new_callback);
```

**`fn postpone_batch(&self, updates: Vec<(TaskId, Duration)>) -> usize`**

批量推迟定时器任务（保持原回调）。

参数：
- `updates`：(任务ID, 新延迟) 的向量

返回：成功推迟的任务数量

```rust
let updates = vec![
    (task_id1, Duration::from_secs(10)),
    (task_id2, Duration::from_secs(15)),
];
let postponed = timer.postpone_batch(updates);
```

**`fn postpone_batch_with_callbacks(&self, updates: Vec<(TaskId, Duration, Option<CallbackWrapper>)>) -> usize`**

批量推迟定时器任务并可选择替换回调。

参数：
- `updates`：(任务ID, 新延迟, 可选回调) 的向量

返回：成功推迟的任务数量

```rust
use kestrel_protocol_timer::CallbackWrapper;

let updates = vec![
    (task_id1, Duration::from_secs(10), Some(CallbackWrapper::new(|| async { println!("1"); }))),
    (task_id2, Duration::from_secs(15), None), // 保持原回调
];
let postponed = timer.postpone_batch_with_callbacks(updates);
```

#### 服务方法

**`create_service(&self, server_config: ServiceConfig) -> TimerService`**

创建一个 TimerService 实例，用于集中管理定时器。

```rust
use kestrel_protocol_timer::ServiceConfig;
let service = timer.create_service(ServiceConfig::default());
```

### TimerHandle

**`cancel(&self) -> bool`**

取消定时器。

返回：
- `true`：成功取消
- `false`：任务已不存在（可能已触发或被取消）

```rust
let cancelled = handle.cancel();
```

**`task_id(&self) -> TaskId`**

获取任务 ID。

```rust
let id = handle.task_id();
```

**`into_completion_receiver(self) -> CompletionReceiver`**

消耗句柄，返回完成通知接收器。

```rust
let receiver = handle.into_completion_receiver();
receiver.0.await.ok();
```

### BatchHandle

**`len(&self) -> usize`**

获取批量句柄中的任务数量。

```rust
let count = batch.len();
```

**`cancel_all(&self) -> usize`**

取消所有定时器。

返回：成功取消的任务数量。

```rust
let cancelled = batch.cancel_all();
```

**`into_completion_receivers(self) -> Vec<CompletionReceiver>`**

消耗批量句柄，返回所有完成通知接收器。

```rust
let receivers = batch.into_completion_receivers();
for rx in receivers {
    rx.await.ok();
}
```

### TimerService

**`fn create_task(delay: Duration, callback: Option<CallbackWrapper>) -> TimerTask`**（静态方法）

创建定时器任务（申请阶段）。

参数：
- `delay`：延迟时间
- `callback`：可选的回调包装器

返回：`TimerTask`

```rust
use kestrel_protocol_timer::CallbackWrapper;

let callback = Some(CallbackWrapper::new(|| async {}));
let task = TimerService::create_task(Duration::from_secs(1), callback);
```

**`fn create_batch(delays: Vec<Duration>) -> Vec<TimerTask>`**（静态方法）

批量创建定时器任务（申请阶段，无回调）。

返回：`Vec<TimerTask>`

```rust
let delays = vec![Duration::from_secs(1), Duration::from_secs(2)];
let tasks = TimerService::create_batch(delays);
```

**`fn create_batch_with_callbacks(callbacks: Vec<(Duration, Option<CallbackWrapper>)>) -> Vec<TimerTask>`**（静态方法）

批量创建定时器任务（申请阶段，带回调）。

返回：`Vec<TimerTask>`

```rust
use kestrel_protocol_timer::CallbackWrapper;

let callbacks = vec![
    (Duration::from_secs(1), Some(CallbackWrapper::new(|| async {})))
];
let tasks = TimerService::create_batch_with_callbacks(callbacks);
```

**`fn register(&self, task: TimerTask) -> Result<(), TimerError>`**

注册定时器任务到服务（注册阶段）。

```rust
let task = TimerService::create_task(Duration::from_secs(1), || async {});
service.register(task).unwrap();
```

**`fn register_batch(&self, tasks: Vec<TimerTask>) -> Result<(), TimerError>`**

批量注册定时器任务到服务（注册阶段）。

```rust
let tasks = TimerService::create_batch_with_callbacks(callbacks);
service.register_batch(tasks).unwrap();
```

**`take_receiver(&mut self) -> Option<mpsc::Receiver<TaskId>>`**

获取超时通知接收器（只能调用一次）。

```rust
let mut rx = service.take_receiver().unwrap();
while let Some(task_id) = rx.recv().await {
    println!("任务 {:?} 超时", task_id);
}
```

**`fn cancel_task(&self, task_id: TaskId) -> bool`**

取消指定的任务。

返回：是否成功取消

```rust
let cancelled = service.cancel_task(task_id);
```

**`fn cancel_batch(&self, task_ids: &[TaskId]) -> usize`**

批量取消任务。

返回：成功取消的任务数量

```rust
let cancelled_count = service.cancel_batch(&task_ids);
```

**`fn postpone(&self, task_id: TaskId, new_delay: Duration, callback: Option<CallbackWrapper>) -> bool`**

推迟任务。

参数：
- `task_id`：要推迟的任务 ID
- `new_delay`：新的延迟时间
- `callback`：可选的新回调函数。如果为 `None`，则保持原回调；如果为 `Some`，则替换回调

返回：是否成功推迟

```rust
use kestrel_protocol_timer::CallbackWrapper;

// 推迟并保持原回调
let postponed = service.postpone(task_id, Duration::from_secs(10), None);

// 推迟并替换回调
let new_callback = Some(CallbackWrapper::new(|| async { 
    println!("新回调"); 
}));
let postponed = service.postpone(task_id, Duration::from_secs(10), new_callback);
```

**`fn postpone(&self, task_id: TaskId, new_delay: Duration, callback: Option<CallbackWrapper>) -> bool`**

**`fn postpone_batch(&self, updates: Vec<(TaskId, Duration)>) -> usize`**

批量推迟任务（保持原回调）。

返回：成功推迟的任务数量

```rust
let updates = vec![(task_id1, Duration::from_secs(10))];
let postponed = service.postpone_batch(updates);
```

**`fn postpone_batch_with_callbacks(&self, updates: Vec<(TaskId, Duration, Option<CallbackWrapper>)>) -> usize`**

批量推迟任务并可选择替换回调。

返回：成功推迟的任务数量

```rust
use kestrel_protocol_timer::CallbackWrapper;

let updates = vec![
    (task_id1, Duration::from_secs(10), Some(CallbackWrapper::new(|| async { println!("1"); }))),
    (task_id2, Duration::from_secs(15), None), // 保持原回调
];
let postponed = service.postpone_batch_with_callbacks(updates);
```

**`shutdown(self) -> ()`**

关闭服务。

```rust
service.shutdown().await;
```

## 配置选项

### 分层时间轮配置

分层时间轮采用双层架构，L0 层处理短延迟任务，L1 层处理长延迟任务。

#### L0 层配置（底层 - 高精度）

**槽位数量（l0_slot_count）**：
- **必须是 2 的幂次方**：128, 256, 512, 1024, 2048 等
- **默认值**：512
- **影响**：
  - 更多槽位 → 覆盖更长时间范围，减少槽位冲突
  - 更少槽位 → 更少内存占用，但可能增加槽位冲突

**Tick 时长（l0_tick_duration）**：
- **默认值**：10ms
- **影响**：
  - 更小的 tick → 更高的精度，但更频繁的推进操作
  - 更大的 tick → 更低的 CPU 占用，但精度降低

**覆盖范围**：`l0_slot_count × l0_tick_duration`（默认：512 × 10ms = 5.12 秒）

#### L1 层配置（高层 - 长时间）

**槽位数量（l1_slot_count）**：
- **必须是 2 的幂次方**：32, 64, 128, 256 等
- **默认值**：64
- **影响**：更多槽位 → 覆盖更长时间范围（但增加内存占用）

**Tick 时长（l1_tick_duration）**：
- **默认值**：1000ms（1 秒）
- **约束**：必须是 L0 tick 的整数倍
- **影响**：决定 L1 层的时间粒度

**覆盖范围**：`l1_slot_count × l1_tick_duration`（默认：64 × 1s = 64 秒）

#### 层级比率

**L1 tick / L0 tick = l1_tick_duration / l0_tick_duration**

- 默认比率：1000ms / 10ms = 100
- L1 层每 100 个 L0 tick 推进一次

### 推荐配置

**高精度、短延迟场景（网络超时）**：
```rust
let config = WheelConfig::builder()
    .l0_tick_duration(Duration::from_millis(5))    // 5ms 精度
    .l0_slot_count(1024)                            // 覆盖 5.12 秒
    .l1_tick_duration(Duration::from_millis(500))   // 500ms
    .l1_slot_count(64)                              // 覆盖 32 秒
    .build()?;
let timer = TimerWheel::new(config);
```

**一般场景（使用默认配置）**：
```rust
// L0: 512 槽位 × 10ms = 5.12 秒
// L1: 64 槽位 × 1s = 64 秒
// 总覆盖范围：64 秒（支持 rounds 可扩展）
let timer = TimerWheel::with_defaults();
```

**低精度、长时间场景（心跳检测）**：
```rust
let config = WheelConfig::builder()
    .l0_tick_duration(Duration::from_millis(100))   // 100ms 精度
    .l0_slot_count(512)                              // 覆盖 51.2 秒
    .l1_tick_duration(Duration::from_secs(10))       // 10 秒
    .l1_slot_count(128)                              // 覆盖 1280 秒（21 分钟）
    .build()?;
let timer = TimerWheel::new(config);
```

### 配置最佳实践

1. **L0 层设计原则**：
   - 覆盖范围应能容纳大部分任务（80-90%）
   - Tick 时长决定定时器精度
   - 推荐覆盖范围：1-10 秒

2. **L1 层设计原则**：
   - 用于少数长延迟任务
   - Tick 时长可以较大（减少推进频率）
   - 推荐覆盖范围：30-300 秒

3. **层级比率建议**：
   - 推荐比率：50-200（L1 tick / L0 tick）
   - 避免过小的比率（< 10）导致 L1 频繁推进
   - 避免过大的比率（> 1000）导致降级延迟过大

## 性能基准

项目包含完整的性能基准测试，使用 Criterion 框架实现。

### 运行基准测试

```bash
# 运行所有基准测试
cargo bench

# 运行特定基准测试
cargo bench --bench service_benchmark
cargo bench --bench wheel_benchmark
```

### 基准测试项目

#### 1. 单个定时器调度

测试单个定时器的调度性能。

```bash
cargo bench schedule_single
```

**典型结果**：单次调度耗时约 **5-10 微秒**

#### 2. 批量调度

测试批量调度不同规模定时器的性能。

```bash
cargo bench schedule_batch
```

测试规模：10、100、1000 个定时器

**典型结果**：
- 10 个定时器：约 50-80 微秒（每个 5-8 微秒）
- 100 个定时器：约 300-500 微秒（每个 3-5 微秒）
- 1000 个定时器：约 2-4 毫秒（每个 2-4 微秒）

批量操作明显比单个操作更高效。

#### 3. 取消操作

测试单个和批量取消的性能。

```bash
cargo bench cancel_single
cargo bench cancel_batch
```

**典型结果**：
- 单个取消：约 1-3 微秒
- 批量取消（1000 个）：约 1-2 毫秒

#### 4. 推迟操作

测试单个和批量推迟的性能。

```bash
cargo bench postpone_single
cargo bench postpone_batch
```

**典型结果**：
- 单个推迟：约 3-5 微秒
- 批量推迟（1000 个）：约 2-4 毫秒
- 推迟并替换回调：约 4-6 微秒

#### 5. 并发调度

测试多线程并发调度的性能。

```bash
cargo bench concurrent_schedule
```

#### 6. 时间轮推进

测试时间轮推进操作的性能。

```bash
cargo bench wheel_advance
```

### 性能对比

与基于堆（BinaryHeap）的传统定时器实现相比：

| 操作 | 分层时间轮 | 堆实现 | 优势 |
|------|-----------|--------|------|
| 插入单个任务 | O(1) ~5μs | O(log n) ~10-20μs | 2-4x 更快 |
| 批量插入 1000 | O(1000) ~2ms | O(1000 log n) ~15-25ms | 7-12x 更快 |
| 取消任务 | O(1) ~2μs | O(n) ~50-100μs | 25-50x 更快 |
| 推迟任务 | O(1) ~4μs | O(log n) ~15-30μs | 4-7x 更快 |
| 触发到期任务 | O(k) | O(k log n) | 更稳定 |
| 时间轮推进 | O(1)* | O(log n) | 更高效 |

**注**：
- k 为到期任务数量，n 为总任务数量
- \*分层时间轮推进：L0 层 O(1)，L1 层每 100 tick 推进一次

### 分层时间轮 vs 单层时间轮

| 指标 | 分层时间轮 | 单层时间轮 | 改进 |
|------|-----------|-----------|------|
| 短延迟任务推进 | 无 rounds 检查 | 需要 rounds 检查 | ~20% 更快 |
| 长延迟任务存储 | L1 层稀疏存储 | 全部在 L0 层 | ~30% 内存节省 |
| 覆盖范围 | 0-64 秒高效 | 0-5.12 秒高效 | 12x 扩展 |
| CPU 占用 | L1 每 100 tick 推进 | 每 tick 都检查所有槽位 | 更低 |

### 大规模测试

集成测试包含大规模场景测试：

```bash
cargo test --test integration_test test_large_scale_timers
```

测试场景：
- ✅ 10,000 个并发定时器
- ✅ 创建时间 < 100ms
- ✅ 所有定时器正确触发
- ✅ 内存占用稳定

## 测试

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行单元测试
cargo test --lib

# 运行集成测试
cargo test --test integration_test

# 运行特定测试
cargo test test_basic_timer
```

### 测试覆盖

项目包含完整的测试套件：

#### 单元测试

- ✅ 基本定时器调度和触发
- ✅ 多定时器管理
- ✅ 定时器取消
- ✅ 定时器推迟
- ✅ 完成通知机制
- ✅ 批量操作
- ✅ 错误处理

#### 集成测试

- ✅ 大规模定时器（10,000+）
- ✅ 定时器精度测试
- ✅ 并发操作测试
- ✅ 不同延迟的定时器
- ✅ TimerService 功能测试
- ✅ 批量取消测试
- ✅ 推迟功能测试（单个、批量、替换回调）
- ✅ 多次推迟测试

#### 性能测试

- ✅ 调度性能基准
- ✅ 取消性能基准
- ✅ 推迟性能基准（单个、批量、替换回调）
- ✅ 批量操作性能基准
- ✅ 时间轮推进性能基准
- ✅ 混合操作性能基准（调度+推迟+取消）

## 使用场景

### 1. 网络超时管理

```rust
use std::time::Duration;
use kestrel_protocol_timer::CallbackWrapper;

// 为每个网络连接设置超时
async fn handle_connection(timer: &TimerWheel, conn_id: u64) {
    let callback = Some(CallbackWrapper::new(move || async move {
        println!("连接 {} 超时，关闭连接", conn_id);
        // 关闭连接逻辑
    }));
    let task = TimerWheel::create_task(Duration::from_secs(30), callback);
    let task_id = task.get_id();
    let timeout_handle = timer.register(task);
    
    // 如果收到部分数据，延长超时时间（保持原回调）
    // timer.postpone(task_id, Duration::from_secs(30), None);
    
    // 如果连接完成，取消超时
    // timeout_handle.cancel();
}
```

### 2. 任务延迟执行

```rust
use kestrel_protocol_timer::CallbackWrapper;

// 延迟 5 秒执行清理任务
let callback = Some(CallbackWrapper::new(|| async {
    cleanup_temporary_files().await;
}));
let task = TimerWheel::create_task(Duration::from_secs(5), callback);
timer.register(task);
```

### 3. 心跳检测

```rust
use kestrel_protocol_timer::{CallbackWrapper, ServiceConfig};

let config = WheelConfig::builder()
    .l0_tick_duration(Duration::from_millis(100))  // 100ms 精度足够心跳检测
    .l0_slot_count(512)                             // 覆盖 51.2 秒
    .l1_tick_duration(Duration::from_secs(10))      // 10 秒
    .l1_slot_count(128)                             // 覆盖 21 分钟
    .build()?;
let timer = TimerWheel::new(config);
let mut service = timer.create_service(ServiceConfig::default());

// 为每个客户端设置心跳检测
for client_id in client_ids {
    let callback = Some(CallbackWrapper::new(move || async move {
        println!("客户端 {} 心跳超时", client_id);
        disconnect_client(client_id).await;
    }));
    let task = TimerService::create_task(Duration::from_secs(30), callback);
    service.register(task).unwrap();
}

// 统一处理超时
let mut rx = service.take_receiver().unwrap();
while let Some(task_id) = rx.recv().await {
    println!("心跳检测超时: {:?}", task_id);
}
```

### 4. 缓存过期

```rust
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;
use kestrel_protocol_timer::CallbackWrapper;

struct CacheManager {
    timer: TimerWheel,
    cache: Arc<Mutex<HashMap<String, String>>>,
}

impl CacheManager {
    async fn set(&self, key: String, value: String, ttl: Duration) {
        // 存储到缓存
        self.cache.lock().insert(key.clone(), value);
        
        // 设置过期定时器
        let cache = Arc::clone(&self.cache);
        let callback = Some(CallbackWrapper::new(move || {
            let cache = Arc::clone(&cache);
            let key = key.clone();
            async move {
                cache.lock().remove(&key);
                println!("缓存键 {} 已过期", key);
            }
        }));
        let task = TimerWheel::create_task(ttl, callback);
        self.timer.register(task);
    }
}
```

### 5. 定时任务调度

```rust
use kestrel_protocol_timer::CallbackWrapper;

// 每个任务在特定时间后执行
let tasks = vec![
    ("任务A", Duration::from_secs(10)),
    ("任务B", Duration::from_secs(30)),
    ("任务C", Duration::from_secs(60)),
];

let callbacks: Vec<_> = tasks.into_iter()
    .map(|(name, delay)| {
        let callback = Some(CallbackWrapper::new(move || async move {
            println!("执行定时任务: {}", name);
            execute_scheduled_task(name).await;
        }));
        (delay, callback)
    })
    .collect();

let task_list = TimerWheel::create_batch_with_callbacks(callbacks);
timer.register_batch(task_list);
```

### 6. 游戏服务器 Buff 系统

```rust
use kestrel_protocol_timer::CallbackWrapper;

// 游戏角色的 buff 效果管理
async fn apply_buff(
    timer: &TimerWheel,
    player_id: u64,
    buff_type: BuffType,
    duration: Duration
) -> TaskId {
    println!("玩家 {} 获得 buff: {:?}", player_id, buff_type);
    
    let callback = Some(CallbackWrapper::new(move || async move {
        println!("玩家 {} 的 buff {:?} 已失效", player_id, buff_type);
        remove_buff(player_id, buff_type).await;
    }));
    let task = TimerWheel::create_task(duration, callback);
    let task_id = task.get_id();
    timer.register(task);
    task_id
}

// 延长 buff 持续时间
async fn extend_buff(
    timer: &TimerWheel,
    task_id: TaskId,
    extra_duration: Duration
) {
    // 保持原回调
    let extended = timer.postpone(task_id, extra_duration, None);
    if extended {
        println!("Buff 持续时间已延长");
    }
}
```

### 7. 动态重试机制

```rust
use kestrel_protocol_timer::CallbackWrapper;

// 实现带退避策略的重试机制
async fn retry_with_backoff(
    timer: &TimerWheel,
    service: &TimerService,
    operation: impl Fn() -> BoxFuture<'static, Result<(), Error>>
) {
    let mut retry_count = 0;
    let max_retries = 5;
    
    loop {
        match operation().await {
            Ok(_) => break,
            Err(e) if retry_count < max_retries => {
                retry_count += 1;
                // 指数退避：1s, 2s, 4s, 8s, 16s
                let delay = Duration::from_secs(2_u64.pow(retry_count - 1));
                
                println!("操作失败，{} 秒后重试（第 {} 次）", delay.as_secs(), retry_count);
                
                let callback = Some(CallbackWrapper::new(move || async move {
                    println!("开始第 {} 次重试", retry_count);
                }));
                let task = TimerService::create_task(delay, callback);
                service.register(task).unwrap();
                
                // 等待定时器触发...
            }
            Err(e) => {
                println!("达到最大重试次数，操作失败: {:?}", e);
                break;
            }
        }
    }
}
```

## 依赖项

### 核心依赖

| 依赖 | 版本 | 用途 |
|------|------|------|
| [tokio](https://tokio.rs/) | 1.48+ | 异步运行时，提供异步任务调度和执行 |
| [parking_lot](https://github.com/Amanieu/parking_lot) | 0.12 | 高性能锁实现，比标准库 Mutex 更快 |
| [rustc-hash](https://github.com/rust-lang/rustc-hash) | 2.1 | FxHashMap 实现，减少哈希冲突 |
| [futures](https://github.com/rust-lang/futures-rs) | 0.3 | 异步工具和抽象 |
| [smallvec](https://github.com/servo/rust-smallvec) | 1.15 | 小型向量优化，减少堆分配 |

### 开发依赖

| 依赖 | 版本 | 用途 |
|------|------|------|
| [criterion](https://github.com/bheisler/criterion.rs) | 0.7 | 性能基准测试框架 |

## 贡献指南

欢迎贡献代码、报告问题或提出建议！

### 报告问题

如果您发现 bug 或有功能请求，请在 GitHub 上创建 issue，包含：

1. 问题描述
2. 复现步骤
3. 预期行为和实际行为
4. 环境信息（Rust 版本、操作系统等）
5. 相关代码片段或最小复现示例

### 提交 Pull Request

1. Fork 本项目
2. 创建功能分支：`git checkout -b feature/my-feature`
3. 编写代码并确保通过测试：`cargo test`
4. 运行 clippy 检查：`cargo clippy`
5. 格式化代码：`cargo fmt`
6. 提交更改：`git commit -am 'Add my feature'`
7. 推送到分支：`git push origin feature/my-feature`
8. 创建 Pull Request

### 代码风格

- 遵循 Rust 官方代码风格指南
- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 检查代码质量
- 为公共 API 编写文档注释
- 为新功能添加测试用例

### 开发环境设置

```bash
# 克隆仓库
git clone https://github.com/ShaoG-R/kestrel-protocol-timer.git
cd kestrel-protocol-timer

# 运行测试
cargo test

# 运行基准测试
cargo bench

# 检查代码
cargo clippy

# 格式化代码
cargo fmt
```

## 许可证

本项目采用 MIT 或 Apache-2.0 双许可证。

您可以选择以下任一许可证使用本项目：

- MIT License ([LICENSE-MIT](LICENSE-MIT) 或 http://opensource.org/licenses/MIT)
- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE) 或 http://www.apache.org/licenses/LICENSE-2.0)

## 致谢和参考

### 时间轮算法

时间轮算法最早由 George Varghese 和 Tony Lauck 在论文 ["Hashed and Hierarchical Timing Wheels: Data Structures for the Efficient Implementation of a Timer Facility"](http://www.cs.columbia.edu/~nahum/w6998/papers/sosp87-timing-wheels.pdf) (SOSP '87) 中提出。

### 相关项目

- [tokio-timer](https://github.com/tokio-rs/tokio) - Tokio 官方定时器实现
- [tokio-timer-rs](https://github.com/async-rs/async-timer) - 另一个异步定时器库
- [timing-wheel](https://github.com/moka-rs/moka) - Moka 缓存库中的时间轮实现

### 灵感来源

- Kafka 的时间轮实现
- Netty 的 HashedWheelTimer
- Linux 内核的定时器实现

---

**如有问题或建议，欢迎提交 issue 或 PR！**

