//! Independent native capability oracle. Build with rustc --test --edition=2024.
//! Every network endpoint is loopback/temporary. No external service is contacted.
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering, fence};
use std::sync::{Arc, Barrier, Condvar, Mutex, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

fn accept_before_deadline(listener: &TcpListener) -> io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Windows can inherit nonblocking mode from the listener.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(TIMEOUT))?;
                stream.set_write_timeout(Some(TIMEOUT))?;
                return Ok(stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
}

fn connect(listener: &TcpListener) -> io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&listener.local_addr()?, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    Ok(stream)
}

// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN atomic_increment
// A structured FluxR version must lower to these operations with these orderings.
fn atomic_increment(counter: &AtomicUsize, iterations: usize) {
    for _ in 0..iterations {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END atomic_increment

// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN thread_value
fn native_thread_value() -> usize { std::thread::spawn(move || 42usize).join().unwrap() }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END thread_value

// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN tcp_write
fn native_tcp_write(stream: &mut TcpStream, bytes: &[u8]) -> io::Result<()> { stream.write_all(bytes) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END tcp_write

#[test]
fn thread_spawn_join_return_and_panic() {
    assert_eq!(native_thread_value(), 42);
    let handle = thread::Builder::new().name("fluxr-native-oracle".into())
        .stack_size(1024 * 1024).spawn(|| {
            assert_eq!(thread::current().name(), Some("fluxr-native-oracle"));
            40usize + 2
        }).unwrap();
    assert_eq!(handle.join().unwrap(), 42);
    let failed = thread::spawn(|| panic!("intentional native thread panic"));
    let payload = failed.join().unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"intentional native thread panic"));
    assert!(thread::available_parallelism().unwrap().get() >= 1);
}

#[test]
fn scoped_threads_borrow_disjoint_slices() {
    let mut values = [0usize; 256];
    thread::scope(|scope| {
        for (index, chunk) in values.chunks_mut(32).enumerate() {
            scope.spawn(move || chunk.fill(index + 1));
        }
    });
    assert_eq!(values.iter().sum::<usize>(), 32 * (1 + 2 + 3 + 4 + 5 + 6 + 7 + 8));
}

#[test]
fn arc_weak_lifetime_and_drop_exactly_once() {
    struct CountDrop(Arc<AtomicUsize>);
    impl Drop for CountDrop { fn drop(&mut self) { self.0.fetch_add(1, Ordering::SeqCst); } }
    let drops = Arc::new(AtomicUsize::new(0));
    let value = Arc::new(CountDrop(Arc::clone(&drops)));
    let weak = Arc::downgrade(&value);
    let copy = Arc::clone(&value);
    assert_eq!(Arc::strong_count(&value), 2);
    thread::spawn(move || drop(copy)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(weak.upgrade().is_some());
    drop(value);
    assert!(weak.upgrade().is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn mutex_poison_recovery_and_concurrent_mutation() {
    let value = Arc::new(Mutex::new(0usize));
    let writer = Arc::clone(&value);
    assert!(thread::spawn(move || {
        let mut guard = writer.lock().unwrap();
        *guard = 7;
        panic!("intentional poison");
    }).join().is_err());
    assert!(value.is_poisoned());
    {
        let mut guard = value.lock().unwrap_err().into_inner();
        assert_eq!(*guard, 7);
        *guard = 0;
        value.clear_poison();
    }
    thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| { for _ in 0..2_000 { *value.lock().unwrap() += 1; } });
        }
    });
    assert_eq!(*value.lock().unwrap(), 8_000);
}

#[test]
fn rwlock_reader_exclusion_writer_visibility() {
    let value = Arc::new(RwLock::new(vec![1usize, 2, 3]));
    let first = value.read().unwrap();
    let second = value.read().unwrap();
    assert_eq!(first.len() + second.len(), 6);
    assert!(value.try_write().is_err());
    drop(first);
    drop(second);
    let writer = Arc::clone(&value);
    thread::spawn(move || writer.write().unwrap().push(4)).join().unwrap();
    assert_eq!(*value.read().unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn barrier_synchronizes_all_participants_and_one_leader() {
    let barrier = Barrier::new(8);
    let values = (0..8).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>();
    let leaders = AtomicUsize::new(0);
    thread::scope(|scope| {
        for index in 0..8 {
            let (barrier, values, leaders) = (&barrier, &values, &leaders);
            scope.spawn(move || {
                values[index].store(index + 1, Ordering::Relaxed);
                if barrier.wait().is_leader() { leaders.fetch_add(1, Ordering::Relaxed); }
                assert_eq!(values.iter().map(|v| v.load(Ordering::Relaxed)).sum::<usize>(), 36);
            });
        }
    });
    assert_eq!(leaders.load(Ordering::Relaxed), 1);
}

#[test]
fn condvar_predicate_notification_and_timeout() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let setter = Arc::clone(&pair);
    let handle = thread::spawn(move || {
        let mut state = setter.0.lock().unwrap();
        *state = true;
        setter.1.notify_one();
    });
    let (guard, result) = pair.1.wait_timeout_while(pair.0.lock().unwrap(), TIMEOUT, |ready| !*ready).unwrap();
    assert!(*guard);
    assert!(!result.timed_out());
    drop(guard);
    handle.join().unwrap();
    let (mut guard, result) = pair.1.wait_timeout_while(pair.0.lock().unwrap(), Duration::from_millis(5), |ready| *ready).unwrap();
    assert!(result.timed_out());
    *guard = false;
}

#[test]
fn bounded_channel_backpressure_fifo_and_disconnect() {
    let (tx, rx) = mpsc::sync_channel(2);
    tx.send(10).unwrap();
    tx.send(20).unwrap();
    assert_eq!(tx.try_send(30), Err(mpsc::TrySendError::Full(30)));
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), 10);
    tx.try_send(30).unwrap();
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), 20);
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), 30);
    assert_eq!(rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    drop(tx);
    assert_eq!(rx.recv_timeout(TIMEOUT), Err(mpsc::RecvTimeoutError::Disconnected));
    let (tx, rx) = mpsc::sync_channel::<usize>(1);
    drop(rx);
    assert_eq!(tx.send(4), Err(mpsc::SendError(4)));
}

#[test]
fn rendezvous_channel_moves_owned_values() {
    let (tx, rx) = mpsc::sync_channel(0);
    let worker = thread::spawn(move || tx.send(vec![1u8, 2, 3, 4]).unwrap());
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), vec![1, 2, 3, 4]);
    worker.join().unwrap();
}

macro_rules! atomic_integer_case {
    ($name:ident, $atomic:ty) => {
        #[test]
        #[allow(deprecated)] // Legacy method remains part of the Rust API surface.
        fn $name() {
            let value = <$atomic>::new(4);
            assert_eq!(value.load(Ordering::Relaxed), 4);
            value.store(5, Ordering::Release);
            assert_eq!(value.load(Ordering::Acquire), 5);
            assert_eq!(value.swap(10, Ordering::AcqRel), 5);
            assert_eq!(value.fetch_add(3, Ordering::Relaxed), 10);
            assert_eq!(value.fetch_sub(2, Ordering::SeqCst), 13);
            assert_eq!(value.fetch_and(7, Ordering::AcqRel), 11);
            assert_eq!(value.fetch_or(8, Ordering::Acquire), 3);
            assert_eq!(value.fetch_xor(1, Ordering::Release), 11);
            assert_eq!(value.fetch_max(15, Ordering::SeqCst), 10);
            assert_eq!(value.fetch_min(12, Ordering::SeqCst), 15);
            assert_eq!(value.compare_exchange(12, 19, Ordering::AcqRel, Ordering::Acquire), Ok(12));
            assert_eq!(value.compare_exchange(12, 22, Ordering::SeqCst, Ordering::Relaxed), Err(19));
            assert_eq!(value.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1)), Ok(19));
            assert_eq!(value.fetch_update(Ordering::SeqCst, Ordering::Relaxed, |_| None), Err(20));
            assert_eq!(value.into_inner(), 20);
        }
    };
}
atomic_integer_case!(atomic_u8_operations, std::sync::atomic::AtomicU8);
atomic_integer_case!(atomic_i8_operations, std::sync::atomic::AtomicI8);
atomic_integer_case!(atomic_u16_operations, std::sync::atomic::AtomicU16);
atomic_integer_case!(atomic_i16_operations, std::sync::atomic::AtomicI16);
atomic_integer_case!(atomic_u32_operations, std::sync::atomic::AtomicU32);
atomic_integer_case!(atomic_i32_operations, std::sync::atomic::AtomicI32);
atomic_integer_case!(atomic_usize_operations, std::sync::atomic::AtomicUsize);
atomic_integer_case!(atomic_isize_operations, std::sync::atomic::AtomicIsize);
#[cfg(target_has_atomic = "64")]
atomic_integer_case!(atomic_u64_operations, std::sync::atomic::AtomicU64);
#[cfg(target_has_atomic = "64")]
atomic_integer_case!(atomic_i64_operations, std::sync::atomic::AtomicI64);

#[test]
fn atomic_bool_pointer_and_fences() {
    let value = AtomicBool::new(false);
    assert!(!value.swap(true, Ordering::SeqCst));
    assert!(value.fetch_xor(true, Ordering::AcqRel));
    assert_eq!(value.compare_exchange(false, true, Ordering::Release, Ordering::Relaxed), Ok(false));
    fence(Ordering::SeqCst);
    std::sync::atomic::compiler_fence(Ordering::AcqRel);
    let mut a = 17usize;
    let mut b = 29usize;
    let pa = &mut a as *mut usize;
    let pb = &mut b as *mut usize;
    let pointer = AtomicPtr::new(pa);
    assert_eq!(pointer.swap(pb, Ordering::AcqRel), pa);
    assert_eq!(pointer.load(Ordering::Acquire), pb);
    assert_eq!(pointer.compare_exchange(pb, pa, Ordering::SeqCst, Ordering::Acquire), Ok(pb));
    assert_eq!(pointer.into_inner(), pa);
}

#[test]
fn release_acquire_publication() {
    let published = AtomicBool::new(false);
    let payload = AtomicUsize::new(0);
    thread::scope(|scope| {
        scope.spawn(|| {
            payload.store(0x123456, Ordering::Relaxed);
            published.store(true, Ordering::Release);
        });
        scope.spawn(|| {
            let deadline = Instant::now() + TIMEOUT;
            while !published.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline, "publication deadline exceeded");
                std::hint::spin_loop();
            }
            assert_eq!(payload.load(Ordering::Relaxed), 0x123456);
        });
    });
}

#[test]
fn cas_weak_contention_does_not_lose_updates() {
    let counter = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..2_000 {
                    let mut old = counter.load(Ordering::Relaxed);
                    loop {
                        match counter.compare_exchange_weak(old, old + 1, Ordering::AcqRel, Ordering::Acquire) {
                            Ok(_) => break,
                            Err(observed) => old = observed,
                        }
                    }
                }
            });
        }
    });
    assert_eq!(counter.load(Ordering::Acquire), 16_000);
}

#[test]
fn fetch_add_contention_and_seqcst_unique_tickets() {
    let counter = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..8 { let counter = &counter; scope.spawn(move || atomic_increment(counter, 5_000)); }
    });
    assert_eq!(counter.load(Ordering::Relaxed), 40_000);
    let ticket = AtomicUsize::new(0);
    let mut all = thread::scope(|scope| {
        let handles = (0..4).map(|_| {
            let ticket = &ticket;
            scope.spawn(move || (0..1_000).map(|_| ticket.fetch_add(1, Ordering::SeqCst)).collect::<Vec<_>>())
        }).collect::<Vec<_>>();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect::<Vec<_>>()
    });
    all.sort_unstable();
    assert_eq!(all, (0..4_000).collect::<Vec<_>>());
}

#[test]
fn tcp_fragmented_io_half_close_and_socket_options() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client = connect(&listener).unwrap();
    let worker = thread::spawn(move || {
        let mut peer = accept_before_deadline(&listener).unwrap();
        peer.set_nodelay(true).unwrap();
        assert!(peer.nodelay().unwrap());
        let mut request = Vec::new();
        peer.read_to_end(&mut request).unwrap();
        assert_eq!(request, b"fragmented request payload");
        for chunk in b"complete response body".chunks(3) { peer.write_all(chunk).unwrap(); }
        peer.shutdown(Shutdown::Write).unwrap();
    });
    let mut client = client;
    let clone = client.try_clone().unwrap();
    assert_eq!(clone.peer_addr().unwrap(), client.peer_addr().unwrap());
    assert_ne!(client.peer_addr().unwrap(), client.local_addr().unwrap());
    drop(clone);
    for chunk in b"fragmented request payload".chunks(2) { native_tcp_write(&mut client, chunk).unwrap(); }
    client.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).unwrap();
    assert_eq!(response, b"complete response body");
    worker.join().unwrap();
}

#[test]
fn tcp_nonblocking_read_and_accept_report_would_block() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(listener.accept().unwrap_err().kind(), io::ErrorKind::WouldBlock);
    let mut client = connect(&listener).unwrap();
    let peer = accept_before_deadline(&listener).unwrap();
    client.set_nonblocking(true).unwrap();
    let error = client.read(&mut [0u8; 1]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    client.set_nonblocking(false).unwrap();
    drop(peer);
    assert_eq!(client.read(&mut [0u8; 1]).unwrap(), 0);
}

#[test]
fn tcp_read_timeout_is_observable() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut client = connect(&listener).unwrap();
    let peer = accept_before_deadline(&listener).unwrap();
    client.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    let error = client.read(&mut [0u8; 1]).unwrap_err();
    assert!(matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut));
    drop(peer);
}

#[test]
fn udp_datagrams_and_connected_peers() {
    let a = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let b = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    for socket in [&a, &b] { socket.set_read_timeout(Some(TIMEOUT)).unwrap(); socket.set_write_timeout(Some(TIMEOUT)).unwrap(); }
    a.connect(b.local_addr().unwrap()).unwrap();
    assert_eq!(a.send(b"one datagram").unwrap(), 12);
    let mut buffer = [0u8; 64];
    let (count, from) = b.recv_from(&mut buffer).unwrap();
    assert_eq!(&buffer[..count], b"one datagram");
    assert_eq!(from, a.local_addr().unwrap());
    b.send_to(b"reply", from).unwrap();
    let count = a.recv(&mut buffer).unwrap();
    assert_eq!(&buffer[..count], b"reply");
}

#[cfg(unix)]
#[test]
fn unix_stream_pair() {
    use std::os::unix::net::UnixStream;
    let (mut a, mut b) = UnixStream::pair().unwrap();
    a.set_read_timeout(Some(TIMEOUT)).unwrap();
    b.set_read_timeout(Some(TIMEOUT)).unwrap();
    a.write_all(b"local bytes").unwrap();
    a.shutdown(Shutdown::Write).unwrap();
    let mut data = Vec::new();
    b.read_to_end(&mut data).unwrap();
    assert_eq!(data, b"local bytes");
}

#[cfg(unix)]
#[test]
fn unix_filesystem_listener() {
    use std::os::unix::net::{UnixListener, UnixStream};
    let directory = std::env::temp_dir().join(format!("fluxr-unix-oracle-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(directory.clone());
    let path = directory.join("test.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let mut client = UnixStream::connect(&path).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(TIMEOUT)).unwrap();
    client.write_all(b"1234").unwrap();
    let mut response = [0u8; 4];
    peer.read_exact(&mut response).unwrap();
    assert_eq!(&response, b"1234");
}

#[test]
fn future_pin_poll_waker_and_thread_wakeup() {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Wake, Waker};
    struct Signal(Mutex<bool>, Condvar);
    impl Wake for Signal {
        fn wake(self: Arc<Self>) {
            *self.0.lock().unwrap() = true;
            self.1.notify_one();
        }
    }
    struct Later { value: Arc<Mutex<Option<usize>>>, waiter: Arc<Mutex<Option<Waker>>> }
    impl Future for Later {
        type Output = usize;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<usize> {
            let mut waiter = self.waiter.lock().unwrap();
            if let Some(value) = self.value.lock().unwrap().take() { Poll::Ready(value) }
            else { *waiter = Some(cx.waker().clone()); Poll::Pending }
        }
    }
    let value = Arc::new(Mutex::new(None));
    let waiter = Arc::new(Mutex::new(None::<Waker>));
    let signal = Arc::new(Signal(Mutex::new(false), Condvar::new()));
    let waker = Waker::from(Arc::clone(&signal));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(Later { value: Arc::clone(&value), waiter: Arc::clone(&waiter) });
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    let worker = thread::spawn(move || {
        let mut waiter = waiter.lock().unwrap();
        *value.lock().unwrap() = Some(42);
        waiter.take().unwrap().wake();
    });
    let (guard, timeout) = signal.1.wait_timeout_while(signal.0.lock().unwrap(), TIMEOUT, |ready| !*ready).unwrap();
    assert!(*guard && !timeout.timed_out());
    drop(guard);
    assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(42));
    worker.join().unwrap();
}
