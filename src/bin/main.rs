#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_hal::prelude::*;
use esp_hal::rmt::Rmt;
use log::info;
use rgbled::{smartLedBuffer, SmartLedsAdapter};
use smart_leds::hsv::{hsv2rgb, Hsv};
use smart_leds::{brightness, gamma, SmartLedsWrite};

#[entry]
fn main() -> ! {
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });
    esp_println::logger::init_logger_from_env();
    esp_alloc::heap_allocator!(72 * 1024);
    let led = peripherals.GPIO8;
    let freq = 80.MHz();
    let rmt_buffer = smartLedBuffer!(1);
    let rmt = Rmt::new(peripherals.RMT, freq).unwrap();
    let mut led = SmartLedsAdapter::new(rmt.channel0, led, rmt_buffer);
    let delay = Delay::new();
    let mut color = Hsv {
        hue: 0,
        sat: 255,
        val: 255,
    };
    let mut data;
    info!("Entering main loop...");
    loop {
        for hue in 0..=255 {
            color.hue = hue;
            // Convert from the HSV color space (where we can easily transition from one
            // color to the other) to the RGB color space that we can then send to the LED
            data = [hsv2rgb(color)];
            // When sending to the LED, we do a gamma correction first (see smart_leds
            // documentation for details) and then limit the brightness to 10 out of 255 so
            // that the output it's not too bright.
            led.write(brightness(gamma(data.iter().cloned()), 10))
                .unwrap();
            delay.delay_millis(20);
        }
    }
}
