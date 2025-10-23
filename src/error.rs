use std::fmt;
use crate::task::TimerWheelId;

/// 定时器错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerError {
    /// 槽位数量无效（必须是 2 的幂次方且大于 0）
    InvalidSlotCount { 
        slot_count: usize,
        reason: &'static str,
    },
    
    /// 内部通信通道已关闭
    ChannelClosed,
    
    /// TimerWheel ID 不匹配
    TimerWheelIdMismatch {
        expected: TimerWheelId,
        actual: TimerWheelId,
    },
}

impl fmt::Display for TimerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimerError::InvalidSlotCount { slot_count, reason } => {
                write!(f, "无效的槽位数量 {}: {}", slot_count, reason)
            }
            TimerError::ChannelClosed => {
                write!(f, "内部通信通道已关闭")
            }
            TimerError::TimerWheelIdMismatch { expected, actual } => {
                write!(f, "TimerWheel ID 不匹配: 期望 {:?}, 实际 {:?}", expected, actual)
            }
        }
    }
}

impl std::error::Error for TimerError {}

