use std::fmt;

/// 定时器错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerError {
    /// 槽位数量无效（必须是 2 的幂次方且大于 0）
    InvalidSlotCount { 
        slot_count: usize,
        reason: &'static str,
    },
    
    /// 配置验证失败
    InvalidConfiguration {
        field: String,
        reason: String,
    },
    
    /// 注册失败（内部通道已满或已关闭）
    RegisterFailed,
}

impl fmt::Display for TimerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimerError::InvalidSlotCount { slot_count, reason } => {
                write!(f, "无效的槽位数量 {}: {}", slot_count, reason)
            }
            TimerError::InvalidConfiguration { field, reason } => {
                write!(f, "配置验证失败 ({}): {}", field, reason)
            }
            TimerError::RegisterFailed => {
                write!(f, "注册失败：内部通道已满或已关闭")
            }
        }
    }
}

impl std::error::Error for TimerError {}

