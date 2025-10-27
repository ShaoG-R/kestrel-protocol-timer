use crate::config::{BatchConfig, WheelConfig};
use crate::task::{TaskCompletionReason, TaskId, TaskLocation, TimerTask};
use rustc_hash::FxHashMap;
use std::time::Duration;
use smallvec::SmallVec;

/// 时间轮数据结构
pub struct Wheel {
    /// 槽位数组，每个槽位存储一组定时器任务
    slots: Vec<Vec<TimerTask>>,
    
    /// 当前时间指针（tick 索引）
    current_tick: u64,
    
    /// 槽位数量
    slot_count: usize,
    
    /// 每个 tick 的时间长度
    tick_duration: Duration,
    
    /// 任务索引，用于快速查找和取消任务
    task_index: FxHashMap<TaskId, TaskLocation>,
    
    /// 批处理配置
    batch_config: BatchConfig,
}

impl Wheel {
    /// 创建新的时间轮
    ///
    /// # 参数
    /// - `config`: 时间轮配置（已经过验证）
    ///
    /// # 注意
    /// 配置参数已在 WheelConfig::builder().build() 中验证，
    /// 因此此方法不会失败。
    pub fn new(config: WheelConfig) -> Self {
        let slot_count = config.slot_count;
        let mut slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            slots.push(Vec::new());
        }

        Self {
            slots,
            current_tick: 0,
            slot_count,
            tick_duration: config.tick_duration,
            task_index: FxHashMap::default(),
            batch_config: BatchConfig::default(),
        }
    }
    
    /// 创建带批处理配置的时间轮
    ///
    /// # 参数
    /// - `config`: 时间轮配置（已经过验证）
    /// - `batch_config`: 批处理配置
    #[allow(dead_code)]
    pub fn with_batch_config(config: WheelConfig, batch_config: BatchConfig) -> Self {
        let slot_count = config.slot_count;
        let mut slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            slots.push(Vec::new());
        }

        Self {
            slots,
            current_tick: 0,
            slot_count,
            tick_duration: config.tick_duration,
            task_index: FxHashMap::default(),
            batch_config,
        }
    }

    /// 获取当前 tick
    #[allow(dead_code)]
    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    /// 获取 tick 时长
    #[allow(dead_code)]
    pub fn tick_duration(&self) -> Duration {
        self.tick_duration
    }

    /// 获取槽位数量
    #[allow(dead_code)]
    pub fn slot_count(&self) -> usize {
        self.slot_count
    }

    /// 计算延迟对应的 tick 数
    pub fn delay_to_ticks(&self, delay: Duration) -> u64 {
        let ticks = delay.as_millis() as u64 / self.tick_duration.as_millis() as u64;
        ticks.max(1) // 至少 1 个 tick
    }

    /// 插入定时器任务
    ///
    /// # 参数
    /// - `delay`: 延迟时间（从当前 tick 开始计算）
    /// - `task`: 定时器任务
    /// - `notifier`: 完成通知器（用于在任务到期或取消时发送通知）
    ///
    /// # 返回
    /// 任务的唯一标识符（TaskId）
    ///
    /// # 实现细节
    /// - 自动计算任务应该插入的槽位和轮次
    /// - 使用位运算优化槽位索引计算
    /// - 维护任务索引以支持 O(1) 查找和取消
    #[inline]
    pub fn insert(&mut self, delay: Duration, mut task: TimerTask, notifier: crate::task::CompletionNotifier) -> TaskId {
        let ticks = self.delay_to_ticks(delay);
        let total_ticks = self.current_tick + ticks;
        
        // 计算槽位索引和轮次
        let slot_index = (total_ticks as usize) & (self.slot_count - 1);
        
        // 修复：使用 total_ticks 计算轮次，而不是 ticks
        // 轮次 = 任务到期时的轮数 - 当前轮数
        let rounds = (total_ticks / self.slot_count as u64).saturating_sub(self.current_tick / self.slot_count as u64) as u32;

        // 准备任务注册（设置 notifier 和时间轮参数）
        task.prepare_for_registration(notifier, total_ticks, rounds);

        let task_id = task.id;
        
        // 获取任务在 Vec 中的索引位置（插入前的长度就是新任务的索引）
        let vec_index = self.slots[slot_index].len();
        let location = TaskLocation::new(slot_index, vec_index, task_id);

        // 插入任务到槽位
        self.slots[slot_index].push(task);
        
        // 记录任务位置
        self.task_index.insert(task_id, location);

        task_id
    }

    /// 批量插入定时器任务
    ///
    /// # 参数
    /// - `tasks`: (延迟时间, 任务, 完成通知器) 的元组列表
    ///
    /// # 返回
    /// 任务 ID 列表
    ///
    /// # 性能优势
    /// - 减少重复的边界检查和容量调整
    /// - 对于相同延迟的任务，可以复用计算结果
    #[inline]
    pub fn insert_batch(&mut self, tasks: Vec<(Duration, TimerTask, crate::task::CompletionNotifier)>) -> Vec<TaskId> {
        let task_count = tasks.len();
        
        // 优化：预先分配 HashMap 容量，避免重新分配
        self.task_index.reserve(task_count);
        
        let mut task_ids = Vec::with_capacity(task_count);
        
        for (delay, mut task, notifier) in tasks {
            let ticks = self.delay_to_ticks(delay);
            let total_ticks = self.current_tick + ticks;
            
            // 计算槽位索引和轮次
            let slot_index = (total_ticks as usize) & (self.slot_count - 1);
            let rounds = (total_ticks / self.slot_count as u64)
                .saturating_sub(self.current_tick / self.slot_count as u64) as u32;

            // 准备任务注册（设置 notifier 和时间轮参数）
            task.prepare_for_registration(notifier, total_ticks, rounds);

            let task_id = task.id;
            
            // 获取任务在 Vec 中的索引位置
            let vec_index = self.slots[slot_index].len();
            let location = TaskLocation::new(slot_index, vec_index, task_id);

            // 插入任务到槽位
            self.slots[slot_index].push(task);
            
            // 记录任务位置
            self.task_index.insert(task_id, location);
            
            task_ids.push(task_id);
        }
        
        task_ids
    }

    /// 取消定时器任务
    ///
    /// # 参数
    /// - `task_id`: 任务 ID
    ///
    /// # 返回
    /// 如果任务存在且成功取消返回 true，否则返回 false
    #[inline]
    pub fn cancel(&mut self, task_id: TaskId) -> bool {
        if let Some(location) = self.task_index.remove(&task_id) {
            let slot = &mut self.slots[location.slot_index];
            
            // 使用 vec_index 直接访问，O(1) 复杂度
            if location.vec_index < slot.len() && slot[location.vec_index].id == task_id {
                // 先取出 notifier 并发送取消通知
                if let Some(notifier) = slot[location.vec_index].completion_notifier.take() {
                    let _ = notifier.0.send(TaskCompletionReason::Cancelled);
                }
                
                // 使用 swap_remove 移除任务
                slot.swap_remove(location.vec_index);
                
                // 如果被交换的元素不是最后一个，需要更新被交换元素的索引
                if location.vec_index < slot.len() {
                    let swapped_task_id = slot[location.vec_index].id;
                    if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                        swapped_location.vec_index = location.vec_index;
                    }
                }
                
                return true;
            }
        }
        false
    }

    /// 批量取消定时器任务
    ///
    /// # 参数
    /// - `task_ids`: 要取消的任务 ID 列表
    ///
    /// # 返回
    /// 成功取消的任务数量
    ///
    /// # 性能优势
    /// - 减少重复的 HashMap 查找开销
    /// - 对同一槽位的多个取消操作可以批量处理
    /// - 使用不稳定排序提升性能
    /// - 小批量优化：根据配置阈值跳过排序，直接处理
    #[inline]
    pub fn cancel_batch(&mut self, task_ids: &[TaskId]) -> usize {
        let mut cancelled_count = 0;
        
        // 小批量优化：直接逐个取消，避免分组和排序的开销
        if task_ids.len() <= self.batch_config.small_batch_threshold {
            for &task_id in task_ids {
                if self.cancel(task_id) {
                    cancelled_count += 1;
                }
            }
            return cancelled_count;
        }
        
        // 按槽位分组以优化批量取消
        // 使用 SmallVec 避免大多数情况下的堆分配
        let mut tasks_by_slot: Vec<SmallVec<[(TaskId, usize); 4]>> = 
            vec![SmallVec::new(); self.slot_count];
        
        // 收集需要取消的任务信息
        for &task_id in task_ids {
            if let Some(location) = self.task_index.get(&task_id) {
                tasks_by_slot[location.slot_index].push((task_id, location.vec_index));
            }
        }
        
        // 对每个槽位进行批量处理
        for (slot_index, tasks) in tasks_by_slot.iter_mut().enumerate() {
            if tasks.is_empty() {
                continue;
            }
            
            // 按 vec_index 降序排序，从后往前删除避免索引失效
            // 使用不稳定排序提升性能
            tasks.sort_unstable_by(|a, b| b.1.cmp(&a.1));
            
            let slot = &mut self.slots[slot_index];
            
            for &(task_id, vec_index) in tasks.iter() {
                // 验证任务仍在预期位置
                if vec_index < slot.len() && slot[vec_index].id == task_id {
                    // 先取出 notifier 并发送取消通知
                    if let Some(notifier) = slot[vec_index].completion_notifier.take() {
                        let _ = notifier.0.send(TaskCompletionReason::Cancelled);
                    }
                    
                    // 使用 swap_remove 移除任务
                    slot.swap_remove(vec_index);
                    
                    // 更新被交换元素的索引
                    if vec_index < slot.len() {
                        let swapped_task_id = slot[vec_index].id;
                        if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                            swapped_location.vec_index = vec_index;
                        }
                    }
                    
                    // 从索引中移除
                    self.task_index.remove(&task_id);
                    cancelled_count += 1;
                }
            }
        }
        
        cancelled_count
    }

    /// 推进时间轮一个 tick，返回所有到期的任务
    ///
    /// # 返回
    /// 到期的任务列表（rounds 为 0 的任务）
    ///
    /// # 实现细节
    /// - 将 current_tick 递增 1
    /// - 检查当前 tick 对应的槽位
    /// - 对于 rounds > 0 的任务，递减 rounds 并保留在槽位中
    /// - 对于 rounds = 0 的任务，从槽位中移除并返回
    /// - 使用 swap_remove 优化删除性能
    pub fn advance(&mut self) -> Vec<TimerTask> {
        self.current_tick += 1;
        let slot_index = (self.current_tick as usize) & (self.slot_count - 1);

        // 直接获取槽位的可变引用，避免内存交换
        let slot = &mut self.slots[slot_index];
        
        // 预分配容量以减少重新分配
        let mut expired_tasks = Vec::new();
        
        // 使用反向迭代 + swap_remove 避免频繁移动元素
        let mut i = 0;
        while i < slot.len() {
            let task = &mut slot[i];
            
            if task.rounds > 0 {
                // 还有轮次，减少轮次，任务保持在原位
                task.rounds -= 1;
                // 更新索引中的位置（可能因为之前的移除而改变）
                if let Some(location) = self.task_index.get_mut(&task.id) {
                    location.vec_index = i;
                }
                i += 1;
            } else {
                // 任务已到期，从索引中移除
                self.task_index.remove(&task.id);
                
                // 使用 swap_remove 移除任务（O(1) 操作）
                let expired_task = slot.swap_remove(i);
                
                // 如果 swap 发生了（即不是最后一个元素），更新被交换元素的索引
                if i < slot.len() {
                    let swapped_task_id = slot[i].id;
                    if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                        swapped_location.vec_index = i;
                    }
                }
                
                expired_tasks.push(expired_task);
                // 不增加 i，因为 swap_remove 将后面的元素移到了当前位置
            }
        }

        expired_tasks
    }

    /// 检查时间轮是否为空
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.task_index.is_empty()
    }

    /// 推迟定时器任务（保持原 TaskId）
    ///
    /// # 参数
    /// - `task_id`: 要推迟的任务 ID
    /// - `new_delay`: 新的延迟时间（从当前 tick 重新开始计算，而非从原定延迟时间继续）
    /// - `new_callback`: 新的回调函数（如果为 None 则保持原回调）
    ///
    /// # 返回
    /// 如果任务存在且成功推迟返回 true，否则返回 false
    ///
    /// # 实现细节
    /// - 从原槽位移除任务，保留其 completion_notifier（不会触发取消通知）
    /// - 更新延迟时间和回调函数（如果提供）
    /// - 根据 new_delay 和 current_tick 重新计算目标槽位和轮次
    /// - 使用原 TaskId 重新插入到新的槽位
    /// - 保持与外部持有的 TaskId 引用一致
    #[inline]
    pub fn postpone(
        &mut self,
        task_id: TaskId,
        new_delay: Duration,
        new_callback: Option<crate::task::CallbackWrapper>,
    ) -> bool {
        // 步骤1: 查找并移除原任务
        if let Some(location) = self.task_index.remove(&task_id) {
            let slot = &mut self.slots[location.slot_index];
            
            // 验证任务仍在预期位置
            if location.vec_index < slot.len() && slot[location.vec_index].id == task_id {
                // 使用 swap_remove 移除任务
                let mut task = slot.swap_remove(location.vec_index);
                
                // 更新被交换元素的索引（如果发生了交换）
                if location.vec_index < slot.len() {
                    let swapped_task_id = slot[location.vec_index].id;
                    if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                        swapped_location.vec_index = location.vec_index;
                    }
                }
                
                // 步骤2: 更新任务的延迟和回调
                task.delay = new_delay;
                if let Some(callback) = new_callback {
                    task.callback = Some(callback);
                }
                
                // 步骤3: 根据新延迟重新计算槽位和轮次
                let ticks = self.delay_to_ticks(new_delay);
                let total_ticks = self.current_tick + ticks;
                let new_slot_index = (total_ticks as usize) & (self.slot_count - 1);
                let new_rounds = (total_ticks / self.slot_count as u64)
                    .saturating_sub(self.current_tick / self.slot_count as u64) as u32;
                
                // 更新任务的时间轮参数
                task.deadline_tick = total_ticks;
                task.rounds = new_rounds;
                
                // 步骤4: 重新插入任务到新槽位
                let new_vec_index = self.slots[new_slot_index].len();
                let new_location = TaskLocation::new(new_slot_index, new_vec_index, task_id);
                
                self.slots[new_slot_index].push(task);
                self.task_index.insert(task_id, new_location);
                
                return true;
            }
        }
        false
    }

    /// 批量推迟定时器任务
    ///
    /// # 参数
    /// - `updates`: (任务ID, 新延迟) 的元组列表
    ///
    /// # 返回
    /// 成功推迟的任务数量
    ///
    /// # 性能优势
    /// - 批量处理减少函数调用开销
    /// - 所有延迟时间都从调用时的 current_tick 重新计算
    ///
    /// # 注意
    /// - 如果某个任务 ID 不存在，该任务会被跳过，不影响其他任务的推迟
    #[inline]
    pub fn postpone_batch(
        &mut self,
        updates: Vec<(TaskId, Duration)>,
    ) -> usize {
        let mut postponed_count = 0;
        
        for (task_id, new_delay) in updates {
            if self.postpone(task_id, new_delay, None) {
                postponed_count += 1;
            }
        }
        
        postponed_count
    }

    /// 批量推迟定时器任务（替换回调）
    ///
    /// # 参数
    /// - `updates`: (任务ID, 新延迟, 新回调) 的元组列表
    ///
    /// # 返回
    /// 成功推迟的任务数量
    pub fn postpone_batch_with_callbacks(
        &mut self,
        updates: Vec<(TaskId, Duration, Option<crate::task::CallbackWrapper>)>,
    ) -> usize {
        let mut postponed_count = 0;
        
        for (task_id, new_delay, new_callback) in updates {
            if self.postpone(task_id, new_delay, new_callback) {
                postponed_count += 1;
            }
        }
        
        postponed_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::CallbackWrapper;

    #[test]
    fn test_wheel_creation() {
        let wheel = Wheel::new(WheelConfig::default());
        assert_eq!(wheel.slot_count(), 512);
        assert_eq!(wheel.current_tick(), 0);
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_delay_to_ticks() {
        let wheel = Wheel::new(WheelConfig::default());
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(100)), 10);
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(50)), 5);
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(1)), 1); // 最小 1 tick
    }

    #[test]
    fn test_wheel_invalid_slot_count() {
        let result = WheelConfig::builder()
            .slot_count(100)
            .build();
        assert!(result.is_err());
        if let Err(crate::error::TimerError::InvalidSlotCount { slot_count, reason }) = result {
            assert_eq!(slot_count, 100);
            assert_eq!(reason, "槽位数量必须是 2 的幂次方");
        } else {
            panic!("Expected InvalidSlotCount error");
        }
    }

    #[test]
    fn test_insert_batch() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 创建批量任务
        let tasks: Vec<(Duration, TimerTask, CompletionNotifier)> = (0..10)
            .map(|i| {
                let callback = CallbackWrapper::new(|| async {});
                let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
                let notifier = CompletionNotifier(completion_tx);
                let task = TimerTask::new(Duration::from_millis(100 + i * 10), Some(callback));
                (Duration::from_millis(100 + i * 10), task, notifier)
            })
            .collect();
        
        let task_ids = wheel.insert_batch(tasks);
        
        assert_eq!(task_ids.len(), 10);
        assert!(!wheel.is_empty());
    }

    #[test]
    fn test_cancel_batch() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入多个任务
        let mut task_ids = Vec::new();
        for i in 0..10 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(100 + i * 10), Some(callback));
            let task_id = wheel.insert(Duration::from_millis(100 + i * 10), task, notifier);
            task_ids.push(task_id);
        }
        
        assert_eq!(task_ids.len(), 10);
        
        // 批量取消前 5 个任务
        let to_cancel = &task_ids[0..5];
        let cancelled_count = wheel.cancel_batch(to_cancel);
        
        assert_eq!(cancelled_count, 5);
        
        // 尝试再次取消相同的任务，应该返回 0
        let cancelled_again = wheel.cancel_batch(to_cancel);
        assert_eq!(cancelled_again, 0);
        
        // 取消剩余的任务
        let remaining = &task_ids[5..10];
        let cancelled_remaining = wheel.cancel_batch(remaining);
        assert_eq!(cancelled_remaining, 5);
        
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_batch_operations_same_slot() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入多个相同延迟的任务（会进入同一个槽位）
        let mut task_ids = Vec::new();
        for _ in 0..20 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(100), Some(callback));
            let task_id = wheel.insert(Duration::from_millis(100), task, notifier);
            task_ids.push(task_id);
        }
        
        // 批量取消所有任务
        let cancelled_count = wheel.cancel_batch(&task_ids);
        assert_eq!(cancelled_count, 20);
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_postpone_single_task() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入任务，延迟 100ms
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(Duration::from_millis(100), task, notifier);
        
        // 推迟任务到 200ms（保持原回调）
        let postponed = wheel.postpone(task_id, Duration::from_millis(200), None);
        assert!(postponed);
        
        // 验证任务仍在时间轮中
        assert!(!wheel.is_empty());
        
        // 推进 100ms（10 ticks），任务不应该触发
        for _ in 0..10 {
            let expired = wheel.advance();
            assert!(expired.is_empty());
        }
        
        // 再推进 100ms（10 ticks），任务应该触发
        let mut triggered = false;
        for _ in 0..10 {
            let expired = wheel.advance();
            if !expired.is_empty() {
                assert_eq!(expired.len(), 1);
                assert_eq!(expired[0].id, task_id);
                triggered = true;
                break;
            }
        }
        assert!(triggered);
    }

    #[test]
    fn test_postpone_with_new_callback() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入任务，带原始回调
        let old_callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(old_callback.clone()));
        let task_id = wheel.insert(Duration::from_millis(100), task, notifier);
        
        // 推迟任务并替换回调
        let new_callback = CallbackWrapper::new(|| async {});
        let postponed = wheel.postpone(task_id, Duration::from_millis(50), Some(new_callback));
        assert!(postponed);
        
        // 推进 50ms（5 ticks），任务应该触发
        // 注意：任务在第 5 个 tick 触发（current_tick 从 0 推进到 5）
        let mut triggered = false;
        for i in 0..5 {
            let expired = wheel.advance();
            if !expired.is_empty() {
                assert_eq!(expired.len(), 1, "第 {} 次推进时应该有 1 个任务触发", i + 1);
                assert_eq!(expired[0].id, task_id);
                triggered = true;
                break;
            }
        }
        assert!(triggered, "任务应该在 5 个 tick 内触发");
    }

    #[test]
    fn test_postpone_nonexistent_task() {
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 尝试推迟不存在的任务
        let fake_task_id = TaskId::new();
        let postponed = wheel.postpone(fake_task_id, Duration::from_millis(100), None);
        assert!(!postponed);
    }

    #[test]
    fn test_postpone_batch() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入 5 个任务，延迟 50ms（5 ticks）
        let mut task_ids = Vec::new();
        for _ in 0..5 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(50), Some(callback));
            let task_id = wheel.insert(Duration::from_millis(50), task, notifier);
            task_ids.push(task_id);
        }
        
        // 批量推迟所有任务到 150ms（15 ticks）
        let updates: Vec<_> = task_ids
            .iter()
            .map(|&id| (id, Duration::from_millis(150)))
            .collect();
        let postponed_count = wheel.postpone_batch(updates);
        assert_eq!(postponed_count, 5);
        
        // 推进 5 ticks（50ms），任务不应该触发
        for _ in 0..5 {
            let expired = wheel.advance();
            assert!(expired.is_empty(), "前 5 个 tick 不应该有任务触发");
        }
        
        // 继续推进 10 ticks（从 tick 5 到 tick 15），所有任务应该在第 15 个 tick 触发
        let mut total_triggered = 0;
        for _ in 0..10 {
            let expired = wheel.advance();
            total_triggered += expired.len();
        }
        assert_eq!(total_triggered, 5, "应该有 5 个任务在推进到 tick 15 时触发");
    }

    #[test]
    fn test_postpone_batch_partial() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入 10 个任务，延迟 50ms（5 ticks）
        let mut task_ids = Vec::new();
        for _ in 0..10 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(50), Some(callback));
            let task_id = wheel.insert(Duration::from_millis(50), task, notifier);
            task_ids.push(task_id);
        }
        
        // 只推迟前 5 个任务到 150ms，包含一个不存在的任务
        let fake_task_id = TaskId::new();
        let mut updates: Vec<_> = task_ids[0..5]
            .iter()
            .map(|&id| (id, Duration::from_millis(150)))
            .collect();
        updates.push((fake_task_id, Duration::from_millis(150)));
        
        let postponed_count = wheel.postpone_batch(updates);
        assert_eq!(postponed_count, 5, "应该有 5 个任务成功推迟（fake_task_id 失败）");
        
        // 推进 5 ticks（50ms），后 5 个未推迟的任务应该触发
        let mut triggered_at_50ms = 0;
        for _ in 0..5 {
            let expired = wheel.advance();
            triggered_at_50ms += expired.len();
        }
        assert_eq!(triggered_at_50ms, 5, "应该有 5 个未推迟的任务在 tick 5 触发");
        
        // 继续推进 10 ticks（从 tick 5 到 tick 15），前 5 个推迟的任务应该触发
        let mut triggered_at_150ms = 0;
        for _ in 0..10 {
            let expired = wheel.advance();
            triggered_at_150ms += expired.len();
        }
        assert_eq!(triggered_at_150ms, 5, "应该有 5 个推迟的任务在 tick 15 触发");
    }

    #[test]
    fn test_multi_round_tasks() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入一个超过一圈的任务（512 slots * 10ms = 5120ms）
        // 延迟 6000ms 需要跨越多个轮次
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(6000), Some(callback));
        let task_id = wheel.insert(Duration::from_millis(6000), task, notifier);
        
        // 6000ms / 10ms = 600 ticks
        // 600 ticks / 512 slots = 1 轮 + 88 ticks
        // 所以任务应该在 slot 88，rounds = 1
        
        // 推进 512 ticks（完成第一轮），任务不应该触发
        for _ in 0..512 {
            let expired = wheel.advance();
            assert!(expired.is_empty(), "第一轮不应该有任务触发");
        }
        
        // 继续推进 88 ticks，任务应该触发
        let mut triggered = false;
        for _ in 0..88 {
            let expired = wheel.advance();
            if !expired.is_empty() {
                assert_eq!(expired.len(), 1);
                assert_eq!(expired[0].id, task_id);
                triggered = true;
                break;
            }
        }
        assert!(triggered, "任务应该在第二轮的第 88 个 tick 触发");
    }

    #[test]
    fn test_minimum_delay() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 测试最小延迟（小于 1 tick 的延迟应该向上取整为 1 tick）
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(1), Some(callback));
        let task_id = wheel.insert(Duration::from_millis(1), task, notifier);
        
        // 推进 1 tick，任务应该触发
        let expired = wheel.advance();
        assert_eq!(expired.len(), 1, "最小延迟任务应该在 1 tick 后触发");
        assert_eq!(expired[0].id, task_id);
    }

    #[test]
    fn test_empty_batch_operations() {
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 测试空批量插入
        let task_ids = wheel.insert_batch(vec![]);
        assert_eq!(task_ids.len(), 0);
        
        // 测试空批量取消
        let cancelled = wheel.cancel_batch(&[]);
        assert_eq!(cancelled, 0);
        
        // 测试空批量推迟
        let postponed = wheel.postpone_batch(vec![]);
        assert_eq!(postponed, 0);
    }

    #[test]
    fn test_postpone_same_task_multiple_times() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入任务
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(Duration::from_millis(100), task, notifier);
        
        // 第一次推迟
        let postponed = wheel.postpone(task_id, Duration::from_millis(200), None);
        assert!(postponed, "第一次推迟应该成功");
        
        // 第二次推迟
        let postponed = wheel.postpone(task_id, Duration::from_millis(300), None);
        assert!(postponed, "第二次推迟应该成功");
        
        // 第三次推迟
        let postponed = wheel.postpone(task_id, Duration::from_millis(50), None);
        assert!(postponed, "第三次推迟应该成功");
        
        // 验证任务在最后一次推迟的时间触发（50ms = 5 ticks）
        let mut triggered = false;
        for _ in 0..5 {
            let expired = wheel.advance();
            if !expired.is_empty() {
                assert_eq!(expired.len(), 1);
                assert_eq!(expired[0].id, task_id);
                triggered = true;
                break;
            }
        }
        assert!(triggered, "任务应该在最后一次推迟的时间触发");
    }

    #[test]
    fn test_advance_empty_slots() {
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 不插入任何任务，推进多个 tick
        for _ in 0..100 {
            let expired = wheel.advance();
            assert!(expired.is_empty(), "空槽位不应该返回任何任务");
        }
        
        assert_eq!(wheel.current_tick(), 100, "current_tick 应该正确递增");
    }

    #[test]
    fn test_cancel_after_postpone() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入任务
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(Duration::from_millis(100), task, notifier);
        
        // 推迟任务
        let postponed = wheel.postpone(task_id, Duration::from_millis(200), None);
        assert!(postponed, "推迟应该成功");
        
        // 取消推迟后的任务
        let cancelled = wheel.cancel(task_id);
        assert!(cancelled, "取消应该成功");
        
        // 推进到原定时间，任务不应该触发
        for _ in 0..20 {
            let expired = wheel.advance();
            assert!(expired.is_empty(), "已取消的任务不应该触发");
        }
        
        assert!(wheel.is_empty(), "时间轮应该为空");
    }

    #[test]
    fn test_slot_boundary() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 测试槽位边界和环绕
        // 第一个任务：延迟 10ms（1 tick），应该在 slot 1 触发
        let callback1 = CallbackWrapper::new(|| async {});
        let (tx1, _rx1) = tokio::sync::oneshot::channel();
        let task1 = TimerTask::new(Duration::from_millis(10), Some(callback1));
        let task_id_1 = wheel.insert(Duration::from_millis(10), task1, CompletionNotifier(tx1));
        
        // 第二个任务：延迟 5110ms（511 ticks），应该在 slot 511 触发
        let callback2 = CallbackWrapper::new(|| async {});
        let (tx2, _rx2) = tokio::sync::oneshot::channel();
        let task2 = TimerTask::new(Duration::from_millis(5110), Some(callback2));
        let task_id_2 = wheel.insert(Duration::from_millis(5110), task2, CompletionNotifier(tx2));
        
        // 推进 1 tick，第一个任务应该触发
        let expired = wheel.advance();
        assert_eq!(expired.len(), 1, "第一个任务应该在 tick 1 触发");
        assert_eq!(expired[0].id, task_id_1);
        
        // 继续推进到 511 ticks（从 tick 1 到 tick 511），第二个任务应该触发
        let mut triggered = false;
        for i in 0..510 {
            let expired = wheel.advance();
            if !expired.is_empty() {
                assert_eq!(expired.len(), 1, "第 {} 次推进应该触发第二个任务", i + 2);
                assert_eq!(expired[0].id, task_id_2);
                triggered = true;
                break;
            }
        }
        assert!(triggered, "第二个任务应该在 tick 511 触发");
        
        assert!(wheel.is_empty(), "所有任务都应该已经触发");
    }

    #[test]
    fn test_batch_cancel_small_threshold() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        // 测试小批量阈值优化
        let batch_config = BatchConfig {
            small_batch_threshold: 5,
        };
        let mut wheel = Wheel::with_batch_config(WheelConfig::default(), batch_config);
        
        // 插入 10 个任务
        let mut task_ids = Vec::new();
        for _ in 0..10 {
            let callback = CallbackWrapper::new(|| async {});
            let (tx, _rx) = tokio::sync::oneshot::channel();
            let task = TimerTask::new(Duration::from_millis(100), Some(callback));
            let task_id = wheel.insert(Duration::from_millis(100), task, CompletionNotifier(tx));
            task_ids.push(task_id);
        }
        
        // 小批量取消（应该使用直接取消路径）
        let cancelled = wheel.cancel_batch(&task_ids[0..3]);
        assert_eq!(cancelled, 3);
        
        // 大批量取消（应该使用分组优化路径）
        let cancelled = wheel.cancel_batch(&task_ids[3..10]);
        assert_eq!(cancelled, 7);
        
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_task_id_uniqueness() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入多个任务，验证 TaskId 唯一性
        let mut task_ids = std::collections::HashSet::new();
        for _ in 0..100 {
            let callback = CallbackWrapper::new(|| async {});
            let (tx, _rx) = tokio::sync::oneshot::channel();
            let task = TimerTask::new(Duration::from_millis(100), Some(callback));
            let task_id = wheel.insert(Duration::from_millis(100), task, CompletionNotifier(tx));
            
            assert!(task_ids.insert(task_id), "TaskId 应该是唯一的");
        }
        
        assert_eq!(task_ids.len(), 100);
    }
}

