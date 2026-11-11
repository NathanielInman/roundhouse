//! The public address through STUN (RFC 5389): one UDP binding request to a
//! STUN server, which answers with the address and port it saw the request
//! come from. It replaces a plain-HTTP "what is my IP" page: the reply is
//! matched to the request by a random transaction ID, IPv6 works the same as
//! IPv4, and asking two servers from the same socket shows how the NAT maps
//! ports.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

const MAGIC: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const MAPPED_ADDRESS: u16 = 0x0001;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// A binding request with this transaction ID.
pub fn request(tid: &[u8; 12]) -> [u8; 20] {
    let mut b = [0u8; 20];
    b[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    b[4..8].copy_from_slice(&MAGIC.to_be_bytes());
    b[8..20].copy_from_slice(tid);
    b
}

/// The mapped address from a binding success reply to `tid`, or `None` for
/// anything else (another transaction, an error reply, a short packet).
pub fn parse(reply: &[u8], tid: &[u8; 12]) -> Option<SocketAddr> {
    if reply.len() < 20
        || u16::from_be_bytes([reply[0], reply[1]]) != BINDING_SUCCESS
        || u32::from_be_bytes(reply[4..8].try_into().ok()?) != MAGIC
        || &reply[8..20] != tid
    {
        return None;
    }
    let len = u16::from_be_bytes([reply[2], reply[3]]) as usize;
    let attrs = reply.get(20..20 + len)?;
    let mut plain = None;
    let mut at = 0;
    while at + 4 <= attrs.len() {
        let ty = u16::from_be_bytes([attrs[at], attrs[at + 1]]);
        let alen = u16::from_be_bytes([attrs[at + 2], attrs[at + 3]]) as usize;
        let v = attrs.get(at + 4..at + 4 + alen)?;
        match ty {
            XOR_MAPPED_ADDRESS => return decode(v, Some(tid)),
            MAPPED_ADDRESS => plain = decode(v, None),
            _ => {}
        }
        // Attributes are padded to four bytes.
        at += 4 + alen.div_ceil(4) * 4;
    }
    plain
}

/// An address attribute; with `tid`, XOR-ed as XOR-MAPPED-ADDRESS is.
fn decode(v: &[u8], tid: Option<&[u8; 12]>) -> Option<SocketAddr> {
    let family = *v.get(1)?;
    let mut port = u16::from_be_bytes([*v.get(2)?, *v.get(3)?]);
    let mut key = [0u8; 16];
    key[..4].copy_from_slice(&MAGIC.to_be_bytes());
    if let Some(t) = tid {
        key[4..].copy_from_slice(t);
        port ^= (MAGIC >> 16) as u16;
    }
    let x = |i: usize, b: u8| if tid.is_some() { b ^ key[i] } else { b };
    let ip = match family {
        1 => {
            let a: [u8; 4] = v.get(4..8)?.try_into().ok()?;
            IpAddr::V4(Ipv4Addr::from(std::array::from_fn::<u8, 4, _>(|i| {
                x(i, a[i])
            })))
        }
        2 => {
            let a: [u8; 16] = v.get(4..20)?.try_into().ok()?;
            IpAddr::V6(Ipv6Addr::from(std::array::from_fn::<u8, 16, _>(|i| {
                x(i, a[i])
            })))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

fn transaction_id() -> [u8; 12] {
    let mut tid = [0u8; 12];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(&mut tid).is_ok() {
            return tid;
        }
    }
    // No urandom: the clock is unpredictable enough to match a reply.
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    tid.copy_from_slice(&n.to_le_bytes()[..12]);
    tid
}

/// Asks `server` from `sock` and waits up to 2 s for the answer.
fn ask(sock: &UdpSocket, server: SocketAddr) -> Option<SocketAddr> {
    let tid = transaction_id();
    sock.send_to(&request(&tid), server).ok()?;
    let mut buf = [0u8; 512];
    for _ in 0..4 {
        let (n, from) = sock.recv_from(&mut buf).ok()?;
        if from == server {
            if let Some(a) = parse(&buf[..n], &tid) {
                return Some(a);
            }
        }
    }
    None
}

/// What the servers saw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    pub v4: Option<IpAddr>,
    pub v6: Option<IpAddr>,
    /// Whether two servers saw the same port: the NAT keeps one mapping per
    /// socket (good for calls and games) rather than one per destination.
    pub same_port: Option<bool>,
}

/// Asks each server over IPv4, and the first over IPv6.
pub fn discover(servers: &[String]) -> Seen {
    let mut seen = Seen::default();
    let resolve = |s: &String, v6: bool| s.to_socket_addrs().ok()?.find(|a| a.is_ipv6() == v6);
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
        let answers: Vec<SocketAddr> = servers
            .iter()
            .filter_map(|s| resolve(s, false))
            .filter_map(|srv| ask(&sock, srv))
            .collect();
        seen.v4 = answers.first().map(SocketAddr::ip);
        if answers.len() >= 2 {
            seen.same_port = Some(answers.iter().all(|a| a.port() == answers[0].port()));
        }
    }
    if let (Ok(sock), Some(srv)) = (
        UdpSocket::bind("[::]:0"),
        servers.first().and_then(|s| resolve(s, true)),
    ) {
        let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
        seen.v6 = ask(&sock, srv).map(|a| a.ip());
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    const TID: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

    // Laid out as stun.cloudflare.com and stun.l.google.com answer (one
    // XOR-MAPPED-ADDRESS), with documentation addresses.
    #[test]
    fn reads_xor_mapped_addresses() {
        let v4 = hex("0101000c2112a4420102030405060708090a0b0c002000080001e242ea12d568");
        assert_eq!(
            parse(&v4, &TID),
            Some("203.0.113.42:50000".parse().unwrap())
        );
        let v6 = hex("010100182112a4420102030405060708090a0b0c002000140002e2420113a9fa0102030405060708090a0b4e");
        assert_eq!(
            parse(&v6, &TID),
            Some("[2001:db8::42]:50000".parse().unwrap())
        );
    }

    #[test]
    fn ignores_other_transactions_and_errors() {
        let v4 = hex("0101000c2112a4420102030405060708090a0b0c002000080001e242ea12d568");
        let mut other = TID;
        other[0] = 9;
        assert_eq!(parse(&v4, &other), None);
        let mut err = v4.clone();
        err[1] = 0x11;
        assert_eq!(parse(&err, &TID), None);
        assert_eq!(parse(&v4[..30], &TID), None);
    }

    #[test]
    fn builds_a_request() {
        let r = request(&TID);
        assert_eq!(&r[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
        assert_eq!(&r[8..], &TID);
    }
}
