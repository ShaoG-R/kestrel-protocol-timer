use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 全局唯一的任务 ID 生成器
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

/// 定时器任务的唯一标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(u64);

impl TaskId {
    /// 生成一个新的唯一任务 ID
    pub fn new() -> Self {
        TaskId(NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed))
    }

    /// 获取任务 ID 的数值
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

/// 定时器回调 trait
/// 
/// 实现此 trait 的类型可以作为定时器的回调函数使用。
/// 
/// # 示例
/// 
/// ```
/// use timer::TimerCallback;
/// use std::future::Future;
/// use std::pin::Pin;
/// 
/// struct MyCallback;
/// 
/// impl TimerCallback for MyCallback {
///     fn call(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
///         Box::pin(async {
///             println!("Timer callback executed!");
///         })
///     }
/// }
/// ```
pub trait TimerCallback: Send + Sync + 'static {
    /// 执行回调，返回一个 Future
    fn call(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// 为闭包实现 TimerCallback trait
/// 支持 Fn() -> Future 类型的闭包（可以多次调用，适合周期性任务）
impl<F, Fut> TimerCallback for F
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn call(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(self())
    }
}

/// 回调包装器类型
pub type CallbackWrapper = Arc<dyn TimerCallback>;

/// 定时器任务类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// 一次性任务
    Once,
    
    /// 周期性任务
    Repeat {
        /// 重复间隔
        interval: Duration,
    },
}

impl TaskType {
    /// 检查是否是周期性任务
    pub fn is_repeat(&self) -> bool {
        matches!(self, TaskType::Repeat { .. })
    }

    /// 获取重复间隔（如果是周期性任务）
    pub fn interval(&self) -> Option<Duration> {
        match self {
            TaskType::Repeat { interval } => Some(*interval),
            TaskType::Once => None,
        }
    }
}

/// 定时器任务
pub struct TimerTask {
    /// 任务唯一标识符
    pub id: TaskId,
    
    /// 到期时间（相对于时间轮的 tick 数）
    pub deadline_tick: u64,
    
    /// 轮次计数（用于超出时间轮范围的任务）
    pub rounds: u32,
    
    /// 异步回调函数（使用 Arc 可以共享，支持周期性任务）
    pub callback: CallbackWrapper,
    
    /// 任务类型（一次性或周期性）
    pub task_type: TaskType,
}

impl TimerTask {
    /// 创建一次性定时器任务
    pub fn once(
        deadline_tick: u64,
        rounds: u32,
        callback: CallbackWrapper,
    ) -> Self {
        Self {
            id: TaskId::new(),
            deadline_tick,
            rounds,
            callback,
            task_type: TaskType::Once,
        }
    }

    /// 创建周期性定时器任务
    pub fn repeat(
        deadline_tick: u64,
        rounds: u32,
        interval: Duration,
        callback: CallbackWrapper,
    ) -> Self {
        Self {
            id: TaskId::new(),
            deadline_tick,
            rounds,
            callback,
            task_type: TaskType::Repeat { interval },
        }
    }

    /// 检查是否是周期性任务
    pub fn is_repeat(&self) -> bool {
        self.task_type.is_repeat()
    }

    /// 获取重复间隔（如果是周期性任务）
    pub fn interval(&self) -> Option<Duration> {
        self.task_type.interval()
    }

    /// 获取回调函数的克隆（用于周期性任务）
    pub fn get_callback(&self) -> CallbackWrapper {
        Arc::clone(&self.callback)
    }

    /// 克隆任务用于周期性执行（保持相同的 ID）
    pub fn clone_for_repeat(&self, new_deadline_tick: u64, new_rounds: u32) -> Self {
        Self {
            id: self.id, // 保持相同的任务 ID，以便能够取消周期性任务
            deadline_tick: new_deadline_tick,
            rounds: new_rounds,
            callback: Arc::clone(&self.callback),
            task_type: self.task_type,
        }
    }
}

/// 任务位置信息，用于取消操作
#[derive(Debug, Clone)]
pub struct TaskLocation {
    pub slot_index: usize,
    /// 任务在槽位 Vec 中的索引位置（用于 O(1) 取消）
    pub vec_index: usize,
    #[allow(dead_code)]
    pub task_id: TaskId,
}

impl TaskLocation {
    pub fn new(slot_index: usize, vec_index: usize, task_id: TaskId) -> Self {
        Self {
            slot_index,
            vec_index,
            task_id,
        }
    }
}

