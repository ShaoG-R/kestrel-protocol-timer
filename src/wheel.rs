use crate::config::{BatchConfig, WheelConfig};
use crate::task::{TaskCompletionReason, TaskId, TaskLocation, TimerTask};
use rustc_hash::FxHashMap;
use std::time::Duration;
use smallvec::SmallVec;

/// 时间轮单层数据结构
struct WheelLayer {
    /// 槽位数组，每个槽位存储一组定时器任务
    slots: Vec<Vec<TimerTask>>,
    
    /// 当前时间指针（tick 索引）
    current_tick: u64,
    
    /// 槽位数量
    slot_count: usize,
    
    /// 每个 tick 的时间长度
    tick_duration: Duration,
    
    /// 缓存：tick 时长（毫秒，u64）- 避免重复转换
    tick_duration_ms: u64,
    
    /// 缓存：槽位掩码（slot_count - 1）- 用于快速取模
    slot_mask: usize,
}

impl WheelLayer {
    fn new(slot_count: usize, tick_duration: Duration) -> Self {
        let mut slots = Vec::with_capacity(slot_count);
        // 为每个槽位预分配容量，减少后续 push 时的重新分配
        // 大多数槽位通常包含 0-4 个任务，预分配 4 个容量
        for _ in 0..slot_count {
            slots.push(Vec::with_capacity(4));
        }
        
        let tick_duration_ms = tick_duration.as_millis() as u64;
        let slot_mask = slot_count - 1;
        
        Self {
            slots,
            current_tick: 0,
            slot_count,
            tick_duration,
            tick_duration_ms,
            slot_mask,
        }
    }
    
    /// 计算延迟对应的 tick 数
    fn delay_to_ticks(&self, delay: Duration) -> u64 {
        let ticks = delay.as_millis() as u64 / self.tick_duration.as_millis() as u64;
        ticks.max(1) // 至少 1 个 tick
    }
}

/// 时间轮数据结构（分层模式）
pub struct Wheel {
    /// L0 层（底层）
    l0: WheelLayer,
    
    /// L1 层（高层）
    l1: WheelLayer,
    
    /// L1 tick 相对于 L0 tick 的比率
    l1_tick_ratio: u64,
    
    /// 任务索引，用于快速查找和取消任务
    task_index: FxHashMap<TaskId, TaskLocation>,
    
    /// 批处理配置
    batch_config: BatchConfig,
    
    /// 缓存：L0 层容量（毫秒）- 避免重复计算
    l0_capacity_ms: u64,
    
    /// 缓存：L1 层容量（tick 数）- 避免重复计算
    l1_capacity_ticks: u64,
}

impl Wheel {
    /// 创建新的时间轮
    ///
    /// # 参数
    /// - `config`: 时间轮配置（已经过验证）
    /// - `batch_config`: 批处理配置
    ///
    /// # 注意
    /// 配置参数已在 WheelConfig::builder().build() 中验证，
    /// 因此此方法不会失败。
    pub fn new(config: WheelConfig, batch_config: BatchConfig) -> Self {
        let h_config = config.hierarchical;
        let l0 = WheelLayer::new(h_config.l0_slot_count, h_config.l0_tick_duration);
        let l1 = WheelLayer::new(h_config.l1_slot_count, h_config.l1_tick_duration);
        
        // 计算 L1 tick 相对于 L0 tick 的比率
        let l1_tick_ratio = l1.tick_duration_ms / l0.tick_duration_ms;
        
        // 预计算容量，避免在 insert 中重复计算
        let l0_capacity_ms = (l0.slot_count as u64) * l0.tick_duration_ms;
        let l1_capacity_ticks = l1.slot_count as u64;
        
        Self {
            l0,
            l1,
            l1_tick_ratio,
            task_index: FxHashMap::default(),
            batch_config,
            l0_capacity_ms,
            l1_capacity_ticks,
        }
    }

    /// 获取当前 tick（L0 层的 tick）
    #[allow(dead_code)]
    pub fn current_tick(&self) -> u64 {
        self.l0.current_tick
    }

    /// 获取 tick 时长（L0 层的 tick 时长）
    #[allow(dead_code)]
    pub fn tick_duration(&self) -> Duration {
        self.l0.tick_duration
    }

    /// 获取槽位数量（L0 层的槽位数量）
    #[allow(dead_code)]
    pub fn slot_count(&self) -> usize {
        self.l0.slot_count
    }

    /// 计算延迟对应的 tick 数（基于 L0 层）
    #[allow(dead_code)]
    pub fn delay_to_ticks(&self, delay: Duration) -> u64 {
        self.l0.delay_to_ticks(delay)
    }
    
    /// 判断延迟应该插入到哪一层
    /// 
    /// 返回：(层级, tick数, rounds)
    /// - 层级：0 = L0, 1 = L1
    /// - tick数：从当前 tick 开始计算的 tick 数
    /// - rounds：轮次（仅在 L1 层超长延迟时使用）
    #[inline(always)]
    fn determine_layer(&self, delay: Duration) -> (u8, u64, u32) {
        let delay_ms = delay.as_millis() as u64;
        
        // 快速路径：大多数任务在 L0 范围内（使用缓存的容量）
        if delay_ms < self.l0_capacity_ms {
            let l0_ticks = (delay_ms / self.l0.tick_duration_ms).max(1);
            return (0, l0_ticks, 0);
        }
        
        // 慢速路径：L1 层任务（使用缓存的值）
        let l1_ticks = (delay_ms / self.l1.tick_duration_ms).max(1);
        
        if l1_ticks < self.l1_capacity_ticks {
            (1, l1_ticks, 0)
        } else {
            let rounds = (l1_ticks / self.l1_capacity_ticks) as u32;
            (1, l1_ticks, rounds)
        }
    }

    /// 插入定时器任务
    ///
    /// # 参数
    /// - `task`: 定时器任务
    /// - `notifier`: 完成通知器（用于在任务到期或取消时发送通知）
    ///
    /// # 返回
    /// 任务的唯一标识符（TaskId）
    ///
    /// # 实现细节
    /// - 自动计算任务应该插入的层级和槽位
    /// - 分层模式：短延迟任务插入 L0，长延迟任务插入 L1
    /// - 使用位运算优化槽位索引计算
    /// - 维护任务索引以支持 O(1) 查找和取消
    #[inline]
    pub fn insert(&mut self, mut task: TimerTask, notifier: crate::task::CompletionNotifier) -> TaskId {
        let (level, ticks, rounds) = self.determine_layer(task.delay);
        
        // 使用 match 减少分支，并使用缓存的槽位掩码
        let (current_tick, slot_mask, slots) = match level {
            0 => (self.l0.current_tick, self.l0.slot_mask, &mut self.l0.slots),
            _ => (self.l1.current_tick, self.l1.slot_mask, &mut self.l1.slots),
        };
        
        let total_ticks = current_tick + ticks;
        let slot_index = (total_ticks as usize) & slot_mask;

        // 准备任务注册（设置 notifier 和时间轮参数）
        task.prepare_for_registration(notifier, total_ticks, rounds);

        let task_id = task.id;
        
        // 获取任务在 Vec 中的索引位置（插入前的长度就是新任务的索引）
        let vec_index = slots[slot_index].len();
        let location = TaskLocation::new(level, slot_index, vec_index);

        // 插入任务到槽位
        slots[slot_index].push(task);
        
        // 记录任务位置
        self.task_index.insert(task_id, location);

        task_id
    }

    /// 批量插入定时器任务
    ///
    /// # 参数
    /// - `tasks`: (任务, 完成通知器) 的元组列表
    ///
    /// # 返回
    /// 任务 ID 列表
    ///
    /// # 性能优势
    /// - 减少重复的边界检查和容量调整
    /// - 对于相同延迟的任务，可以复用计算结果
    #[inline]
    pub fn insert_batch(&mut self, tasks: Vec<(TimerTask, crate::task::CompletionNotifier)>) -> Vec<TaskId> {
        let task_count = tasks.len();
        
        // 优化：预先分配 HashMap 容量，避免重新分配
        self.task_index.reserve(task_count);
        
        let mut task_ids = Vec::with_capacity(task_count);
        
        for (mut task, notifier) in tasks {
            let (level, ticks, rounds) = self.determine_layer(task.delay);
            
            // 使用 match 减少分支，并使用缓存的槽位掩码
            let (current_tick, slot_mask, slots) = match level {
                0 => (self.l0.current_tick, self.l0.slot_mask, &mut self.l0.slots),
                _ => (self.l1.current_tick, self.l1.slot_mask, &mut self.l1.slots),
            };
            
            let total_ticks = current_tick + ticks;
            let slot_index = (total_ticks as usize) & slot_mask;

            // 准备任务注册（设置 notifier 和时间轮参数）
            task.prepare_for_registration(notifier, total_ticks, rounds);

            let task_id = task.id;
            
            // 获取任务在 Vec 中的索引位置
            let vec_index = slots[slot_index].len();
            let location = TaskLocation::new(level, slot_index, vec_index);

            // 插入任务到槽位
            slots[slot_index].push(task);
            
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
        // 从索引中移除任务位置
        let location = match self.task_index.remove(&task_id) {
            Some(loc) => loc,
            None => return false,
        };
        
        // 使用 match 获取槽位引用，减少分支
        let slot = match location.level {
            0 => &mut self.l0.slots[location.slot_index],
            _ => &mut self.l1.slots[location.slot_index],
        };
        
        // 边界检查和 ID 验证
        if location.vec_index >= slot.len() || slot[location.vec_index].id != task_id {
            // 索引不一致，重新插入 location 以保持数据一致性
            self.task_index.insert(task_id, location);
            return false;
        }
        
        // 发送取消通知
        if let Some(notifier) = slot[location.vec_index].completion_notifier.take() {
            let _ = notifier.0.send(TaskCompletionReason::Cancelled);
        }
        
        // 使用 swap_remove 移除任务，记录被交换的任务 ID
        let removed_task = slot.swap_remove(location.vec_index);
        
        // 如果发生了交换（vec_index 不是最后一个元素）
        if location.vec_index < slot.len() {
            let swapped_task_id = slot[location.vec_index].id;
            // 一次性更新被交换元素的索引，避免再次 HashMap 查询
            if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                swapped_location.vec_index = location.vec_index;
            }
        }
        
        // 确保移除的是正确的任务
        debug_assert_eq!(removed_task.id, task_id);
        true
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
        
        // 按层级和槽位分组以优化批量取消
        // 使用 SmallVec 避免大多数情况下的堆分配
        let l0_slot_count = self.l0.slot_count;
        let l1_slot_count = self.l1.slot_count;
        
        let mut l0_tasks_by_slot: Vec<SmallVec<[(TaskId, usize); 4]>> = 
            vec![SmallVec::new(); l0_slot_count];
        let mut l1_tasks_by_slot: Vec<SmallVec<[(TaskId, usize); 4]>> = 
            vec![SmallVec::new(); l1_slot_count];
        
        // 收集需要取消的任务信息
        for &task_id in task_ids {
            if let Some(location) = self.task_index.get(&task_id) {
                if location.level == 0 {
                    l0_tasks_by_slot[location.slot_index].push((task_id, location.vec_index));
                } else {
                    l1_tasks_by_slot[location.slot_index].push((task_id, location.vec_index));
                }
            }
        }
        
        // 处理 L0 层的取消
        for (slot_index, tasks) in l0_tasks_by_slot.iter_mut().enumerate() {
            if tasks.is_empty() {
                continue;
            }
            
            // 按 vec_index 降序排序，从后往前删除避免索引失效
            tasks.sort_unstable_by(|a, b| b.1.cmp(&a.1));
            
            let slot = &mut self.l0.slots[slot_index];
            
            for &(task_id, vec_index) in tasks.iter() {
                if vec_index < slot.len() && slot[vec_index].id == task_id {
                    if let Some(notifier) = slot[vec_index].completion_notifier.take() {
                        let _ = notifier.0.send(TaskCompletionReason::Cancelled);
                    }
                    
                    slot.swap_remove(vec_index);
                    
                    if vec_index < slot.len() {
                        let swapped_task_id = slot[vec_index].id;
                        if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                            swapped_location.vec_index = vec_index;
                        }
                    }
                    
                    self.task_index.remove(&task_id);
                    cancelled_count += 1;
                }
            }
        }
        
        // 处理 L1 层的取消
        for (slot_index, tasks) in l1_tasks_by_slot.iter_mut().enumerate() {
            if tasks.is_empty() {
                continue;
            }
            
            tasks.sort_unstable_by(|a, b| b.1.cmp(&a.1));
            
            let slot = &mut self.l1.slots[slot_index];
            
            for &(task_id, vec_index) in tasks.iter() {
                if vec_index < slot.len() && slot[vec_index].id == task_id {
                    if let Some(notifier) = slot[vec_index].completion_notifier.take() {
                        let _ = notifier.0.send(TaskCompletionReason::Cancelled);
                    }
                    
                    slot.swap_remove(vec_index);
                    
                    if vec_index < slot.len() {
                        let swapped_task_id = slot[vec_index].id;
                        if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                            swapped_location.vec_index = vec_index;
                        }
                    }
                    
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
    /// 到期的任务列表
    ///
    /// # 实现细节
    /// - L0 层每次推进 1 tick（无 rounds 检查）
    /// - L1 层每 (L1_tick / L0_tick) 次推进一次
    /// - L1 到期任务批量降级到 L0
    pub fn advance(&mut self) -> Vec<TimerTask> {
        // 推进 L0 层
        self.l0.current_tick += 1;
        
        let mut expired_tasks = Vec::new();
        
        // 处理 L0 层的到期任务（分层模式：L0 层没有 rounds，直接返回所有任务）
        let l0_slot_index = (self.l0.current_tick as usize) & self.l0.slot_mask;
        let l0_slot = &mut self.l0.slots[l0_slot_index];
        
        let i = 0;
        while i < l0_slot.len() {
            let task = &l0_slot[i];
            
            // 从索引中移除
            self.task_index.remove(&task.id);
            
            // 使用 swap_remove 移除任务
            let expired_task = l0_slot.swap_remove(i);
            
            // 更新被交换元素的索引
            if i < l0_slot.len() {
                let swapped_task_id = l0_slot[i].id;
                if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                    swapped_location.vec_index = i;
                }
            }
            
            expired_tasks.push(expired_task);
        }
        
        // 处理 L1 层
        // 检查是否需要推进 L1 层
        if self.l0.current_tick % self.l1_tick_ratio == 0 {
            self.l1.current_tick += 1;
            let l1_slot_index = (self.l1.current_tick as usize) & self.l1.slot_mask;
            let l1_slot = &mut self.l1.slots[l1_slot_index];
            
            // 收集 L1 层到期的任务
            let mut tasks_to_demote = Vec::new();
            let mut i = 0;
            while i < l1_slot.len() {
                let task = &mut l1_slot[i];
                
                if task.rounds > 0 {
                    // 还有轮次，减少轮次并保留
                    task.rounds -= 1;
                    if let Some(location) = self.task_index.get_mut(&task.id) {
                        location.vec_index = i;
                    }
                    i += 1;
                } else {
                    // rounds = 0，需要降级到 L0
                    self.task_index.remove(&task.id);
                    let task_to_demote = l1_slot.swap_remove(i);
                    
                    if i < l1_slot.len() {
                        let swapped_task_id = l1_slot[i].id;
                        if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                            swapped_location.vec_index = i;
                        }
                    }
                    
                    tasks_to_demote.push(task_to_demote);
                }
            }
            
            // 降级任务到 L0
            self.demote_tasks(tasks_to_demote);
        }
        
        expired_tasks
    }
    
    /// 降级任务从 L1 到 L0
    /// 
    /// 将 L1 到期的任务重新计算并插入到 L0 层
    fn demote_tasks(&mut self, tasks: Vec<TimerTask>) {
        for task in tasks {
            // 计算任务在 L0 层的剩余延迟
            // 任务的 deadline_tick 是基于 L1 tick 的，需要转换为 L0 tick
            let l1_tick_ratio = self.l1_tick_ratio;
            
            // 计算任务原本的到期时间（L1 tick）
            let l1_deadline = task.deadline_tick;
            
            // 转换为 L0 tick 的到期时间
            let l0_deadline_tick = l1_deadline * l1_tick_ratio;
            let l0_current_tick = self.l0.current_tick;
            
            // 计算剩余的 L0 ticks
            let remaining_l0_ticks = if l0_deadline_tick > l0_current_tick {
                l0_deadline_tick - l0_current_tick
            } else {
                1 // 至少在下一个 tick 触发
            };
            
            // 计算 L0 槽位索引
            let target_l0_tick = l0_current_tick + remaining_l0_ticks;
            let l0_slot_index = (target_l0_tick as usize) & self.l0.slot_mask;
            
            let task_id = task.id;
            let vec_index = self.l0.slots[l0_slot_index].len();
            let location = TaskLocation::new(0, l0_slot_index, vec_index);
            
            // 插入到 L0 层
            self.l0.slots[l0_slot_index].push(task);
            self.task_index.insert(task_id, location);
        }
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
    /// - 从原层级/槽位移除任务，保留其 completion_notifier（不会触发取消通知）
    /// - 更新延迟时间和回调函数（如果提供）
    /// - 根据 new_delay 重新计算目标层级、槽位和轮次
    /// - 可能发生跨层迁移（L0 <-> L1）
    /// - 使用原 TaskId 重新插入到新位置
    /// - 保持与外部持有的 TaskId 引用一致
    #[inline]
    pub fn postpone(
        &mut self,
        task_id: TaskId,
        new_delay: Duration,
        new_callback: Option<crate::task::CallbackWrapper>,
    ) -> bool {
        // 步骤1: 查找并移除原任务
        let old_location = match self.task_index.remove(&task_id) {
            Some(loc) => loc,
            None => return false,
        };
        
        // 使用 match 获取槽位引用
        let slot = match old_location.level {
            0 => &mut self.l0.slots[old_location.slot_index],
            _ => &mut self.l1.slots[old_location.slot_index],
        };
        
        // 验证任务仍在预期位置
        if old_location.vec_index >= slot.len() || slot[old_location.vec_index].id != task_id {
            // 索引不一致，重新插入并返回失败
            self.task_index.insert(task_id, old_location);
            return false;
        }
        
        // 使用 swap_remove 移除任务
        let mut task = slot.swap_remove(old_location.vec_index);
        
        // 更新被交换元素的索引（如果发生了交换）
        if old_location.vec_index < slot.len() {
            let swapped_task_id = slot[old_location.vec_index].id;
            if let Some(swapped_location) = self.task_index.get_mut(&swapped_task_id) {
                swapped_location.vec_index = old_location.vec_index;
            }
        }
        
        // 步骤2: 更新任务的延迟和回调
        task.delay = new_delay;
        if let Some(callback) = new_callback {
            task.callback = Some(callback);
        }
        
        // 步骤3: 根据新延迟重新计算层级、槽位和轮次
        let (new_level, ticks, new_rounds) = self.determine_layer(new_delay);
        
        // 使用 match 减少分支，并使用缓存的槽位掩码
        let (current_tick, slot_mask, slots) = match new_level {
            0 => (self.l0.current_tick, self.l0.slot_mask, &mut self.l0.slots),
            _ => (self.l1.current_tick, self.l1.slot_mask, &mut self.l1.slots),
        };
        
        let total_ticks = current_tick + ticks;
        let new_slot_index = (total_ticks as usize) & slot_mask;
        
        // 更新任务的时间轮参数
        task.deadline_tick = total_ticks;
        task.rounds = new_rounds;
        
        // 步骤4: 重新插入任务到新层级/槽位
        let new_vec_index = slots[new_slot_index].len();
        let new_location = TaskLocation::new(new_level, new_slot_index, new_vec_index);
        
        slots[new_slot_index].push(task);
        self.task_index.insert(task_id, new_location);
        
        true
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
        let wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        assert_eq!(wheel.slot_count(), 512);
        assert_eq!(wheel.current_tick(), 0);
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_hierarchical_wheel_creation() {
        let config = WheelConfig::default();
        
        let wheel = Wheel::new(config, BatchConfig::default());
        assert_eq!(wheel.slot_count(), 512); // L0 槽位数
        assert_eq!(wheel.current_tick(), 0);
        assert!(wheel.is_empty());
        // L1 层始终存在于分层模式中
        assert_eq!(wheel.l1.slot_count, 64);
        assert_eq!(wheel.l1_tick_ratio, 100); // 1000ms / 10ms = 100
    }

    #[test]
    fn test_hierarchical_config_validation() {
        // L1 tick 必须是 L0 tick 的整数倍
        let result = WheelConfig::builder()
            .l0_tick_duration(Duration::from_millis(10))
            .l0_slot_count(512)
            .l1_tick_duration(Duration::from_millis(15)) // 不是整数倍
            .l1_slot_count(64)
            .build();
        
        assert!(result.is_err());
        
        // 正确的配置
        let result = WheelConfig::builder()
            .l0_tick_duration(Duration::from_millis(10))
            .l0_slot_count(512)
            .l1_tick_duration(Duration::from_secs(1)) // 1000ms / 10ms = 100
            .l1_slot_count(64)
            .build();
        
        assert!(result.is_ok());
    }

    #[test]
    fn test_layer_determination() {
        let config = WheelConfig::default();
        
        let wheel = Wheel::new(config, BatchConfig::default());
        
        // 短延迟应该进入 L0 层
        // L0: 512 slots * 10ms = 5120ms
        let (level, _, rounds) = wheel.determine_layer(Duration::from_millis(100));
        assert_eq!(level, 0);
        assert_eq!(rounds, 0);
        
        // 长延迟应该进入 L1 层
        // 超过 L0 范围（>5120ms）
        let (level, _, rounds) = wheel.determine_layer(Duration::from_secs(10));
        assert_eq!(level, 1);
        assert_eq!(rounds, 0);
        
        // 超长延迟应该进入 L1 层并有 rounds
        // L1: 64 slots * 1000ms = 64000ms
        let (level, _, rounds) = wheel.determine_layer(Duration::from_secs(120));
        assert_eq!(level, 1);
        assert!(rounds > 0);
    }

    #[test]
    fn test_hierarchical_insert_and_advance() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let config = WheelConfig::default();
        
        let mut wheel = Wheel::new(config, BatchConfig::default());
        
        // 插入短延迟任务到 L0
        let callback = CallbackWrapper::new(|| async {});
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(task, CompletionNotifier(tx));
        
        // 验证任务在 L0 层
        let location = wheel.task_index.get(&task_id).unwrap();
        assert_eq!(location.level, 0);
        
        // 推进 10 ticks（100ms）
        for _ in 0..10 {
            let expired = wheel.advance();
            if !expired.is_empty() {
                assert_eq!(expired.len(), 1);
                assert_eq!(expired[0].id, task_id);
                return;
            }
        }
        panic!("Task should have expired");
    }

    #[test]
    fn test_hierarchical_l1_to_l0_demotion() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let config = WheelConfig::builder()
            .l0_tick_duration(Duration::from_millis(10))
            .l0_slot_count(512)
            .l1_tick_duration(Duration::from_millis(100)) // L1 tick = 100ms
            .l1_slot_count(64)
            .build()
            .unwrap();
        
        let mut wheel = Wheel::new(config, BatchConfig::default());
        let l1_tick_ratio = wheel.l1_tick_ratio;
        assert_eq!(l1_tick_ratio, 10); // 100ms / 10ms = 10
        
        // 插入任务，延迟 6000ms（超过 L0 范围 5120ms）
        let callback = CallbackWrapper::new(|| async {});
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let task = TimerTask::new(Duration::from_millis(6000), Some(callback));
        let task_id = wheel.insert(task, CompletionNotifier(tx));
        
        // 验证任务在 L1 层
        let location = wheel.task_index.get(&task_id).unwrap();
        assert_eq!(location.level, 1);
        
        // 推进到 L1 槽位到期（6000ms / 100ms = 60 L1 ticks）
        // 60 L1 ticks = 600 L0 ticks
        let mut demoted = false;
        for i in 0..610 {
            wheel.advance();
            
            // 检查任务是否被降级到 L0
            if let Some(location) = wheel.task_index.get(&task_id) {
                if location.level == 0 && !demoted {
                    demoted = true;
                    println!("Task demoted to L0 at L0 tick {}", i);
                }
            }
        }
        
        assert!(demoted, "Task should have been demoted from L1 to L0");
    }

    #[test]
    fn test_cross_layer_cancel() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let config = WheelConfig::default();
        
        let mut wheel = Wheel::new(config, BatchConfig::default());
        
        // 插入 L0 任务
        let callback1 = CallbackWrapper::new(|| async {});
        let (tx1, _rx1) = tokio::sync::oneshot::channel();
        let task1 = TimerTask::new(Duration::from_millis(100), Some(callback1));
        let task_id1 = wheel.insert(task1, CompletionNotifier(tx1));
        
        // 插入 L1 任务
        let callback2 = CallbackWrapper::new(|| async {});
        let (tx2, _rx2) = tokio::sync::oneshot::channel();
        let task2 = TimerTask::new(Duration::from_secs(10), Some(callback2));
        let task_id2 = wheel.insert(task2, CompletionNotifier(tx2));
        
        // 验证层级
        assert_eq!(wheel.task_index.get(&task_id1).unwrap().level, 0);
        assert_eq!(wheel.task_index.get(&task_id2).unwrap().level, 1);
        
        // 取消 L0 任务
        assert!(wheel.cancel(task_id1));
        assert!(wheel.task_index.get(&task_id1).is_none());
        
        // 取消 L1 任务
        assert!(wheel.cancel(task_id2));
        assert!(wheel.task_index.get(&task_id2).is_none());
        
        assert!(wheel.is_empty());
    }

    #[test]
    fn test_cross_layer_postpone() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let config = WheelConfig::default();
        
        let mut wheel = Wheel::new(config, BatchConfig::default());
        
        // 插入 L0 任务（100ms）
        let callback = CallbackWrapper::new(|| async {});
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(task, CompletionNotifier(tx));
        
        // 验证在 L0 层
        assert_eq!(wheel.task_index.get(&task_id).unwrap().level, 0);
        
        // 推迟到 10 秒（应该迁移到 L1）
        assert!(wheel.postpone(task_id, Duration::from_secs(10), None));
        
        // 验证已迁移到 L1 层
        assert_eq!(wheel.task_index.get(&task_id).unwrap().level, 1);
        
        // 再推迟回 200ms（应该迁移回 L0）
        assert!(wheel.postpone(task_id, Duration::from_millis(200), None));
        
        // 验证已迁移回 L0 层
        assert_eq!(wheel.task_index.get(&task_id).unwrap().level, 0);
    }

    #[test]
    fn test_delay_to_ticks() {
        let wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(100)), 10);
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(50)), 5);
        assert_eq!(wheel.delay_to_ticks(Duration::from_millis(1)), 1); // 最小 1 tick
    }

    #[test]
    fn test_wheel_invalid_slot_count() {
        let result = WheelConfig::builder()
            .l0_slot_count(100)
            .build();
        assert!(result.is_err());
        if let Err(crate::error::TimerError::InvalidSlotCount { slot_count, reason }) = result {
            assert_eq!(slot_count, 100);
            assert_eq!(reason, "L0 层槽位数量必须是 2 的幂次方");
        } else {
            panic!("Expected InvalidSlotCount error");
        }
    }

    #[test]
    fn test_insert_batch() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 创建批量任务
        let tasks: Vec<(TimerTask, CompletionNotifier)> = (0..10)
            .map(|i| {
                let callback = CallbackWrapper::new(|| async {});
                let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
                let notifier = CompletionNotifier(completion_tx);
                let task = TimerTask::new(Duration::from_millis(100 + i * 10), Some(callback));
                (task, notifier)
            })
            .collect();
        
        let task_ids = wheel.insert_batch(tasks);
        
        assert_eq!(task_ids.len(), 10);
        assert!(!wheel.is_empty());
    }

    #[test]
    fn test_cancel_batch() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入多个任务
        let mut task_ids = Vec::new();
        for i in 0..10 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(100 + i * 10), Some(callback));
            let task_id = wheel.insert(task, notifier);
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入多个相同延迟的任务（会进入同一个槽位）
        let mut task_ids = Vec::new();
        for _ in 0..20 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(100), Some(callback));
            let task_id = wheel.insert(task, notifier);
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入任务，延迟 100ms
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(task, notifier);
        
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入任务，带原始回调
        let old_callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(old_callback.clone()));
        let task_id = wheel.insert(task, notifier);
        
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
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 尝试推迟不存在的任务
        let fake_task_id = TaskId::new();
        let postponed = wheel.postpone(fake_task_id, Duration::from_millis(100), None);
        assert!(!postponed);
    }

    #[test]
    fn test_postpone_batch() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入 5 个任务，延迟 50ms（5 ticks）
        let mut task_ids = Vec::new();
        for _ in 0..5 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(50), Some(callback));
            let task_id = wheel.insert(task, notifier);
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入 10 个任务，延迟 50ms（5 ticks）
        let mut task_ids = Vec::new();
        for _ in 0..10 {
            let callback = CallbackWrapper::new(|| async {});
            let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
            let notifier = CompletionNotifier(completion_tx);
            let task = TimerTask::new(Duration::from_millis(50), Some(callback));
            let task_id = wheel.insert(task, notifier);
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 分层模式下测试 L1 层的多轮任务
        // L1: 64 slots * 1000ms = 64000ms
        // 插入一个超过 L1 一圈的任务，延迟 120000ms (120 秒)
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_secs(120), Some(callback));
        let task_id = wheel.insert(task, notifier);
        
        // 120000ms / 1000ms = 120 L1 ticks
        // 120 ticks / 64 slots = 1 轮 + 56 ticks
        // 任务应该在 L1 层，slot 56，rounds = 1
        
        // 验证任务在 L1 层
        let location = wheel.task_index.get(&task_id).unwrap();
        assert_eq!(location.level, 1);
        
        // L1 tick 每 100 个 L0 tick 推进一次
        // 推进 64 * 100 = 6400 个 L0 ticks（完成 L1 的第一轮）
        for _ in 0..6400 {
            let _expired = wheel.advance();
            // L1 第一轮期间，任务不应该降级或触发
        }
        
        // 任务应该还在 L1 层，但 rounds 减少了
        let location = wheel.task_index.get(&task_id);
        if let Some(loc) = location {
            assert_eq!(loc.level, 1);
        }
        
        // 继续推进直到任务触发（大约还需要 56 * 100 = 5600 个 L0 ticks）
        let mut triggered = false;
        for _ in 0..6000 {
            let expired = wheel.advance();
            if expired.iter().any(|t| t.id == task_id) {
                triggered = true;
                break;
            }
        }
        assert!(triggered, "任务应该在 L1 第二轮触发");
    }

    #[test]
    fn test_minimum_delay() {
        use crate::task::{TimerTask, CompletionNotifier};
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 测试最小延迟（小于 1 tick 的延迟应该向上取整为 1 tick）
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(1), Some(callback));
        let task_id: TaskId = wheel.insert(task, notifier);
        
        // 推进 1 tick，任务应该触发
        let expired = wheel.advance();
        assert_eq!(expired.len(), 1, "最小延迟任务应该在 1 tick 后触发");
        assert_eq!(expired[0].id, task_id);
    }

    #[test]
    fn test_empty_batch_operations() {
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入任务
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(task, notifier);
        
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
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入任务
        let callback = CallbackWrapper::new(|| async {});
        let (completion_tx, _completion_rx) = tokio::sync::oneshot::channel();
        let notifier = CompletionNotifier(completion_tx);
        let task = TimerTask::new(Duration::from_millis(100), Some(callback));
        let task_id = wheel.insert(task, notifier);
        
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 测试槽位边界和环绕
        // 第一个任务：延迟 10ms（1 tick），应该在 slot 1 触发
        let callback1 = CallbackWrapper::new(|| async {});
        let (tx1, _rx1) = tokio::sync::oneshot::channel();
        let task1 = TimerTask::new(Duration::from_millis(10), Some(callback1));
        let task_id_1 = wheel.insert(task1, CompletionNotifier(tx1));
        
        // 第二个任务：延迟 5110ms（511 ticks），应该在 slot 511 触发
        let callback2 = CallbackWrapper::new(|| async {});
        let (tx2, _rx2) = tokio::sync::oneshot::channel();
        let task2 = TimerTask::new(Duration::from_millis(5110), Some(callback2));
        let task_id_2 = wheel.insert(task2, CompletionNotifier(tx2));
        
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
        let mut wheel = Wheel::new(WheelConfig::default(), batch_config);
        
        // 插入 10 个任务
        let mut task_ids = Vec::new();
        for _ in 0..10 {
            let callback = CallbackWrapper::new(|| async {});
            let (tx, _rx) = tokio::sync::oneshot::channel();
            let task = TimerTask::new(Duration::from_millis(100), Some(callback));
            let task_id = wheel.insert(task, CompletionNotifier(tx));
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
        
        let mut wheel = Wheel::new(WheelConfig::default(), BatchConfig::default());
        
        // 插入多个任务，验证 TaskId 唯一性
        let mut task_ids = std::collections::HashSet::new();
        for _ in 0..100 {
            let callback = CallbackWrapper::new(|| async {});
            let (tx, _rx) = tokio::sync::oneshot::channel();
            let task = TimerTask::new(Duration::from_millis(100), Some(callback));
            let task_id = wheel.insert(task, CompletionNotifier(tx));
            
            assert!(task_ids.insert(task_id), "TaskId 应该是唯一的");
        }
        
        assert_eq!(task_ids.len(), 100);
    }
}

