//! 定时器配置模块
//!
//! 提供分层的配置结构和 Builder 模式，用于配置时间轮、服务和批处理行为。

use crate::error::TimerError;
use std::time::Duration;

/// 时间轮配置
///
/// 用于配置时间轮的基本参数，包括 tick 时长和槽位数量。
///
/// # 示例
/// ```no_run
/// use kestrel_protocol_timer::WheelConfig;
/// use std::time::Duration;
///
/// // 使用默认配置
/// let config = WheelConfig::default();
///
/// // 使用 Builder 自定义配置
/// let config = WheelConfig::builder()
///     .tick_duration(Duration::from_millis(20))
///     .slot_count(1024)
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct WheelConfig {
    /// 每个 tick 的时间长度
    pub tick_duration: Duration,
    /// 槽位数量（必须是 2 的幂次方）
    pub slot_count: usize,
}

impl Default for WheelConfig {
    fn default() -> Self {
        Self {
            tick_duration: Duration::from_millis(10),
            slot_count: 512,
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
    tick_duration: Duration,
    slot_count: usize,
}

impl Default for WheelConfigBuilder {
    fn default() -> Self {
        let config = WheelConfig::default();
        Self {
            tick_duration: config.tick_duration,
            slot_count: config.slot_count,
        }
    }
}

impl WheelConfigBuilder {
    /// 设置 tick 时长
    pub fn tick_duration(mut self, duration: Duration) -> Self {
        self.tick_duration = duration;
        self
    }

    /// 设置槽位数量
    pub fn slot_count(mut self, count: usize) -> Self {
        self.slot_count = count;
        self
    }

    /// 构建配置并进行验证
    ///
    /// # 返回
    /// - `Ok(WheelConfig)`: 配置有效
    /// - `Err(TimerError)`: 配置验证失败
    ///
    /// # 验证规则
    /// - tick_duration 必须大于 0
    /// - slot_count 必须大于 0 且是 2 的幂次方
    pub fn build(self) -> Result<WheelConfig, TimerError> {
        // 验证 tick_duration
        if self.tick_duration.is_zero() {
            return Err(TimerError::InvalidConfiguration {
                field: "tick_duration".to_string(),
                reason: "tick 时长必须大于 0".to_string(),
            });
        }

        // 验证 slot_count
        if self.slot_count == 0 {
            return Err(TimerError::InvalidSlotCount {
                slot_count: self.slot_count,
                reason: "槽位数量必须大于 0",
            });
        }

        if !self.slot_count.is_power_of_two() {
            return Err(TimerError::InvalidSlotCount {
                slot_count: self.slot_count,
                reason: "槽位数量必须是 2 的幂次方",
            });
        }

        Ok(WheelConfig {
            tick_duration: self.tick_duration,
            slot_count: self.slot_count,
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
/// // 使用 Builder 自定义配置
/// let config = TimerConfig::builder()
///     .tick_duration(std::time::Duration::from_millis(20))
///     .slot_count(1024)
///     .command_channel_capacity(1024)
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
    /// 设置 tick 时长
    pub fn tick_duration(mut self, duration: Duration) -> Self {
        self.wheel_builder = self.wheel_builder.tick_duration(duration);
        self
    }

    /// 设置槽位数量
    pub fn slot_count(mut self, count: usize) -> Self {
        self.wheel_builder = self.wheel_builder.slot_count(count);
        self
    }

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
        assert_eq!(config.tick_duration, Duration::from_millis(10));
        assert_eq!(config.slot_count, 512);
    }

    #[test]
    fn test_wheel_config_builder() {
        let config = WheelConfig::builder()
            .tick_duration(Duration::from_millis(20))
            .slot_count(1024)
            .build()
            .unwrap();

        assert_eq!(config.tick_duration, Duration::from_millis(20));
        assert_eq!(config.slot_count, 1024);
    }

    #[test]
    fn test_wheel_config_validation_zero_tick() {
        let result = WheelConfig::builder()
            .tick_duration(Duration::ZERO)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_wheel_config_validation_invalid_slot_count() {
        let result = WheelConfig::builder()
            .slot_count(100)
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
        assert_eq!(config.wheel.slot_count, 512);
        assert_eq!(config.service.command_channel_capacity, 512);
        assert_eq!(config.batch.small_batch_threshold, 10);
    }

    #[test]
    fn test_timer_config_builder() {
        let config = TimerConfig::builder()
            .tick_duration(Duration::from_millis(20))
            .slot_count(1024)
            .command_channel_capacity(1024)
            .timeout_channel_capacity(2000)
            .small_batch_threshold(20)
            .build()
            .unwrap();

        assert_eq!(config.wheel.tick_duration, Duration::from_millis(20));
        assert_eq!(config.wheel.slot_count, 1024);
        assert_eq!(config.service.command_channel_capacity, 1024);
        assert_eq!(config.service.timeout_channel_capacity, 2000);
        assert_eq!(config.batch.small_batch_threshold, 20);
    }
}

