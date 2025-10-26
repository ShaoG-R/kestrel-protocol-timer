use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;

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
/// use kestrel_protocol_timer::TimerCallback;
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

/// 完成通知器，用于在任务完成时发送通知
pub struct CompletionNotifier(pub oneshot::Sender<()>);

/// 定时器任务
pub struct TimerTask {
    /// 任务唯一标识符
    pub id: TaskId,
    
    /// 到期时间（相对于时间轮的 tick 数）
    pub deadline_tick: u64,
    
    /// 轮次计数（用于超出时间轮范围的任务）
    pub rounds: u32,
    
    /// 异步回调函数（可选）
    pub callback: Option<CallbackWrapper>,
    
    /// 完成通知器（用于在任务完成时发送通知）
    pub completion_notifier: CompletionNotifier,
}

impl TimerTask {
    /// 创建一次性定时器任务
    pub fn once(
        deadline_tick: u64,
        rounds: u32,
        callback: Option<CallbackWrapper>,
        completion_notifier: CompletionNotifier,
    ) -> Self {
        Self {
            id: TaskId::new(),
            deadline_tick,
            rounds,
            callback,
            completion_notifier,
        }
    }

    /// 获取回调函数的克隆（如果存在）
    pub fn get_callback(&self) -> Option<CallbackWrapper> {
        self.callback.as_ref().map(Arc::clone)
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

