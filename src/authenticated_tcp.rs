//! Shared-secret authentication for fixed-size, ordered exchanges over TCP.
//! This transport authenticates messages; it does not encrypt them.
use ring::{hmac, rand};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FRAME: usize = 16 * 1024;

pub enum Endpoint {
    Listen(TcpListener),
    Connect(SocketAddr),
}

impl Endpoint {
    pub fn slot(&self) -> usize {
        usize::from(matches!(self, Self::Connect(_)))
    }

    pub fn open(&self, secret: &[u8; 32], domain: &[u8]) -> io::Result<Link> {
        self.open_with(secret, domain, TIMEOUT, TIMEOUT)
    }

    /// Derive independent direction keys from the supplied application domain
    /// and fresh nonces. The first verified exchange authenticates the peer.
    pub fn open_with(
        &self,
        secret: &[u8; 32],
        domain: &[u8],
        open_timeout: Duration,
        io_timeout: Duration,
    ) -> io::Result<Link> {
        if domain.is_empty() || domain.len() > 256 || open_timeout.is_zero() || io_timeout.is_zero()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid link parameters",
            ));
        }
        let deadline = Instant::now()
            .checked_add(open_timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid link timeout"))?;
        if let Self::Listen(listener) = self {
            listener.set_nonblocking(true)?;
        }
        let mut stream = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "link open timed out",
                ));
            }
            let result = match self {
                Self::Listen(listener) => listener.accept().map(|(stream, _)| stream),
                Self::Connect(addr) => {
                    TcpStream::connect_timeout(addr, Duration::from_secs(1).min(remaining))
                }
            };
            match result {
                Ok(stream) => break stream,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::TimedOut
                    ) =>
                {
                    thread::sleep(
                        Duration::from_millis(100)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(error) => return Err(error),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(io_timeout))?;
        stream.set_write_timeout(Some(io_timeout))?;

        let mut nonces = [[0; 32]; 2];
        let slot = self.slot();
        rand::SecureRandom::fill(&rand::SystemRandom::new(), &mut nonces[slot])
            .map_err(|_| io::Error::other("secure randomness unavailable"))?;
        stream.write_all(&nonces[slot])?;
        stream.read_exact(&mut nonces[1 - slot])?;
        let secret = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let keys = [0, 1].map(|role| {
            let mut context = hmac::Context::with_key(&secret);
            context.update(domain);
            context.update(&nonces[0]);
            context.update(&nonces[1]);
            context.update(&[role]);
            hmac::Key::new(hmac::HMAC_SHA256, context.sign().as_ref())
        });
        Ok(Link {
            stream,
            keys,
            slot,
            sequence: Some(0),
        })
    }
}

pub struct Link {
    stream: TcpStream,
    keys: [hmac::Key; 2],
    slot: usize,
    sequence: Option<u64>,
}

impl Link {
    /// Both ends must exchange messages of the same size and in the same order.
    /// Failed I/O or authentication permanently closes this link.
    pub fn exchange<const N: usize>(&mut self, message: [u8; N]) -> io::Result<[u8; N]> {
        let mut response = [0; N];
        self.exchange_into(&message, &mut response)?;
        Ok(response)
    }

    pub fn exchange_into(&mut self, message: &[u8], response: &mut [u8]) -> io::Result<()> {
        if message.len() > MAX_FRAME || message.len() != response.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid exchange frame length",
            ));
        }
        let sequence = self
            .sequence
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "link is closed"))?;
        let result = self.exchange_at(sequence.to_be_bytes(), message, response);
        if result.is_ok() {
            self.sequence = sequence.checked_add(1);
        } else {
            response.fill(0);
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        result
    }

    fn exchange_at(
        &mut self,
        header: [u8; 8],
        message: &[u8],
        response: &mut [u8],
    ) -> io::Result<()> {
        let mut outgoing = hmac::Context::with_key(&self.keys[self.slot]);
        outgoing.update(&header);
        outgoing.update(message);
        self.stream.write_all(message)?;
        self.stream.write_all(outgoing.sign().as_ref())?;

        let mut tag = [0; 32];
        self.stream.read_exact(response)?;
        self.stream.read_exact(&mut tag)?;
        let mut signed = Vec::with_capacity(header.len() + response.len());
        signed.extend_from_slice(&header);
        signed.extend_from_slice(response);
        hmac::verify(&self.keys[1 - self.slot], &signed, &tag).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "peer authentication failed")
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatch_fails_closed_and_new_link_can_authenticate() -> io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let client = Endpoint::Connect(listener.local_addr()?);
        let server = Endpoint::Listen(listener);
        let open = |endpoint: &Endpoint, domain: &[u8]| {
            endpoint.open_with(
                &[8; 32],
                domain,
                Duration::from_secs(2),
                Duration::from_secs(2),
            )
        };
        thread::scope(|scope| -> io::Result<()> {
            let remote = scope.spawn(|| open(&client, b"other")?.exchange([9]));
            let mut link = open(&server, b"example")?;
            let mut response = [255];
            let local_error = link.exchange_into(&[7], &mut response).unwrap_err().kind();
            assert_eq!(response, [0]);
            let remote_error = remote.join().unwrap().unwrap_err().kind();
            assert!([local_error, remote_error].contains(&io::ErrorKind::InvalidData));
            assert_eq!(
                link.exchange([7]).unwrap_err().kind(),
                io::ErrorKind::NotConnected
            );
            Ok(())
        })?;
        thread::scope(|scope| -> io::Result<()> {
            let remote = scope.spawn(|| -> io::Result<()> {
                let mut link = open(&client, b"example")?;
                assert_eq!(link.exchange([9])?, [7]);
                assert_eq!(link.exchange([5])?, [3]);
                Ok(())
            });
            let mut link = open(&server, b"example")?;
            assert_eq!(link.exchange([7])?, [9]);
            assert_eq!(link.exchange([3])?, [5]);
            remote.join().unwrap()
        })
    }
}
