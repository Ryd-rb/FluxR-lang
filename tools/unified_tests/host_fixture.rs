//! Trusted native services for independent unified-runtime integration tests.
//! The agent-facing callers are injected FluxR programs, not Rust substitutions.
use fluxr::runtime::{global, Limits, ModuleSnapshot, RuntimeError, Type, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicU64 = AtomicU64::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BAD_CALLS: AtomicU64 = AtomicU64::new(0);
static PINNED: OnceLock<Mutex<Option<ModuleSnapshot>>> = OnceLock::new();
const TIMEOUT: Duration = Duration::from_secs(3);

fn uint(arguments: &[Value], index: usize) -> u64 {
    match arguments.get(index) {
        Some(Value::U64(value)) => *value,
        _ => panic!("typed host argument was not validated"),
    }
}

fn tcp_roundtrip(value: u64) -> u64 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + TIMEOUT;
        let mut peer = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "TCP accept timed out");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("TCP accept failed: {error}"),
            }
        };
        peer.set_read_timeout(Some(TIMEOUT)).unwrap();
        peer.set_write_timeout(Some(TIMEOUT)).unwrap();
        let mut incoming = [0; 8];
        peer.read_exact(&mut incoming).unwrap();
        let mut end = [0];
        assert_eq!(peer.read(&mut end).unwrap(), 0);
        let result = u64::from_be_bytes(incoming).rotate_left(13) ^ 0x55aa_0123_4567_89ef;
        peer.write_all(&result.to_be_bytes()[..3]).unwrap();
        peer.write_all(&result.to_be_bytes()[3..]).unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
    });
    let mut client = TcpStream::connect_timeout(&address, TIMEOUT).unwrap();
    client.set_read_timeout(Some(TIMEOUT)).unwrap();
    client.set_write_timeout(Some(TIMEOUT)).unwrap();
    client.write_all(&value.to_be_bytes()[..2]).unwrap();
    client.write_all(&value.to_be_bytes()[2..]).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut result = [0; 8];
    client.read_exact(&mut result).unwrap();
    let mut end = [0];
    assert_eq!(client.read(&mut end).unwrap(), 0);
    worker.join().unwrap();
    u64::from_be_bytes(result)
}

fn threaded_sum(n: u64) -> u64 {
    assert!(n <= 200_000, "fixture work bound");
    let total = Arc::new(AtomicU64::new(0));
    thread::scope(|scope| {
        for lane in 0..4 {
            let total = total.clone();
            scope.spawn(move || {
                let mut subtotal = 0;
                let mut i = lane + 1;
                while i <= n {
                    subtotal += i;
                    i += 4;
                }
                total.fetch_add(subtotal, Ordering::AcqRel);
            });
        }
    });
    total.load(Ordering::Acquire)
}

// Deliberately small, real PostgreSQL wire client for the fixture's trusted,
// loopback, trust-authenticated database. The challenge separately tests SCRAM.
struct Pg(TcpStream);
impl Pg {
    fn connect() -> Self {
        let address: SocketAddr = format!(
            "{}:{}",
            std::env::var("FLUXR_TEST_PG_HOST").unwrap(),
            std::env::var("FLUXR_TEST_PG_PORT").unwrap()
        ).parse().unwrap();
        assert!(address.ip().is_loopback());
        let stream = TcpStream::connect_timeout(&address, TIMEOUT).unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(TIMEOUT)).unwrap();
        let mut client = Self(stream);
        let mut body = 196608u32.to_be_bytes().to_vec();
        for (key, value) in [
            ("user", std::env::var("FLUXR_TEST_PG_USER").unwrap()),
            ("database", std::env::var("FLUXR_TEST_PG_DATABASE").unwrap()),
            ("application_name", "fluxr-v16-host-fixture".to_string()),
        ] {
            body.extend_from_slice(key.as_bytes()); body.push(0);
            assert!(!value.as_bytes().contains(&0));
            body.extend_from_slice(value.as_bytes()); body.push(0);
        }
        body.push(0);
        client.0.write_all(&((body.len() + 4) as u32).to_be_bytes()).unwrap();
        client.0.write_all(&body).unwrap();
        let mut authenticated = false;
        loop {
            let (tag, payload) = client.receive();
            match tag {
                b'R' => { assert_eq!(payload, [0, 0, 0, 0]); authenticated = true; }
                b'Z' => { assert!(authenticated); assert_eq!(payload, b"I"); break; }
                b'S' | b'K' | b'N' => (),
                _ => panic!("unexpected PostgreSQL startup message {tag}"),
            }
        }
        client
    }
    fn receive(&mut self) -> (u8, Vec<u8>) {
        let mut header = [0; 5];
        self.0.read_exact(&mut header).unwrap();
        let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
        assert!((4..=1_048_576).contains(&length));
        let mut payload = vec![0; length - 4];
        self.0.read_exact(&mut payload).unwrap();
        (header[0], payload)
    }
    fn query(&mut self, sql: &str) -> Vec<Vec<Option<Vec<u8>>>> {
        assert!(!sql.as_bytes().contains(&0));
        self.0.write_all(b"Q").unwrap();
        self.0.write_all(&((sql.len() + 5) as u32).to_be_bytes()).unwrap();
        self.0.write_all(sql.as_bytes()).unwrap();
        self.0.write_all(&[0]).unwrap();
        let mut rows = Vec::new();
        let mut bytes = 0;
        loop {
            let (tag, payload) = self.receive();
            bytes += payload.len(); assert!(bytes <= 4_194_304);
            match tag {
                b'D' => {
                    assert!(payload.len() >= 2);
                    let count = u16::from_be_bytes(payload[..2].try_into().unwrap()) as usize;
                    let mut offset = 2; let mut row = Vec::new();
                    for _ in 0..count {
                        assert!(offset + 4 <= payload.len());
                        let length = i32::from_be_bytes(payload[offset..offset + 4].try_into().unwrap()); offset += 4;
                        if length == -1 { row.push(None); }
                        else { assert!(length >= 0 && offset + length as usize <= payload.len()); row.push(Some(payload[offset..offset + length as usize].to_vec())); offset += length as usize; }
                    }
                    assert_eq!(offset, payload.len()); rows.push(row);
                }
                b'Z' => { assert_eq!(payload.len(), 1); return rows; }
                b'E' => panic!("PostgreSQL query error: {}", String::from_utf8_lossy(&payload)),
                b'T' | b'C' | b'N' | b'S' | b'I' => (),
                _ => panic!("unexpected PostgreSQL query message {tag}"),
            }
        }
    }
}
impl Drop for Pg {
    fn drop(&mut self) { let _ = self.0.write_all(&[b'X', 0, 0, 0, 4]); }
}

fn pg_add(delta: u64) -> u64 {
    assert!(delta <= 1_000_000, "fixture work bound");
    let mut pg = Pg::connect();
    pg.query("BEGIN");
    let rows = pg.query(&format!("UPDATE unified_probe SET n=n+{delta} WHERE id=1 RETURNING n"));
    assert_eq!(rows.len(), 1);
    let value = std::str::from_utf8(rows[0][0].as_ref().unwrap()).unwrap().parse().unwrap();
    pg.query("COMMIT");
    value
}

fn pg_rollback(delta: u64) -> u64 {
    assert!(delta <= 1_000_000, "fixture work bound");
    let mut pg = Pg::connect();
    pg.query("BEGIN");
    pg.query(&format!("UPDATE unified_probe SET n=n+{delta} WHERE id=1"));
    pg.query("ROLLBACK");
    let rows = pg.query("SELECT n FROM unified_probe WHERE id=1");
    assert_eq!(rows.len(), 1);
    std::str::from_utf8(rows[0][0].as_ref().unwrap()).unwrap().parse().unwrap()
}

#[cfg(unix)]
fn unix_listener_probe() -> i64 {
    use std::os::unix::net::{UnixListener, UnixStream};
    let path = std::env::var("FLUXR_TEST_UNIX_PATH").unwrap();
    assert!(path.len() < 100);
    match UnixListener::bind(&path) {
        Err(error) => error.raw_os_error().unwrap_or(-1) as i64,
        Ok(listener) => {
            let mut client = UnixStream::connect(&path).unwrap();
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(TIMEOUT)).unwrap();
            client.write_all(b"unified").unwrap();
            let mut payload = [0; 7]; peer.read_exact(&mut payload).unwrap();
            assert_eq!(&payload, b"unified");
            drop(client); drop(peer); drop(listener);
            std::fs::remove_file(&path).unwrap();
            0
        }
    }
}

fn register_services() {
    let runtime = global();
    runtime.register_host("atomic_add", vec![Type::U64], Type::U64, |args| {
        Ok(Value::U64(COUNTER.fetch_add(uint(args, 0), Ordering::AcqRel) + uint(args, 0)))
    }).unwrap();
    runtime.register_host("atomic_read", vec![], Type::U64, |_| {
        Ok(Value::U64(COUNTER.load(Ordering::Acquire)))
    }).unwrap();
    runtime.register_host("thread_sum", vec![Type::U64], Type::U64, |args| {
        Ok(Value::U64(threaded_sum(uint(args, 0))))
    }).unwrap();
    runtime.register_host("tcp_roundtrip", vec![Type::U64], Type::U64, |args| {
        Ok(Value::U64(tcp_roundtrip(uint(args, 0))))
    }).unwrap();
    runtime.register_host("pg_add", vec![Type::U64], Type::U64, |args| {
        Ok(Value::U64(pg_add(uint(args, 0))))
    }).unwrap();
    runtime.register_host("pg_rollback", vec![Type::U64], Type::U64, |args| {
        Ok(Value::U64(pg_rollback(uint(args, 0))))
    }).unwrap();
    runtime.register_host("reader_stats", vec![], Type::List(Box::new(Type::U64)), |_| {
        Ok(Value::List(Arc::new(vec![Value::U64(CALLS.load(Ordering::Acquire)), Value::U64(BAD_CALLS.load(Ordering::Acquire))])))
    }).unwrap();
    runtime.register_host("pin_hot", vec![], Type::U64, |_| {
        let snapshot = global().snapshot("hot")?;
        let revision = snapshot.call("f", &[], &[], Limits::default())?.revision;
        *PINNED.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(snapshot);
        Ok(Value::U64(revision))
    }).unwrap();
    runtime.register_host("call_pinned", vec![], Type::I64, |_| {
        let guard = PINNED.get_or_init(|| Mutex::new(None)).lock().unwrap();
        Ok(guard.as_ref().unwrap().call("f", &[], &[], Limits::default())?.value)
    }).unwrap();
    runtime.register_host("release_pin", vec![], Type::Unit, |_| {
        PINNED.get_or_init(|| Mutex::new(None)).lock().unwrap().take();
        Ok(Value::Unit)
    }).unwrap();
    runtime.register_host("reentrant_install", vec![Type::U64], Type::U64, |args| {
        let runtime = global();
        let expected = match runtime.snapshot("aux") {
            Ok(snapshot) => Some(snapshot.call("id", &[], &[], Limits::default())?.revision),
            Err(_) => None,
        };
        runtime.install("aux", expected, &format!("fluxr 1\npub id 0 uint ret {}u\n", uint(args, 0)))?;
        Ok(runtime.call("aux", "id", &[], &[], Limits::default())?.value)
    }).unwrap();
    runtime.register_host("host_error", vec![], Type::Unit, |_| -> Result<Value, RuntimeError> {
        Err(global().install("never_valid", None, "invalid source").unwrap_err())
    }).unwrap();
    runtime.register_host("host_panic", vec![], Type::Unit, |_| -> Result<Value, RuntimeError> {
        panic!("independent trusted host callback panic probe")
    }).unwrap();
    runtime.register_host("wrong_result", vec![], Type::U64, |_| {
        Ok(Value::Bool(true))
    }).unwrap();
    #[cfg(unix)]
    runtime.register_host("unix_listener", vec![], Type::I64, |_| {
        Ok(Value::I64(unix_listener_probe()))
    }).unwrap();
}

fn main() {
    // This is reached after the compiler-inserted Runtime startup.
    register_services();
    global().install("hot", None, "fluxr 1\npub f 0 int ret 20\n").unwrap();
    let stopping = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let stopping = stopping.clone();
        workers.push(thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match global().call("hot", "f", &[], &[], Limits::default()) {
                    Ok(result) if result.value == Value::I64((result.revision * 17 + 3) as i64) => (),
                    _ => { BAD_CALLS.fetch_add(1, Ordering::AcqRel); }
                }
                CALLS.fetch_add(1, Ordering::AcqRel);
                thread::sleep(Duration::from_micros(200));
            }
        }));
    }
    println!("UNIFIED_HOST_READY {}", std::process::id());
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    stopping.store(true, Ordering::Release);
    for worker in workers { worker.join().unwrap(); }
    println!("UNIFIED_HOST_STOP {} {}", CALLS.load(Ordering::Acquire), BAD_CALLS.load(Ordering::Acquire));
    assert_eq!(BAD_CALLS.load(Ordering::Acquire), 0);
}
