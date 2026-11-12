#[cfg(test)]
mod round4 {
    use super::*;
    use std::sync::mpsc as thread_channel;

    fn field(bytes: &[u8]) -> Vec<libc::c_char> {
        bytes.iter().map(|&b| b as libc::c_char).collect()
    }

    #[test]
    fn every_nul_position_and_full_fields_remain_bounded() {
        let mut cases = 0;
        for width in [0, 1, 16, 1024] {
            for prefix in 0..=width {
                let mut bytes = vec![b'x'; width];
                if prefix < width {
                    bytes[prefix] = 0;
                    bytes[prefix + 1..].fill(0xff);
                }
                assert_eq!(cstr_field(&field(&bytes)), "x".repeat(prefix));
                cases += 1;
            }
        }
        assert_eq!(cases, 1045);
    }

    #[test]
    fn byte_values_and_utf8_edges_obey_first_nul() {
        for byte in 0..=255u8 {
            let expected = match byte {
                0 => String::new(),
                1..=127 => char::from(byte).to_string(),
                _ => "\u{fffd}".into(),
            };
            assert_eq!(cstr_field(&field(&[byte, 0, b'z'])), expected);
        }
        let cases: &[(&[u8], &str)] = &[
            (b"\xc3\xa9", "é"),
            (b"\xe2\x82\xac", "€"),
            (b"\xf0\x9f\x92\xbe", "💾"),
            (b"\xc3\0\xa9", "\u{fffd}"),
            (b"\xe2\x82\0\xac", "\u{fffd}"),
            (b"\xf0\x9f\x92\0\xbe", "\u{fffd}"),
            (b"a\xffb\0z", "a\u{fffd}b"),
            (b"\0\xf0\x9f\x92\xbe", ""),
            (b"\xc3\xa9\0\xff", "é"),
            (b"\xc0\xaf", "\u{fffd}\u{fffd}"),
        ];
        for &(bytes, expected) in cases {
            assert_eq!(cstr_field(&field(bytes)), expected);
        }
    }

    #[test]
    fn expired_tokens_cannot_complete_successive_notifications() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut allowed = Vec::new();
        for index in 0..4 {
            let expired_id = index * 2;
            acknowledge_sleep(&tx, Duration::ZERO, || allowed.push(expired_id));
            let PowerEvent::WillSleep { ack: expired } = rx.try_recv().unwrap() else {
                panic!("expected sleep event");
            };
            assert!(expired.is_closed());

            let (done_tx, done_rx) = thread_channel::channel();
            let event_tx = &tx;
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    acknowledge_sleep(event_tx, Duration::from_millis(150), || {
                        done_tx.send(expired_id + 1).unwrap();
                    });
                });
                let PowerEvent::WillSleep { ack: current } = rx.blocking_recv().unwrap() else {
                    panic!("expected next sleep event");
                };
                assert!(!current.is_closed());
                assert_eq!(expired.send(()), Err(()));
                assert!(!current.is_closed());
                assert!(matches!(
                    done_rx.try_recv(),
                    Err(thread_channel::TryRecvError::Empty)
                ));
                if index % 2 == 0 {
                    current.send(()).unwrap();
                } else {
                    drop(current);
                }
                allowed.push(done_rx.recv_timeout(Duration::from_secs(1)).unwrap());
            });
            assert!(matches!(
                done_rx.try_recv(),
                Err(thread_channel::TryRecvError::Disconnected)
            ));
            assert!(matches!(
                rx.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
        assert_eq!(allowed, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn closed_event_channel_repeats_do_not_reuse_queued_ack() {
        for drop_receiver in [false, true] {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let mut allowed = Vec::new();
            acknowledge_sleep(&tx, Duration::ZERO, || allowed.push(0));
            rx.close();
            if !drop_receiver {
                let PowerEvent::WillSleep { ack } = rx.try_recv().unwrap() else {
                    panic!("expected queued sleep event");
                };
                assert_eq!(ack.send(()), Err(()));
            }
            let mut receiver = Some(rx);
            if drop_receiver {
                drop(receiver.take());
            }
            for id in 1..=16 {
                let budget = [
                    Duration::ZERO,
                    Duration::from_millis(1),
                    MAX_ACK_WAIT,
                    Duration::MAX,
                ][(id - 1) % 4];
                acknowledge_sleep(&tx, budget, || allowed.push(id));
            }
            assert_eq!(allowed, (0..=16).collect::<Vec<_>>());
            if let Some(mut rx) = receiver {
                assert!(matches!(
                    rx.try_recv(),
                    Err(mpsc::error::TryRecvError::Disconnected)
                ));
            }
        }
    }
}
