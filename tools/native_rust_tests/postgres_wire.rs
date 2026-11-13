//! Live PostgreSQL 18 qualification fixture, not a production database driver.
//! No fallback/mock: missing explicit configuration is a test failure.
//! Endpoint must be a loopback IP. Login supports trust authentication only.
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Row = Vec<Option<Vec<u8>>>;
const MAX_MESSAGE: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct Config { endpoint: SocketAddr, user: String, database: String }
impl Config {
    fn from_env() -> Self {
        let required = |key| std::env::var(key).unwrap_or_else(|_| panic!("{key} is required for real PostgreSQL tests"));
        let host: IpAddr = required("FLUXR_TEST_PG_HOST").parse().expect("PG host must be an IP");
        assert!(host.is_loopback(), "native fixtures require loopback PostgreSQL");
        let port: u16 = required("FLUXR_TEST_PG_PORT").parse().unwrap();
        assert_ne!(port, 0);
        Self { endpoint: SocketAddr::new(host, port), user: required("FLUXR_TEST_PG_USER"), database: required("FLUXR_TEST_PG_DATABASE") }
    }
}

#[derive(Debug)]
struct Reply { rows: Vec<Row>, commands: Vec<String>, sqlstates: Vec<String>, transaction: u8 }
impl Reply {
    fn ok(self) -> Self { assert!(self.sqlstates.is_empty(), "PostgreSQL error(s): {:?}", self.sqlstates); self }
    fn scalar(&self) -> &str { std::str::from_utf8(self.rows[0][0].as_ref().unwrap()).unwrap() }
}

struct Pg { stream: TcpStream }
fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }
fn cstring(bytes: &mut Vec<u8>, value: &str) { assert!(!value.as_bytes().contains(&0)); bytes.extend_from_slice(value.as_bytes()); bytes.push(0); }
impl Pg {
    fn connect(config: &Config) -> io::Result<Self> {
        let stream = TcpStream::connect_timeout(&config.endpoint, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        stream.set_nodelay(true)?;
        let mut pg = Self { stream };
        let mut body = 196608u32.to_be_bytes().to_vec();
        for (key, value) in [("user", config.user.as_str()), ("database", config.database.as_str()), ("application_name", "fluxr-native-v15-fixture"), ("client_encoding", "UTF8")] {
            cstring(&mut body, key); cstring(&mut body, value);
        }
        body.push(0);
        pg.stream.write_all(&((body.len() + 4) as u32).to_be_bytes())?;
        pg.stream.write_all(&body)?;
        let mut authenticated = false;
        loop {
            let (tag, data) = pg.receive()?;
            match tag {
                b'R' if data == 0u32.to_be_bytes() => authenticated = true,
                b'R' => return Err(invalid("fixture requires a loopback trust-authenticated PostgreSQL instance")),
                b'E' => return Err(invalid(&format!("PostgreSQL startup error: {}", String::from_utf8_lossy(&data)))),
                b'Z' => { if !authenticated || data != b"I" { return Err(invalid("invalid startup completion")); } break; }
                b'S' | b'K' | b'N' => {},
                _ => return Err(invalid("unexpected PostgreSQL startup message")),
            }
        }
        let version = pg.query("SHOW server_version_num")?.ok().scalar().parse::<u32>().unwrap();
        if !(180000..190000).contains(&version) { return Err(invalid(&format!("PostgreSQL18 required, observed {version}"))); }
        Ok(pg)
    }
    fn send(&mut self, tag: u8, body: &[u8]) -> io::Result<()> {
        if body.len() > MAX_MESSAGE - 4 { return Err(invalid("outbound message too large")); }
        self.stream.write_all(&[tag])?;
        self.stream.write_all(&((body.len() + 4) as u32).to_be_bytes())?;
        self.stream.write_all(body)
    }
    fn receive(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let mut header = [0u8; 5]; self.stream.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header[1..5].try_into().unwrap()) as usize;
        if !(4..=MAX_MESSAGE).contains(&length) { return Err(invalid("invalid PostgreSQL message length")); }
        let mut data = vec![0; length - 4]; self.stream.read_exact(&mut data)?;
        Ok((header[0], data))
    }
    fn query(&mut self, sql: &str) -> io::Result<Reply> {
        let mut body = Vec::new(); cstring(&mut body, sql); self.send(b'Q', &body)?; self.drain()
    }
    fn parameterized(&mut self, sql: &str, params: &[Option<&[u8]>]) -> io::Result<Reply> {
        let mut parse = Vec::new(); cstring(&mut parse, ""); cstring(&mut parse, sql);
        parse.extend_from_slice(&0u16.to_be_bytes()); self.send(b'P', &parse)?;
        let mut bind = vec![0, 0]; // unnamed portal and prepared statement
        bind.extend_from_slice(&0u16.to_be_bytes()); // text parameter format
        bind.extend_from_slice(&u16::try_from(params.len()).unwrap().to_be_bytes());
        for value in params {
            match value {
                None => bind.extend_from_slice(&(-1i32).to_be_bytes()),
                Some(bytes) => { bind.extend_from_slice(&i32::try_from(bytes.len()).unwrap().to_be_bytes()); bind.extend_from_slice(bytes); }
            }
        }
        bind.extend_from_slice(&0u16.to_be_bytes()); // text result format
        self.send(b'B', &bind)?;
        self.send(b'E', &[0, 0, 0, 0, 0])?; // unnamed portal, unlimited rows
        self.send(b'S', &[])?;
        self.drain()
    }
    fn drain(&mut self) -> io::Result<Reply> {
        let mut reply = Reply { rows: Vec::new(), commands: Vec::new(), sqlstates: Vec::new(), transaction: 0 };
        let mut received = 0usize;
        loop {
            let (tag, data) = self.receive()?;
            received = received.checked_add(data.len()).ok_or_else(|| invalid("result budget overflow"))?;
            if received > 4 * MAX_MESSAGE { return Err(invalid("result budget exceeded")); }
            match tag {
                b'D' => reply.rows.push(parse_row(&data)?),
                b'C' => { if data.last() != Some(&0) { return Err(invalid("unterminated command tag")); } reply.commands.push(String::from_utf8_lossy(&data[..data.len()-1]).into_owned()); }
                b'E' => {
                    let mut cursor = 0;
                    let mut state = None;
                    while cursor < data.len() && data[cursor] != 0 {
                        let kind = data[cursor]; cursor += 1;
                        let end = data[cursor..].iter().position(|b| *b == 0).ok_or_else(|| invalid("unterminated error field"))? + cursor;
                        if kind == b'C' { state = Some(String::from_utf8_lossy(&data[cursor..end]).into_owned()); }
                        cursor = end + 1;
                    }
                    reply.sqlstates.push(state.ok_or_else(|| invalid("error lacks SQLSTATE"))?);
                }
                b'Z' => { if data.len() != 1 || !b"ITE".contains(&data[0]) { return Err(invalid("invalid transaction status")); } reply.transaction = data[0]; return Ok(reply); }
                b'T' | b'1' | b'2' | b'3' | b'n' | b'I' | b'N' | b'S' => {},
                _ => return Err(invalid("unexpected PostgreSQL query message")),
            }
        }
    }
}
impl Drop for Pg { fn drop(&mut self) { let _ = self.send(b'X', &[]); } }

// FLUXR_NATIVE_REPLACEABLE_FUNCTION_BEGIN pg_roundtrip
fn native_pg_roundtrip(pg: &mut Pg, sql: &str) -> io::Result<Reply> { pg.query(sql) }
// FLUXR_NATIVE_REPLACEABLE_FUNCTION_END pg_roundtrip


fn parse_row(data: &[u8]) -> io::Result<Row> {
    if data.len() < 2 { return Err(invalid("short DataRow")); }
    let count = u16::from_be_bytes(data[..2].try_into().unwrap()) as usize;
    let mut at = 2;
    let mut row = Vec::with_capacity(count);
    for _ in 0..count {
        if at + 4 > data.len() { return Err(invalid("short DataRow field length")); }
        let length = i32::from_be_bytes(data[at..at+4].try_into().unwrap()); at += 4;
        if length == -1 { row.push(None); continue; }
        if length < 0 || length as usize > data.len() - at { return Err(invalid("invalid DataRow field length")); }
        row.push(Some(data[at..at+length as usize].to_vec())); at += length as usize;
    }
    if at != data.len() { return Err(invalid("DataRow trailing bytes")); }
    Ok(row)
}

struct Table { config: Config, name: String }
impl Table {
    fn create(config: Config, label: &str) -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let name = format!("fluxr_{}_{}_{}", label, std::process::id(), stamp);
        let mut pg = Pg::connect(&config).unwrap();
        pg.query(&format!("CREATE TABLE {name} (id integer PRIMARY KEY, n integer NOT NULL DEFAULT 0, body text)")).unwrap().ok();
        Self { config, name }
    }
}
impl Drop for Table {
    fn drop(&mut self) {
        if let Ok(mut pg) = Pg::connect(&self.config) { let _ = pg.query(&format!("DROP TABLE IF EXISTS {}", self.name)); }
    }
}

#[test]
fn live_pg18_rows_null_utf8_and_extended_parameters() {
    let config = Config::from_env();
    let mut pg = Pg::connect(&config).unwrap();
    let reply = native_pg_roundtrip(&mut pg, "SELECT 42::integer, NULL::text, 'héllø 🌍'::text, ''::text").unwrap().ok();
    assert_eq!(reply.rows, vec![vec![Some(b"42".to_vec()), None, Some("héllø 🌍".as_bytes().to_vec()), Some(vec![])]]);
    assert_eq!(reply.transaction, b'I');
    let malicious = b"x'); DROP TABLE never_do_this; --";
    let reply = pg.parameterized("SELECT $1::text, $2::integer + 1, $3::text", &[Some(malicious), Some(b"41"), None]).unwrap().ok();
    assert_eq!(reply.rows, vec![vec![Some(malicious.to_vec()), Some(b"42".to_vec()), None]]);
}

#[test]
fn live_pg18_commit_persists_reconnect_and_rollback_does_not() {
    let config = Config::from_env();
    let table = Table::create(config.clone(), "commit");
    let mut pg = Pg::connect(&config).unwrap();
    let durability = pg.query("SHOW synchronous_commit").unwrap().ok();
    assert_ne!(durability.scalar(), "off", "durable commit qualification requires synchronous_commit");
    assert_eq!(pg.query("BEGIN").unwrap().ok().transaction, b'T');
    pg.query(&format!("INSERT INTO {} (id,n) VALUES (1,7)", table.name)).unwrap().ok();
    let commit = pg.query("COMMIT").unwrap().ok();
    assert_eq!(commit.transaction, b'I');
    assert_eq!(commit.commands, vec!["COMMIT"]);
    drop(pg);
    let mut reopened = Pg::connect(&config).unwrap();
    assert_eq!(reopened.query(&format!("SELECT n FROM {} WHERE id=1", table.name)).unwrap().ok().scalar(), "7");
    reopened.query("BEGIN").unwrap().ok();
    reopened.query(&format!("INSERT INTO {} (id,n) VALUES (2,99)", table.name)).unwrap().ok();
    reopened.query("ROLLBACK").unwrap().ok();
    assert_eq!(reopened.query(&format!("SELECT count(*) FROM {}", table.name)).unwrap().ok().scalar(), "1");
}

#[test]
fn live_pg18_constraint_error_and_failed_transaction_recovery() {
    let config = Config::from_env();
    let table = Table::create(config.clone(), "error");
    let mut pg = Pg::connect(&config).unwrap();
    pg.query(&format!("INSERT INTO {} (id) VALUES (1)", table.name)).unwrap().ok();
    pg.query("BEGIN").unwrap().ok();
    let failure = pg.query(&format!("INSERT INTO {} (id) VALUES (1)", table.name)).unwrap();
    assert_eq!(failure.sqlstates, vec!["23505"]);
    assert_eq!(failure.transaction, b'E');
    let aborted = pg.query("SELECT 1").unwrap();
    assert_eq!(aborted.sqlstates, vec!["25P02"]);
    pg.query("ROLLBACK").unwrap().ok();
    assert_eq!(pg.query("SELECT 42").unwrap().ok().scalar(), "42");
    let malformed = pg.parameterized("SELECT $1::integer", &[Some(b"not-an-integer")]).unwrap();
    assert_eq!(malformed.sqlstates, vec!["22P02"]);
    assert_eq!(pg.query("SELECT 7").unwrap().ok().scalar(), "7");
}

#[test]
fn live_pg18_parallel_transactions_preserve_all_updates() {
    let config = Config::from_env();
    let table = Table::create(config.clone(), "concurrent");
    let mut pg = Pg::connect(&config).unwrap();
    pg.query(&format!("INSERT INTO {} (id,n) VALUES (1,0)", table.name)).unwrap().ok();
    let barrier = Arc::new(Barrier::new(4));
    thread::scope(|scope| {
        for _ in 0..4 {
            let config = config.clone(); let name = &table.name; let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                let mut worker = Pg::connect(&config).unwrap(); barrier.wait();
                for _ in 0..20 {
                    worker.query("BEGIN").unwrap().ok();
                    worker.query(&format!("UPDATE {name} SET n=n+1 WHERE id=1")).unwrap().ok();
                    worker.query("COMMIT").unwrap().ok();
                }
            });
        }
    });
    assert_eq!(pg.query(&format!("SELECT n FROM {} WHERE id=1", table.name)).unwrap().ok().scalar(), "80");
}

#[test]
fn live_pg18_mvcc_visibility_and_lock_timeout_recovery() {
    let config = Config::from_env();
    let table = Table::create(config.clone(), "mvcc");
    let mut a = Pg::connect(&config).unwrap();
    let mut b = Pg::connect(&config).unwrap();
    a.query(&format!("INSERT INTO {} (id,n) VALUES (1,0)", table.name)).unwrap().ok();
    a.query("BEGIN").unwrap().ok();
    a.query(&format!("UPDATE {} SET n=9 WHERE id=1", table.name)).unwrap().ok();
    assert_eq!(b.query(&format!("SELECT n FROM {} WHERE id=1", table.name)).unwrap().ok().scalar(), "0");
    b.query("SET lock_timeout='50ms'").unwrap().ok();
    let locked = b.query(&format!("UPDATE {} SET n=8 WHERE id=1", table.name)).unwrap();
    assert_eq!(locked.sqlstates, vec!["55P03"]);
    a.query("COMMIT").unwrap().ok();
    assert_eq!(b.query(&format!("SELECT n FROM {} WHERE id=1", table.name)).unwrap().ok().scalar(), "9");
}
