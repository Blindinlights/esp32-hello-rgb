#![no_std]
use core::{fmt::Debug, slice::IterMut};

use esp_hal::{
    clock::Clocks,
    gpio::OutputPin,
    peripheral::Peripheral,
    rmt::{Error as RmtError, PulseCode, TxChannel, TxChannelConfig, TxChannelCreator},
};
use smart_leds_trait::{SmartLedsWrite, RGB8};

const SK68XX_CODE_PERIOD: u32 = 1250; // 800kHz
const SK68XX_T0H_NS: u32 = 400; // 300ns per SK6812 datasheet, 400 per WS2812. Some require >350ns for T0H. Others <500ns for T0H.
const SK68XX_T0L_NS: u32 = SK68XX_CODE_PERIOD - SK68XX_T0H_NS;
const SK68XX_T1H_NS: u32 = 850; // 900ns per SK6812 datasheet, 850 per WS2812. > 550ns is sometimes enough. Some require T1H >= 2 * T0H. Some require > 300ns T1L.
const SK68XX_T1L_NS: u32 = SK68XX_CODE_PERIOD - SK68XX_T1H_NS;

/// All types of errors that can happen during the conversion and transmission
/// of LED commands
#[derive(Debug)]
pub enum LedAdapterError {
    /// Raised in the event that the provided data container is not large enough
    BufferSizeExceeded,
    /// Raised if something goes wrong in the transmission,
    TransmissionError(RmtError),
}

/// Macro to allocate a buffer sized for a specific number of LEDs to be
/// addressed.
///
/// Attempting to use more LEDs that the buffer is configured for will result in
/// an `LedAdapterError:BufferSizeExceeded` error.
#[macro_export]
macro_rules! smartLedBuffer {
    ( $buffer_size: literal ) => {
        // The size we're assigning here is calculated as following
        //  (
        //   Nr. of LEDs
        //   * channels (r,g,b -> 3)
        //   * pulses per channel 8)
        //  ) + 1 additional pulse for the end delimiter
        [0u32; $buffer_size * 24 + 1]
    };
}

/// Adapter taking an RMT channel and a specific pin and providing RGB LED
/// interaction functionality using the `smart-leds` crate
pub struct SmartLedsAdapter<TX, const BUFFER_SIZE: usize>
where
    TX: TxChannel,
{
    channel: Option<TX>,
    rmt_buffer: [u32; BUFFER_SIZE],
    pulses: (u32, u32),
}

impl<'d, TX, const BUFFER_SIZE: usize> SmartLedsAdapter<TX, BUFFER_SIZE>
where
    TX: TxChannel,
{
    /// Create a new adapter object that drives the pin using the RMT channel.
    pub fn new<C, O>(
        channel: C,
        pin: impl Peripheral<P = O> + 'd,
        rmt_buffer: [u32; BUFFER_SIZE],
    ) -> SmartLedsAdapter<TX, BUFFER_SIZE>
    where
        O: OutputPin + 'd,
        C: TxChannelCreator<'d, TX, O>,
    {
        let config = TxChannelConfig {
            clk_divider: 1,
            idle_output_level: false,
            carrier_modulation: false,
            idle_output: true,

            ..TxChannelConfig::default()
        };

        let channel = channel.configure(pin, config).unwrap();

        // Assume the RMT peripheral is set up to use the APB clock
        let clocks = Clocks::get();
        let src_clock = clocks.apb_clock.to_MHz();

        Self {
            channel: Some(channel),
            rmt_buffer,
            pulses: (
                <u32 as PulseCode>::new(
                    true,
                    ((SK68XX_T0H_NS * src_clock) / 1000) as u16,
                    false,
                    ((SK68XX_T0L_NS * src_clock) / 1000) as u16,
                ),
                <u32 as PulseCode>::new(
                    true,
                    ((SK68XX_T1H_NS * src_clock) / 1000) as u16,
                    false,
                    ((SK68XX_T1L_NS * src_clock) / 1000) as u16,
                ),
            ),
        }
    }

    fn convert_rgb_to_pulse(
        value: RGB8,
        mut_iter: &mut IterMut<u32>,
        pulses: (u32, u32),
    ) -> Result<(), LedAdapterError> {
        Self::convert_rgb_channel_to_pulses(value.g, mut_iter, pulses)?;
        Self::convert_rgb_channel_to_pulses(value.r, mut_iter, pulses)?;
        Self::convert_rgb_channel_to_pulses(value.b, mut_iter, pulses)?;

        Ok(())
    }

    fn convert_rgb_channel_to_pulses(
        channel_value: u8,
        mut_iter: &mut IterMut<u32>,
        pulses: (u32, u32),
    ) -> Result<(), LedAdapterError> {
        for position in [128, 64, 32, 16, 8, 4, 2, 1] {
            *mut_iter.next().ok_or(LedAdapterError::BufferSizeExceeded)? =
                match channel_value & position {
                    0 => pulses.0,
                    _ => pulses.1,
                }
        }

        Ok(())
    }
}

impl<TX, const BUFFER_SIZE: usize> SmartLedsWrite for SmartLedsAdapter<TX, BUFFER_SIZE>
where
    TX: TxChannel,
{
    type Error = LedAdapterError;
    type Color = RGB8;

    /// Convert all RGB8 items of the iterator to the RMT format and
    /// add them to internal buffer, then start a singular RMT operation
    /// based on that buffer.
    fn write<T, I>(&mut self, iterator: T) -> Result<(), Self::Error>
    where
        T: IntoIterator<Item = I>,
        I: Into<Self::Color>,
    {
        // We always start from the beginning of the buffer
        let mut seq_iter = self.rmt_buffer.iter_mut();

        // Add all converted iterator items to the buffer.
        // This will result in an `BufferSizeExceeded` error in case
        // the iterator provides more elements than the buffer can take.
        for item in iterator {
            Self::convert_rgb_to_pulse(item.into(), &mut seq_iter, self.pulses)?;
        }

        // Finally, add an end element.
        *seq_iter.next().ok_or(LedAdapterError::BufferSizeExceeded)? = 0;

        // Perform the actual RMT operation. We use the u32 values here right away.
        let channel = self.channel.take().unwrap();

        match channel
            .transmit(&self.rmt_buffer)
            .map_err(LedAdapterError::TransmissionError)?
            .wait()
        {
            Ok(chan) => {
                self.channel = Some(chan);
                Ok(())
            }
            Err((e, chan)) => {
                self.channel = Some(chan);
                Err(LedAdapterError::TransmissionError(e))
            }
        }
    }
}