//! Segger RTT implementation.
//!
//! RTT is a protocol for sending debugging messages to a connected host. The
//! embedded platform configures a portion of memory in a special way, and then
//! the host uses a JTAG connection to read the messages out of the chip's
//! memory.
//!
//! Usage
//! -----
//!
//! ```
//! let virtual_alarm_rtt = static_init!(
//!     capsules::virtual_alarm::VirtualMuxAlarm<'static, nrf5x::rtc::Rtc>,
//!     capsules::virtual_alarm::VirtualMuxAlarm::new(mux_alarm)
//! );
//!
//! let rtt_memory = static_init!(
//!     capsules::segger_rtt::SeggerRttMemory,
//!     capsules::segger_rtt::SeggerRttMemory::new(b"Terminal\0",
//!         &mut capsules::segger_rtt::UP_BUFFER,
//!         b"Terminal\0",
//!         &mut capsules::segger_rtt::DOWN_BUFFER)
//! );
//!
//! let rtt = static_init!(
//!     capsules::segger_rtt::SeggerRtt<VirtualMuxAlarm<'static, nrf5x::rtc::Rtc>>,
//!     capsules::segger_rtt::SeggerRtt::new(virtual_alarm_rtt, rtt_memory,
//!         &mut capsules::segger_rtt::UP_BUFFER,
//!         &mut capsules::segger_rtt::DOWN_BUFFER)
//! );
//! virtual_alarm_rtt.set_client(rtt);
//! ```

use kernel::common::cells::{OptionalCell, TakeCell};
use kernel::hil;
use kernel::hil::time::Frequency;

/// Buffer for transmitting to the host.
pub static mut UP_BUFFER: [u8; 1024] = [0; 1024];

/// Buffer for receiving messages from the host.
pub static mut DOWN_BUFFER: [u8; 32] = [0; 32];

/// This structure is defined by the segger RTT protocol.
#[repr(C)]
pub struct SeggerRttMemory {
    id: [u8; 16],
    number_up_buffers: u32,
    number_down_buffers: u32,
    up_buffer: SeggerRttBuffer,
    down_buffer: SeggerRttBuffer,
}

#[repr(C)]
pub struct SeggerRttBuffer {
    name: *const u8, // Pointer to the name of this channel. Must be a 4 byte thin pointer.
    buffer: *const u8, // Pointer to the buffer for this channel.
    length: u32,
    write_position: u32,
    read_position: u32,
    flags: u32,
}

impl SeggerRttMemory {
    pub fn new(
        up_buffer_name: &'static [u8],
        up_buffer: &'static [u8],
        down_buffer_name: &'static [u8],
        down_buffer: &'static [u8],
    ) -> SeggerRttMemory {
        SeggerRttMemory {
            // Must be "SEGGER RTT".
            id: [
                0x53, 0x45, 0x47, 0x47, 0x45, 0x52, 0x20, 0x52, 0x54, 0x54, 0, 0, 0, 0, 0, 0,
            ],
            number_up_buffers: 1,
            number_down_buffers: 1,
            up_buffer: SeggerRttBuffer {
                name: up_buffer_name.as_ptr(),
                buffer: up_buffer.as_ptr(),
                length: 1024,
                write_position: 0,
                read_position: 0,
                flags: 0,
            },
            down_buffer: SeggerRttBuffer {
                name: down_buffer_name.as_ptr(),
                buffer: down_buffer.as_ptr(),
                length: 32,
                write_position: 0,
                read_position: 0,
                flags: 0,
            },
        }
    }
}

pub struct SeggerRtt<'a, A: hil::time::Alarm + 'a> {
    alarm: &'a A, // Dummy alarm so we can get a callback.
    config: TakeCell<'static, SeggerRttMemory>,
    up_buffer: TakeCell<'static, [u8]>,
    _down_buffer: TakeCell<'static, [u8]>,
    client: OptionalCell<&'static hil::uart::Client>,
    client_buffer: TakeCell<'static, [u8]>,
}

impl<'a, A: hil::time::Alarm + 'a> SeggerRtt<'a, A> {
    pub fn new(
        alarm: &'a A,
        config: &'static mut SeggerRttMemory,
        up_buffer: &'static mut [u8],
        down_buffer: &'static mut [u8],
    ) -> SeggerRtt<'a, A> {
        SeggerRtt {
            alarm: alarm,
            config: TakeCell::new(config),
            up_buffer: TakeCell::new(up_buffer),
            _down_buffer: TakeCell::new(down_buffer),
            client: OptionalCell::empty(),
            client_buffer: TakeCell::empty(),
        }
    }
}

impl<'a, A: hil::time::Alarm + 'a> hil::uart::UART for SeggerRtt<'a, A> {
    fn set_client(&self, client: &'static hil::uart::Client) {
        self.client.set(client);
    }

    fn init(&self, _params: hil::uart::UARTParams) {}

    fn transmit(&self, tx_data: &'static mut [u8], tx_len: usize) {
        self.up_buffer.map(|buffer| {
            self.config.map(|config| {
                // Copy the incoming data into the buffer. Once we increment
                // the `write_position` the RTT listener will go ahead and read
                // the message from us.
                let mut index = config.up_buffer.write_position as usize;
                let buffer_len = config.up_buffer.length as usize;

                for i in 0..tx_len {
                    buffer[(i + index) % buffer_len] = tx_data[i];
                }

                index = (index + tx_len) % buffer_len;
                config.up_buffer.write_position = index as u32;
            });
        });

        // Save the client buffer so we can pass it back with the callback.
        self.client_buffer.replace(tx_data);

        // Start a short timer so that we get a callback and can issue the
        // callback to the client.
        let interval = (100 as u32) * <A::Frequency>::frequency() / 1000000;
        let tics = self.alarm.now().wrapping_add(interval);
        self.alarm.set_alarm(tics);
    }

    fn receive(&self, _rx_buf: &'static mut [u8], _rx_len: usize) {}

    fn abort_receive(&self) {}
}

impl<'a, A: hil::time::Alarm + 'a> hil::time::Client for SeggerRtt<'a, A> {
    fn fired(&self) {
        self.client.map(|client| {
            self.client_buffer.take().map(|buffer| {
                client.transmit_complete(buffer, hil::uart::Error::CommandComplete);
            });
        });
    }
}