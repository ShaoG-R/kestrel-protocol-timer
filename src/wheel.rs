use crate::error::TimerError;
use crate::task::{TaskId, TaskLocation, TimerTask};
use rustc_hash::FxHashMap;
use std::time::Duration;

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
}

impl Wheel {
    /// 创建新的时间轮
    ///
    /// # 参数
    /// - `tick_duration`: 每个 tick 的时间长度
    /// - `slot_count`: 槽位数量（必须是 2 的幂次方以优化取模运算）
    ///
    /// # 返回
    /// - `Ok(Self)`: 成功创建时间轮
    /// - `Err(TimerError)`: 槽位数量无效
    pub fn new(tick_duration: Duration, slot_count: usize) -> Result<Self, TimerError> {
        if slot_count == 0 {
            return Err(TimerError::InvalidSlotCount {
                slot_count,
                reason: "槽位数量必须大于 0",
            });
        }
        
        if !slot_count.is_power_of_two() {
            return Err(TimerError::InvalidSlotCount {
                slot_count,
                reason: "槽位数量必须是 2 的幂次方",
            });
        }

        let mut slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            slots.push(Vec::new());
        }

        Ok(Self {
            slots,
            current_tick: 0,
            slot_count,
            tick_duration,
            task_index: FxHashMap::default(),
        })
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
    ///
    /// # 返回
    /// 任务 ID
    pub fn insert(&mut self, delay: Duration, mut task: TimerTask) -> TaskId {
        let ticks = self.delay_to_ticks(delay);
        let total_ticks = self.current_tick + ticks;
        
        // 计算槽位索引和轮次
        let slot_index = (total_ticks as usize) & (self.slot_count - 1);
        let rounds = (ticks / self.slot_count as u64) as u32;

        task.deadline_tick = total_ticks;
        task.rounds = rounds;

        let task_id = task.id;
        let location = TaskLocation::new(slot_index, task_id);

        // 插入任务到槽位
        self.slots[slot_index].push(task);
        
        // 记录任务位置
        self.task_index.insert(task_id, location);

        task_id
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
            if let Some(pos) = slot.iter().position(|t| t.id == task_id) {
                slot.swap_remove(pos);
                return true;
            }
        }
        false
    }

    /// 推进时间轮，返回所有到期的任务
    ///
    /// # 返回
    /// 到期的任务列表
    pub fn advance(&mut self) -> Vec<TimerTask> {
        self.current_tick += 1;
        let slot_index = (self.current_tick as usize) & (self.slot_count - 1);

        // 取出当前槽位的所有任务
        let mut tasks = std::mem::take(&mut self.slots[slot_index]);
        let mut expired_tasks = Vec::new();
        let mut pending_tasks = Vec::new();

        for mut task in tasks.drain(..) {
            // 从索引中移除
            self.task_index.remove(&task.id);

            if task.rounds > 0 {
                // 还有轮次，减少轮次并重新插入
                task.rounds -= 1;
                let task_id = task.id;
                let location = TaskLocation::new(slot_index, task_id);
                self.task_index.insert(task_id, location);
                pending_tasks.push(task);
            } else {
                // 已到期
                expired_tasks.push(task);
            }
        }

        // 将未到期的任务放回槽位
        self.slots[slot_index] = pending_tasks;

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
        let wheel = Wheel::new(Duration::from_millis(10), 512).unwrap();
        assert_eq!(wheel.slot_count(), 512);
        assert_eq!(wheel.current_tick(), 0);
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_delay_to_ticks() {
        let wheel = Wheel::new(Duration::from_millis(10), 512).unwrap();
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(100)), 10);
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(50)), 5);
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(1)), 1); // 最小 1 tick
    }

    #[test]
    fn test_wheel_invalid_slot_count() {
        let result = Wheel::new(Duration::from_millis(10), 100);
        assert!(result.is_err());
        if let Err(TimerError::InvalidSlotCount { slot_count, reason }) = result {
            assert_eq!(slot_count, 100);
            assert_eq!(reason, "槽位数量必须是 2 的幂次方");
        } else {
            panic!("Expected InvalidSlotCount error");
        }
    }
}

