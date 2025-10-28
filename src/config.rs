//! 定时器配置模块
//!
//! 提供分层的配置结构和 Builder 模式，用于配置时间轮、服务和批处理行为。

use crate::error::TimerError;
use std::time::Duration;

/// 分层时间轮配置
///
/// 用于配置2层时间轮的参数。L0 层处理短延迟任务，L1 层处理长延迟任务。
///
/// # 示例
/// ```no_run
/// use kestrel_protocol_timer::HierarchicalWheelConfig;
/// use std::time::Duration;
///
/// let config = HierarchicalWheelConfig {
///     l0_tick_duration: Duration::from_millis(10),
///     l0_slot_count: 512,
///     l1_tick_duration: Duration::from_secs(1),
///     l1_slot_count: 60,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct HierarchicalWheelConfig {
    /// L0 层（底层）每个 tick 的时间长度
    pub l0_tick_duration: Duration,
    /// L0 层槽位数量（必须是 2 的幂次方）
    pub l0_slot_count: usize,
    
    /// L1 层（高层）每个 tick 的时间长度
    pub l1_tick_duration: Duration,
    /// L1 层槽位数量（必须是 2 的幂次方）
    pub l1_slot_count: usize,
}

impl Default for HierarchicalWheelConfig {
    fn default() -> Self {
        Self {
            l0_tick_duration: Duration::from_millis(10),
            l0_slot_count: 512,
            l1_tick_duration: Duration::from_secs(1),
            l1_slot_count: 64,
        }
    }
}

/// 时间轮配置
///
/// 用于配置分层时间轮的参数。系统仅支持分层模式。
///
/// # 示例
/// ```no_run
/// use kestrel_protocol_timer::WheelConfig;
/// use std::time::Duration;
///
/// // 使用默认配置（分层模式）
/// let config = WheelConfig::default();
///
/// // 使用 Builder 自定义配置
/// let config = WheelConfig::builder()
///     .l0_tick_duration(Duration::from_millis(20))
///     .l0_slot_count(1024)
///     .l1_tick_duration(Duration::from_secs(2))
///     .l1_slot_count(128)
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct WheelConfig {
    /// 分层时间轮配置
    pub hierarchical: HierarchicalWheelConfig,
}

impl Default for WheelConfig {
    fn default() -> Self {
        Self {
            hierarchical: HierarchicalWheelConfig::default(),
        }
    }
}

impl WheelConfig {
    /// 创建配置构建器
    pub fn builder() -> WheelConfigBuilder {
        WheelConfigBuilder::default()
    }
}

/// 时间轮配置构建器
#[derive(Debug, Clone)]
pub struct WheelConfigBuilder {
    hierarchical: HierarchicalWheelConfig,
}

impl Default for WheelConfigBuilder {
    fn default() -> Self {
        Self {
            hierarchical: HierarchicalWheelConfig::default(),
        }
    }
}

impl WheelConfigBuilder {
    /// 设置 L0 层 tick 时长
    pub fn l0_tick_duration(mut self, duration: Duration) -> Self {
        self.hierarchical.l0_tick_duration = duration;
        self
    }

    /// 设置 L0 层槽位数量
    pub fn l0_slot_count(mut self, count: usize) -> Self {
        self.hierarchical.l0_slot_count = count;
        self
    }

    /// 设置 L1 层 tick 时长
    pub fn l1_tick_duration(mut self, duration: Duration) -> Self {
        self.hierarchical.l1_tick_duration = duration;
        self
    }

    /// 设置 L1 层槽位数量
    pub fn l1_slot_count(mut self, count: usize) -> Self {
        self.hierarchical.l1_slot_count = count;
        self
    }

    /// 构建配置并进行验证
    ///
    /// # 返回
    /// - `Ok(WheelConfig)`: 配置有效
    /// - `Err(TimerError)`: 配置验证失败
    ///
    /// # 验证规则
    /// - L0 tick 时长必须大于 0
    /// - L1 tick 时长必须大于 0
    /// - L0 槽位数量必须大于 0 且是 2 的幂次方
    /// - L1 槽位数量必须大于 0 且是 2 的幂次方
    /// - L1 tick 必须是 L0 tick 的整数倍
    pub fn build(self) -> Result<WheelConfig, TimerError> {
        let h = &self.hierarchical;
        
        // 验证 L0 层配置
        if h.l0_tick_duration.is_zero() {
            return Err(TimerError::InvalidConfiguration {
                field: "l0_tick_duration".to_string(),
                reason: "L0 层 tick 时长必须大于 0".to_string(),
            });
        }

        if h.l0_slot_count == 0 {
            return Err(TimerError::InvalidSlotCount {
                slot_count: h.l0_slot_count,
                reason: "L0 层槽位数量必须大于 0",
            });
        }

        if !h.l0_slot_count.is_power_of_two() {
            return Err(TimerError::InvalidSlotCount {
                slot_count: h.l0_slot_count,
                reason: "L0 层槽位数量必须是 2 的幂次方",
            });
        }

        // 验证 L1 层配置
        if h.l1_tick_duration.is_zero() {
            return Err(TimerError::InvalidConfiguration {
                field: "l1_tick_duration".to_string(),
                reason: "L1 层 tick 时长必须大于 0".to_string(),
            });
        }

        if h.l1_slot_count == 0 {
            return Err(TimerError::InvalidSlotCount {
                slot_count: h.l1_slot_count,
                reason: "L1 层槽位数量必须大于 0",
            });
        }

        if !h.l1_slot_count.is_power_of_two() {
            return Err(TimerError::InvalidSlotCount {
                slot_count: h.l1_slot_count,
                reason: "L1 层槽位数量必须是 2 的幂次方",
            });
        }

        // 验证 L1 tick 是 L0 tick 的整数倍
        let l0_ms = h.l0_tick_duration.as_millis() as u64;
        let l1_ms = h.l1_tick_duration.as_millis() as u64;
        if l1_ms % l0_ms != 0 {
            return Err(TimerError::InvalidConfiguration {
                field: "l1_tick_duration".to_string(),
                reason: format!(
                    "L1 tick 时长 ({} ms) 必须是 L0 tick 时长 ({} ms) 的整数倍",
                    l1_ms, l0_ms
                ),
            });
        }

        Ok(WheelConfig {
            hierarchical: self.hierarchical,
        })
    }
}

/// 服务配置
///
/// 用于配置 TimerService 的通道容量。
///
/// # 示例
/// ```no_run
/// use kestrel_protocol_timer::ServiceConfig;
///
/// // 使用默认配置
/// let config = ServiceConfig::default();
///
/// // 使用 Builder 自定义配置
/// let config = ServiceConfig::builder()
///     .command_channel_capacity(1024)
///     .timeout_channel_capacity(2000)
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// 命令通道容量
    pub command_channel_capacity: usize,
    /// 超时通道容量
    pub timeout_channel_capacity: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            command_channel_capacity: 512,
            timeout_channel_capacity: 1000,
        }
    }
}

impl ServiceConfig {
    /// 创建配置构建器
    pub fn builder() -> ServiceConfigBuilder {
        ServiceConfigBuilder::default()
    }
}

/// 服务配置构建器
#[derive(Debug, Clone)]
pub struct ServiceConfigBuilder {
    command_channel_capacity: usize,
    timeout_channel_capacity: usize,
}

impl Default for ServiceConfigBuilder {
    fn default() -> Self {
        let config = ServiceConfig::default();
        Self {
            command_channel_capacity: config.command_channel_capacity,
            timeout_channel_capacity: config.timeout_channel_capacity,
        }
    }
}

impl ServiceConfigBuilder {
    /// 设置命令通道容量
    pub fn command_channel_capacity(mut self, capacity: usize) -> Self {
        self.command_channel_capacity = capacity;
        self
    }

    /// 设置超时通道容量
    pub fn timeout_channel_capacity(mut self, capacity: usize) -> Self {
        self.timeout_channel_capacity = capacity;
        self
    }

    /// 构建配置并进行验证
    ///
    /// # 返回
    /// - `Ok(ServiceConfig)`: 配置有效
    /// - `Err(TimerError)`: 配置验证失败
    ///
    /// # 验证规则
    /// - 所有通道容量必须大于 0
    pub fn build(self) -> Result<ServiceConfig, TimerError> {
        if self.command_channel_capacity == 0 {
            return Err(TimerError::InvalidConfiguration {
                field: "command_channel_capacity".to_string(),
                reason: "命令通道容量必须大于 0".to_string(),
            });
        }

        if self.timeout_channel_capacity == 0 {
            return Err(TimerError::InvalidConfiguration {
                field: "timeout_channel_capacity".to_string(),
                reason: "超时通道容量必须大于 0".to_string(),
            });
        }

        Ok(ServiceConfig {
            command_channel_capacity: self.command_channel_capacity,
            timeout_channel_capacity: self.timeout_channel_capacity,
        })
    }
}

/// 批处理配置
///
/// 用于配置批量操作的优化参数。
///
/// # 示例
/// ```no_run
/// use kestrel_protocol_timer::BatchConfig;
///
/// // 使用默认配置
/// let config = BatchConfig::default();
///
/// // 自定义配置
/// let config = BatchConfig {
///     small_batch_threshold: 20,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// 小批量阈值，用于批量取消优化
    /// 
    /// 当批量取消的任务数量小于等于此值时，直接逐个取消而不进行分组排序
    pub small_batch_threshold: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            small_batch_threshold: 10,
        }
    }
}

/// 顶层定时器配置
///
/// 组合所有子配置，提供完整的定时器系统配置。
///
/// # 示例
/// ```no_run
/// use kestrel_protocol_timer::TimerConfig;
///
/// // 使用默认配置
/// let config = TimerConfig::default();
///
/// // 使用 Builder 自定义配置（仅配置服务参数）
/// let config = TimerConfig::builder()
///     .command_channel_capacity(1024)
///     .timeout_channel_capacity(2000)
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct TimerConfig {
    /// 时间轮配置
    pub wheel: WheelConfig,
    /// 服务配置
    pub service: ServiceConfig,
    /// 批处理配置
    pub batch: BatchConfig,
}

impl Default for TimerConfig {
    fn default() -> Self {
        Self {
            wheel: WheelConfig::default(),
            service: ServiceConfig::default(),
            batch: BatchConfig::default(),
        }
    }
}

impl TimerConfig {
    /// 创建配置构建器
    pub fn builder() -> TimerConfigBuilder {
        TimerConfigBuilder::default()
    }
}

/// 顶层定时器配置构建器
#[derive(Debug)]
pub struct TimerConfigBuilder {
    wheel_builder: WheelConfigBuilder,
    service_builder: ServiceConfigBuilder,
    batch_config: BatchConfig,
}

impl Default for TimerConfigBuilder {
    fn default() -> Self {
        Self {
            wheel_builder: WheelConfigBuilder::default(),
            service_builder: ServiceConfigBuilder::default(),
            batch_config: BatchConfig::default(),
        }
    }
}

impl TimerConfigBuilder {
    /// 设置命令通道容量
    pub fn command_channel_capacity(mut self, capacity: usize) -> Self {
        self.service_builder = self.service_builder.command_channel_capacity(capacity);
        self
    }

    /// 设置超时通道容量
    pub fn timeout_channel_capacity(mut self, capacity: usize) -> Self {
        self.service_builder = self.service_builder.timeout_channel_capacity(capacity);
        self
    }

    /// 设置小批量阈值
    pub fn small_batch_threshold(mut self, threshold: usize) -> Self {
        self.batch_config.small_batch_threshold = threshold;
        self
    }

    /// 构建配置并进行验证
    ///
    /// # 返回
    /// - `Ok(TimerConfig)`: 配置有效
    /// - `Err(TimerError)`: 配置验证失败
    pub fn build(self) -> Result<TimerConfig, TimerError> {
        Ok(TimerConfig {
            wheel: self.wheel_builder.build()?,
            service: self.service_builder.build()?,
            batch: self.batch_config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wheel_config_default() {
        let config = WheelConfig::default();
        assert_eq!(config.hierarchical.l0_tick_duration, Duration::from_millis(10));
        assert_eq!(config.hierarchical.l0_slot_count, 512);
        assert_eq!(config.hierarchical.l1_tick_duration, Duration::from_secs(1));
        assert_eq!(config.hierarchical.l1_slot_count, 64);
    }

    #[test]
    fn test_wheel_config_builder() {
        let config = WheelConfig::builder()
            .l0_tick_duration(Duration::from_millis(20))
            .l0_slot_count(1024)
            .l1_tick_duration(Duration::from_secs(2))
            .l1_slot_count(128)
            .build()
            .unwrap();

        assert_eq!(config.hierarchical.l0_tick_duration, Duration::from_millis(20));
        assert_eq!(config.hierarchical.l0_slot_count, 1024);
        assert_eq!(config.hierarchical.l1_tick_duration, Duration::from_secs(2));
        assert_eq!(config.hierarchical.l1_slot_count, 128);
    }

    #[test]
    fn test_wheel_config_validation_zero_tick() {
        let result = WheelConfig::builder()
            .l0_tick_duration(Duration::ZERO)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_wheel_config_validation_invalid_slot_count() {
        let result = WheelConfig::builder()
            .l0_slot_count(100)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_service_config_default() {
        let config = ServiceConfig::default();
        assert_eq!(config.command_channel_capacity, 512);
        assert_eq!(config.timeout_channel_capacity, 1000);
    }

    #[test]
    fn test_service_config_builder() {
        let config = ServiceConfig::builder()
            .command_channel_capacity(1024)
            .timeout_channel_capacity(2000)
            .build()
            .unwrap();

        assert_eq!(config.command_channel_capacity, 1024);
        assert_eq!(config.timeout_channel_capacity, 2000);
    }

    #[test]
    fn test_batch_config_default() {
        let config = BatchConfig::default();
        assert_eq!(config.small_batch_threshold, 10);
    }

    #[test]
    fn test_timer_config_default() {
        let config = TimerConfig::default();
        assert_eq!(config.wheel.hierarchical.l0_slot_count, 512);
        assert_eq!(config.service.command_channel_capacity, 512);
        assert_eq!(config.batch.small_batch_threshold, 10);
    }

    #[test]
    fn test_timer_config_builder() {
        let config = TimerConfig::builder()
            .command_channel_capacity(1024)
            .timeout_channel_capacity(2000)
            .small_batch_threshold(20)
            .build()
            .unwrap();

        assert_eq!(config.service.command_channel_capacity, 1024);
        assert_eq!(config.service.timeout_channel_capacity, 2000);
        assert_eq!(config.batch.small_batch_threshold, 20);
    }
}

