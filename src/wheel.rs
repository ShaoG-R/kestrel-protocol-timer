use crate::config::{BatchConfig, WheelConfig};
use crate::task::{TaskId, TaskLocation, TimerTask};
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
    /// - `delay`: 延迟时间
    /// - `task`: 定时器任务
    /// - `notifier`: 完成通知器
    ///
    /// # 返回
    /// 任务 ID

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
    pub fn insert_batch(&mut self, tasks: Vec<(Duration, TimerTask, crate::task::CompletionNotifier)>) -> Vec<TaskId> {
        let mut task_ids = Vec::with_capacity(tasks.len());
        
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
    pub fn cancel(&mut self, task_id: TaskId) -> bool {
        if let Some(location) = self.task_index.remove(&task_id) {
            let slot = &mut self.slots[location.slot_index];
            
            // 使用 vec_index 直接访问，O(1) 复杂度
            if location.vec_index < slot.len() && slot[location.vec_index].id == task_id {
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

    /// 推进时间轮，返回所有到期的任务
    ///
    /// # 返回
    /// 到期的任务列表
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        use std::sync::Arc;
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 创建批量任务
        let tasks: Vec<(Duration, TimerTask, CompletionNotifier)> = (0..10)
            .map(|i| {
                let callback = Arc::new(|| async {}) as Arc<dyn crate::task::TimerCallback>;
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
        use std::sync::Arc;
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入多个任务
        let mut task_ids = Vec::new();
        for i in 0..10 {
            let callback = Arc::new(|| async {}) as Arc<dyn crate::task::TimerCallback>;
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
        use std::sync::Arc;
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default());
        
        // 插入多个相同延迟的任务（会进入同一个槽位）
        let mut task_ids = Vec::new();
        for _ in 0..20 {
            let callback = Arc::new(|| async {}) as Arc<dyn crate::task::TimerCallback>;
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
}

