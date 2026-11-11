//! System sleep/wake notifications via IOKit.
//!
//! macOS asks registered clients for permission before an idle sleep and tells
//! them just before a forced one. Both arrive on a CFRunLoop, so this module
//! owns a dedicated thread running one, and forwards events to the engine over
//! a channel.
//!
//! The contract that matters: **every notification must be acknowledged with
//! `IOAllowPowerChange`**, on every path including errors and timeouts. Failing
//! to acknowledge stalls the whole system's sleep for ~30 seconds and then
//! sleeps anyway, so there is nothing to gain by withholding it — we take a
//! bounded moment to unmount cleanly, then always allow the change.
//!
//! Wake is also detected here rather than only by the monotonic-vs-wall-clock
//! heuristic in the engine, which cannot distinguish a sleep from an NTP step.

use std::ffi::c_void;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::warn;

#[derive(Debug)]
pub enum PowerEvent {
    /// The system intends to sleep. The daemon should unmount promptly and
    /// drop `ack` (or send on it) when finished; the acknowledgement fires
    /// either way once the budget elapses.
    WillSleep { ack: oneshot::Sender<()> },
    /// The system has finished waking.
    DidWake,
}

// ---------------------------------------------------------------------------
// IOKit / CoreFoundation FFI
// ---------------------------------------------------------------------------

#[allow(non_camel_case_types)]
type io_object_t = u32;
#[allow(non_camel_case_types)]
type io_service_t = io_object_t;
#[allow(non_camel_case_types)]
type io_connect_t = io_object_t;
type IONotificationPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;

type IOServiceInterestCallback = extern "C" fn(
    refcon: *mut c_void,
    service: io_service_t,
    message_type: u32,
    message_argument: *mut c_void,
);

/// The system is asking whether it may sleep; a client may veto. We never veto.
const K_IO_MESSAGE_CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
/// Sleep is happening. Last chance to act, and it is a short one.
const K_IO_MESSAGE_SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
/// A pending sleep was cancelled.
const K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP: u32 = 0xE000_0290;
/// The system is fully awake again.
const K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;
// Leave room before IOKit's roughly 30-second forced acknowledgement deadline.
const MAX_ACK_WAIT: Duration = Duration::from_secs(25);

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IORegisterForSystemPower(
        refcon: *mut c_void,
        the_port_ref: *mut IONotificationPortRef,
        callback: IOServiceInterestCallback,
        notifier: *mut io_object_t,
    ) -> io_connect_t;

    fn IONotificationPortGetRunLoopSource(notify: IONotificationPortRef) -> CFRunLoopSourceRef;

    fn IOAllowPowerChange(kernel_port: io_connect_t, notification_id: isize) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    static kCFRunLoopCommonModes: CFStringRef;
}

/// Shared with the C callback. Raw pointers are not `Send`, but this struct
/// only holds plain data and a channel, and lives for the process lifetime.
struct CallbackContext {
    root_port: io_connect_t,
    tx: mpsc::UnboundedSender<PowerEvent>,
}

extern "C" fn power_callback(
    refcon: *mut c_void,
    _service: io_service_t,
    message_type: u32,
    message_argument: *mut c_void,
) {
    // SAFETY: refcon is the Box::into_raw'd CallbackContext handed to
    // IORegisterForSystemPower, which outlives every callback.
    let ctx = unsafe { &*(refcon as *const CallbackContext) };
    let notification_id = message_argument as isize;

    match message_type {
        K_IO_MESSAGE_CAN_SYSTEM_SLEEP => {
            // Never veto: a daemon that blocks idle sleep is a battery bug.
            unsafe { IOAllowPowerChange(ctx.root_port, notification_id) };
        }
        K_IO_MESSAGE_SYSTEM_WILL_SLEEP => {
            acknowledge_sleep(&ctx.tx, MAX_ACK_WAIT, || {
                unsafe { IOAllowPowerChange(ctx.root_port, notification_id) };
            });
        }
        K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP | K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON => {
            let _ = ctx.tx.send(PowerEvent::DidWake);
        }
        _ => {}
    }
}

// Keep the acknowledgement in one place, including a closed engine channel.
fn acknowledge_sleep(
    tx: &mpsc::UnboundedSender<PowerEvent>,
    budget: Duration,
    allow: impl FnOnce(),
) {
    let (ack_tx, ack_rx) = oneshot::channel();
    if tx.send(PowerEvent::WillSleep { ack: ack_tx }).is_ok() {
        let _ = ack_rx.blocking_recv_timeout(budget.min(MAX_ACK_WAIT));
    }
    allow();
}

/// `oneshot::Receiver` has no blocking timeout, and the power callback runs on
/// a CFRunLoop thread with no tokio context, so wait the hard way.
trait BlockingRecvTimeout {
    fn blocking_recv_timeout(self, timeout: Duration) -> Result<(), ()>;
}

impl BlockingRecvTimeout for oneshot::Receiver<()> {
    fn blocking_recv_timeout(mut self, timeout: Duration) -> Result<(), ()> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.try_recv() {
                Ok(()) => return Ok(()),
                Err(oneshot::error::TryRecvError::Closed) => return Ok(()),
                Err(oneshot::error::TryRecvError::Empty) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

/// Register for system power notifications on a dedicated thread.
///
/// Returns the receiving end of the event stream. If registration fails the
/// channel simply stays silent: the engine still detects wake by the
/// monotonic-clock gap, so power notifications are an upgrade, not a
/// dependency.
pub fn watch() -> mpsc::UnboundedReceiver<PowerEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::Builder::new()
        .name("mountd-power".into())
        .spawn(move || {
            let mut notify_port: IONotificationPortRef = std::ptr::null_mut();
            let mut notifier: io_object_t = 0;

            let ctx = Box::new(CallbackContext { root_port: 0, tx });
            let ctx_ptr = Box::into_raw(ctx);

            // SAFETY: all pointers are valid for the duration of the call, and
            // ctx_ptr is deliberately leaked — the callback uses it for the
            // lifetime of the process.
            let root_port = unsafe {
                IORegisterForSystemPower(
                    ctx_ptr as *mut c_void,
                    &mut notify_port,
                    power_callback,
                    &mut notifier,
                )
            };

            if root_port == 0 || notify_port.is_null() {
                warn!(
                    "could not register for system power notifications; \
                       falling back to clock-gap wake detection"
                );
                // SAFETY: registration failed, so no callback can be running.
                unsafe { drop(Box::from_raw(ctx_ptr)) };
                return;
            }

            // SAFETY: ctx_ptr is uniquely owned here; no callback can fire
            // before the run loop source is added below.
            unsafe { (*ctx_ptr).root_port = root_port };

            // SAFETY: notify_port is non-null and this thread owns the run loop.
            unsafe {
                let source = IONotificationPortGetRunLoopSource(notify_port);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                CFRunLoopRun();
            }
        })
        .map_err(|e| warn!("could not start the power-notification thread: {e}"))
        .ok();

    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constants_match_iokit() {
        // Copied from IOKit/pwr_mgt/IOPMLib.h; a typo here would silently mean
        // "we never hear about sleep", which is invisible until data is lost.
        assert_eq!(K_IO_MESSAGE_CAN_SYSTEM_SLEEP, 0xE0000270);
        assert_eq!(K_IO_MESSAGE_SYSTEM_WILL_SLEEP, 0xE0000280);
        assert_eq!(K_IO_MESSAGE_SYSTEM_WILL_NOT_SLEEP, 0xE0000290);
        assert_eq!(K_IO_MESSAGE_SYSTEM_HAS_POWERED_ON, 0xE0000300);
    }

    #[test]
    fn ack_wait_returns_promptly_when_the_engine_finishes() {
        let (tx, rx) = oneshot::channel();
        let start = std::time::Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            let _ = tx.send(());
        });
        assert!(rx.blocking_recv_timeout(Duration::from_secs(5)).is_ok());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn ack_wait_gives_up_after_the_budget() {
        // The whole point: a wedged unmount must not hold system sleep open.
        let (_tx, rx) = oneshot::channel::<()>();
        let start = std::time::Instant::now();
        assert!(rx
            .blocking_recv_timeout(Duration::from_millis(100))
            .is_err());
        assert!(start.elapsed() >= Duration::from_millis(100));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn ack_wait_treats_a_dropped_sender_as_done() {
        // The engine dropping the ack without sending (e.g. it panicked or had
        // nothing to unmount) must not cost the full budget.
        let (tx, rx) = oneshot::channel::<()>();
        drop(tx);
        let start = std::time::Instant::now();
        assert!(rx.blocking_recv_timeout(Duration::from_secs(5)).is_ok());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn sleep_is_acknowledged_once_when_engine_is_gone_or_times_out() {
        for closed in [true, false] {
            let (tx, rx) = mpsc::unbounded_channel();
            let _receiver = if closed {
                drop(rx);
                None
            } else {
                Some(rx)
            };
            let mut calls = 0;
            acknowledge_sleep(&tx, Duration::from_millis(25), || calls += 1);
            assert_eq!(calls, 1);
        }
    }

    #[test]
    fn sleep_is_acknowledged_when_engine_drops_ack_after_error() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let engine = std::thread::spawn(move || {
            if let Some(PowerEvent::WillSleep { ack }) = rx.blocking_recv() {
                drop(ack);
            }
        });
        let mut calls = 0;
        acknowledge_sleep(&tx, Duration::from_secs(1), || calls += 1);
        engine.join().unwrap();
        assert_eq!(calls, 1);
    }
}
