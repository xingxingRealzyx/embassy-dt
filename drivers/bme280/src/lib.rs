#![no_std]
#![deny(unsafe_code)]
#![warn(missing_docs)]

//! BME280 temperature/humidity sensor driver following the `embassy-dt` device-tree
//! initialization convention.
//!
//! Convention: a driver type provides
//! `async fn init(deps..., &NodeDesc) -> Result<Self, DeviceError>`, where deps are
//! passed by value in device-tree dependency order. This driver's signature is
//! `Bme280::init(i2c, node)`; the required property is `addr` (7-bit I2C address).
//!
//! ```text
//! device bme280: Bme280Driver { bus: i2c0, addr: 0x76 };
//! ```

use embassy_dt::{DeviceError, NodeDesc};
use embedded_hal_async::i2c::I2c;

const REG_ID: u8 = 0xD0;
const REG_RESET: u8 = 0xE0;
const RESET_CMD: u8 = 0xB6;
const REG_CTRL_HUM: u8 = 0xF2;
const REG_CTRL_MEAS: u8 = 0xF4;
const REG_CALIB1: u8 = 0x88; // dig_T1..dig_H1（26 字节）
const REG_CALIB2: u8 = 0xE1; // dig_H2..dig_H6（7 字节）
const REG_TEMP: u8 = 0xFA;
const REG_HUM: u8 = 0xFD;
const CHIP_ID: u8 = 0x60;

/// BME280 驱动：持有 I2C 总线所有权（设备树注入）。
pub struct Bme280<I2C> {
    i2c: I2C,
    addr: u8,
    dig_t1: u16,
    dig_t2: i16,
    dig_t3: i16,
    dig_h1: u8,
    dig_h2: i16,
    dig_h3: i8,
    dig_h4: i16,
    dig_h5: i16,
    dig_h6: i8,
    t_fine: i32,
}

impl<I2C: I2c> Bme280<I2C> {
    /// 设备树约定入口：reset → 校验芯片 ID → 读取校准参数 → 配置过采样。
    ///
    /// 需要的节点属性：`addr`（如 `0x76`）。
    pub async fn init(mut i2c: I2C, node: &NodeDesc) -> Result<Self, DeviceError> {
        let addr = node
            .prop("addr")
            .and_then(|p| p.as_u32())
            .map(|a| a as u8)
            .ok_or(DeviceError::InvalidProp("addr"))?;

        i2c.write(addr, &[REG_RESET, RESET_CMD])
            .await
            .map_err(|_| DeviceError::msg("bme280: reset failed"))?;

        let mut id = [0u8; 1];
        i2c.write_read(addr, &[REG_ID], &mut id)
            .await
            .map_err(|_| DeviceError::msg("bme280: chip id read failed"))?;
        if id[0] != CHIP_ID {
            return Err(DeviceError::msg("bme280: unexpected chip id"));
        }

        let mut calib1 = [0u8; 26];
        i2c.write_read(addr, &[REG_CALIB1], &mut calib1)
            .await
            .map_err(|_| DeviceError::msg("bme280: calibration read failed"))?;
        let dig_t1 = u16::from_le_bytes([calib1[0], calib1[1]]);
        let dig_t2 = i16::from_le_bytes([calib1[2], calib1[3]]);
        let dig_t3 = i16::from_le_bytes([calib1[4], calib1[5]]);
        let dig_h1 = calib1[25];

        let mut calib2 = [0u8; 7];
        i2c.write_read(addr, &[REG_CALIB2], &mut calib2)
            .await
            .map_err(|_| DeviceError::msg("bme280: calibration read failed"))?;
        let dig_h2 = i16::from_le_bytes([calib2[0], calib2[1]]);
        let dig_h3 = calib2[2] as i8;
        let dig_h4 = ((calib2[3] as i16) << 4) | (calib2[4] & 0x0F) as i16;
        let dig_h5 = ((calib2[5] as i16) << 4) | ((calib2[4] >> 4) as i16);
        let dig_h6 = calib2[6] as i8;

        // 过采样 x1 + 常开模式。
        i2c.write(addr, &[REG_CTRL_HUM, 0x01])
            .await
            .map_err(|_| DeviceError::msg("bme280: config failed"))?;
        i2c.write(addr, &[REG_CTRL_MEAS, 0x27])
            .await
            .map_err(|_| DeviceError::msg("bme280: config failed"))?;

        Ok(Self {
            i2c,
            addr,
            dig_t1,
            dig_t2,
            dig_t3,
            dig_h1,
            dig_h2,
            dig_h3,
            dig_h4,
            dig_h5,
            dig_h6,
            t_fine: 0,
        })
    }

    /// 温度，单位 0.01 °C。
    pub async fn temperature(&mut self) -> Result<i32, DeviceError> {
        let mut buf = [0u8; 3];
        self.i2c
            .write_read(self.addr, &[REG_TEMP], &mut buf)
            .await
            .map_err(|_| DeviceError::msg("bme280: temperature read failed"))?;
        let adc = ((buf[0] as i32) << 12) | ((buf[1] as i32) << 4) | ((buf[2] as i32) >> 4);
        Ok(self.compensate_t(adc))
    }

    /// 相对湿度，单位 0.001 %RH。
    pub async fn humidity(&mut self) -> Result<i32, DeviceError> {
        let mut buf = [0u8; 2];
        self.i2c
            .write_read(self.addr, &[REG_HUM], &mut buf)
            .await
            .map_err(|_| DeviceError::msg("bme280: humidity read failed"))?;
        let adc = ((buf[0] as i32) << 8) | buf[1] as i32;
        Ok(self.compensate_h(adc))
    }

    /// BME280 数据手册整数补偿公式。
    fn compensate_t(&mut self, adc: i32) -> i32 {
        let var1 = (((adc >> 3) - ((self.dig_t1 as i32) << 1)) * (self.dig_t2 as i32)) >> 11;
        let var2 = (((((adc >> 4) - self.dig_t1 as i32) * ((adc >> 4) - self.dig_t1 as i32)) >> 12)
            * self.dig_t3 as i32)
            >> 14;
        self.t_fine = var1 + var2;
        (self.t_fine * 5 + 128) >> 8
    }

    fn compensate_h(&mut self, adc: i32) -> i32 {
        // 数据手册公式的整数版；用 i64 避免饱和输入下的中间值溢出。
        let mut v1 = self.t_fine as i64 - 76800;
        let a = ((adc as i64) << 14)
            - ((self.dig_h4 as i64) << 20)
            - (self.dig_h5 as i64) * v1
            + 16384;
        let b = ((((((v1 * self.dig_h6 as i64) >> 10)
            * (((v1 * self.dig_h3 as i64) >> 11) + 32768))
            >> 10)
            + 2097152)
            * self.dig_h2 as i64
            + 8192)
            >> 14;
        v1 = (a >> 15) * b;
        v1 -= ((((v1 >> 15) * (v1 >> 15)) >> 7) * self.dig_h1 as i64) >> 4;
        v1 = v1.clamp(0, 419_430_400);
        (v1 >> 12) as i32
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // 测试的 block_on 需要手动构造 Waker
mod tests {
    use super::*;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    use embassy_dt::{NodeDesc, NodeKind, Prop};
    use embedded_hal_async::i2c::{ErrorType, Operation};

    fn block_on<F: Future>(fut: F) -> F::Output {
        fn noop_raw(_: *const ()) -> RawWaker {
            RawWaker::new(core::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(noop_raw, |_| {}, |_| {}, |_| {});
        let waker = unsafe { Waker::from_raw(noop_raw(core::ptr::null())) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = pin!(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => core::hint::spin_loop(),
            }
        }
    }

    /// 内存寄存器模拟的 I2C 总线。
    struct MockI2c {
        regs: [u8; 256],
    }

    impl ErrorType for MockI2c {
        type Error = core::convert::Infallible;
    }

    impl I2c for MockI2c {
        async fn transaction<'a>(
            &mut self,
            _address: u8,
            operations: &mut [Operation<'a>],
        ) -> Result<(), Self::Error> {
            let mut reg = 0u8;
            for op in operations {
                match op {
                    Operation::Write(buf) => {
                        if buf.len() >= 2 {
                            self.regs[buf[0] as usize] = buf[1];
                        }
                        reg = buf[0];
                    }
                    Operation::Read(buf) => {
                        for (i, b) in buf.iter_mut().enumerate() {
                            *b = self.regs[(reg as usize + i) & 0xFF];
                        }
                    }
                }
            }
            Ok(())
        }
    }

    /// BME280 数据手册补偿公式示例值。
    fn mock_with_datasheet_sample() -> MockI2c {
        let mut mock = MockI2c { regs: [0; 256] };
        mock.regs[REG_ID as usize] = CHIP_ID;
        // 校准参数（datasheet 4.4.1 示例）
        let t1 = 27_504u16.to_le_bytes();
        let t2 = 26_435i16.to_le_bytes();
        let t3 = (-1000i16).to_le_bytes();
        mock.regs[0x88..0x8A].copy_from_slice(&t1);
        mock.regs[0x8A..0x8C].copy_from_slice(&t2);
        mock.regs[0x8C..0x8E].copy_from_slice(&t3);
        mock.regs[0xA1] = 75; // dig_H1
        let h2 = 363i16.to_le_bytes();
        mock.regs[0xE1..0xE3].copy_from_slice(&h2);
        mock.regs[0xE3] = 0; // dig_H3
        mock.regs[0xE4] = (388 << 4) as u8 & 0xF0 | 0x0F; // H4 高 4 位在 0xE5 低半字节
        // dig_H4 = (0xE4 << 4) | (0xE5 & 0x0F) = 388 → 0xE4 = 0x18, 0xE5 & 0x0F = 0x4
        mock.regs[0xE4] = 0x18;
        mock.regs[0xE5] = 0xF4; // 低 4 位 = 4，高 4 位给 H5
        // dig_H5 = (0xE6 << 4) | (0xE5 >> 4) = 255 → 0xE6 = 0x0F, 0xE5>>4 = 0xF
        mock.regs[0xE6] = 0x0F;
        mock.regs[0xE7] = 14; // dig_H6（datasheet 示例为正 14）
        // 原始采样值
        let adc_t = 519_888u32;
        mock.regs[0xFA] = ((adc_t >> 12) & 0xFF) as u8;
        mock.regs[0xFB] = ((adc_t >> 4) & 0xFF) as u8;
        mock.regs[0xFC] = ((adc_t << 4) & 0xFF) as u8;
        let adc_h = 290u16;
        mock.regs[0xFD] = ((adc_h >> 8) & 0xFF) as u8;
        mock.regs[0xFE] = (adc_h & 0xFF) as u8;
        mock
    }

    #[test]
    fn init_rejects_bad_chip_id() {
        let mut mock = MockI2c { regs: [0; 256] };
        mock.regs[REG_ID as usize] = 0x99;
        let node = NodeDesc::new(
            "bme280",
            NodeKind::Device,
            &[],
            &[("addr", Prop::U32(0x76))],
        );
        let err = block_on(Bme280::init(mock, &node)).err().unwrap();
        assert_eq!(err, DeviceError::msg("bme280: unexpected chip id"));
    }

    #[test]
    fn init_requires_addr_prop() {
        let mock = MockI2c { regs: [0; 256] };
        let node = NodeDesc::new("bme280", NodeKind::Device, &[], &[]);
        let err = block_on(Bme280::init(mock, &node)).err().unwrap();
        assert_eq!(err, DeviceError::InvalidProp("addr"));
    }

    #[test]
    fn compensates_temperature_datasheet_sample() {
        let mock = mock_with_datasheet_sample();
        let node = NodeDesc::new(
            "bme280",
            NodeKind::Device,
            &[],
            &[("addr", Prop::U32(0x76))],
        );
        let mut dev = block_on(Bme280::init(mock, &node)).unwrap();
        // datasheet 示例：adc_T=519888 → 25.08 °C
        assert_eq!(block_on(dev.temperature()).unwrap(), 2508);
    }

    #[test]
    fn compensates_humidity_synthetic_vectors() {
        let node = NodeDesc::new(
            "bme280",
            NodeKind::Device,
            &[],
            &[("addr", Prop::U32(0x76))],
        );

        // 向量 1：校准全零 + dig_H2=1 → t_fine=0；adc_h=16384 → H=256
        // （期望值由公式独立推导：v1=(16384<<14+16384)>>15*128=1048576，H=1048576>>12）
        let mut mock = MockI2c { regs: [0; 256] };
        mock.regs[REG_ID as usize] = CHIP_ID;
        mock.regs[0xE1] = 0x01; // dig_H2 = 1
        mock.regs[0xFD] = 0x40; // adc_h = 16384
        mock.regs[0xFE] = 0x00;
        let mut dev = block_on(Bme280::init(mock, &node)).unwrap();
        assert_eq!(block_on(dev.temperature()).unwrap(), 0);
        assert_eq!(block_on(dev.humidity()).unwrap(), 256);

        // 向量 2：datasheet 温度校准 + 湿度校准
        // dig_H1=75, H2=363, H3=0, H4=0, H5=0, H6=0；
        // adc_t=519888 → T=2508；adc_h=65535（满量程）→ 湿度饱和 102400（100%）
        let mut mock = MockI2c { regs: [0; 256] };
        mock.regs[REG_ID as usize] = CHIP_ID;
        mock.regs[0x88..0x8A].copy_from_slice(&27_504u16.to_le_bytes());
        mock.regs[0x8A..0x8C].copy_from_slice(&26_435i16.to_le_bytes());
        mock.regs[0x8C..0x8E].copy_from_slice(&(-1000i16).to_le_bytes());
        mock.regs[0xA1] = 75; // dig_H1
        mock.regs[0xE1..0xE3].copy_from_slice(&363i16.to_le_bytes()); // dig_H2
        mock.regs[0xFA] = ((519_888u32 >> 12) & 0xFF) as u8; // adc_t = 519888
        mock.regs[0xFB] = ((519_888u32 >> 4) & 0xFF) as u8;
        mock.regs[0xFC] = ((519_888u32 << 4) & 0xFF) as u8;
        mock.regs[0xFD] = 0xFF; // adc_h = 65535
        mock.regs[0xFE] = 0xFF;
        let mut dev = block_on(Bme280::init(mock, &node)).unwrap();
        assert_eq!(block_on(dev.temperature()).unwrap(), 2508);
        assert_eq!(block_on(dev.humidity()).unwrap(), 102_400);
    }
}
