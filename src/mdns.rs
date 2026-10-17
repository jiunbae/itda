//! Finds Macs with Screen Sharing on the local network: a one-shot mDNS (Bonjour) browse
//! for `_rfb._tcp.local`, sent from every local IPv4 interface.

use serde::Serialize;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Found {
    /// Bonjour instance name ("Alex's Mac mini").
    pub name: String,
    /// What to connect to: the .local name when Windows can resolve it, else the IP.
    pub host: String,
    pub hostname: String,
    pub ip: String,
    pub port: u16,
    /// From the TXT record: "windows" for a PC sharing with Itda; empty for a Mac.
    pub os: String,
}

const SERVICE: &str = "_rfb._tcp.local";

fn query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in SERVICE.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 12, 0x80, 1]); // PTR, IN with the unicast-response bit
    q
}

fn local_ipv4() -> Vec<Ipv4Addr> {
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into());
    let mut v: Vec<Ipv4Addr> = (host.as_str(), 0)
        .to_socket_addrs()
        .map(|it| it.filter_map(|a| if let SocketAddr::V4(a) = a { Some(*a.ip()) } else { None }).collect())
        .unwrap_or_default();
    v.retain(|ip| !ip.is_loopback() && !ip.is_link_local());
    if v.is_empty() {
        v.push(Ipv4Addr::UNSPECIFIED);
    }
    v
}

fn set_multicast_if(s: &UdpSocket, ip: Ipv4Addr) {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{setsockopt, IPPROTO_IP, IP_MULTICAST_IF};
    let addr = u32::from_ne_bytes(ip.octets());
    unsafe { setsockopt(s.as_raw_socket() as usize, IPPROTO_IP, IP_MULTICAST_IF, &addr as *const u32 as *const u8, 4) };
}

/// Browse for about `wait`, returning every Mac that answered.
pub fn browse(wait: Duration) -> Vec<Found> {
    let q = query();
    let socks: Vec<UdpSocket> = local_ipv4()
        .into_iter()
        .filter_map(|ip| {
            let s = UdpSocket::bind((ip, 0)).ok()?;
            set_multicast_if(&s, ip);
            s.send_to(&q, "224.0.0.251:5353").ok()?;
            s.set_read_timeout(Some(Duration::from_millis(100))).ok()?;
            Some(s)
        })
        .collect();
    let mut recs = Records::default();
    let end = Instant::now() + wait;
    let mut buf = [0u8; 9000];
    while Instant::now() < end {
        for s in &socks {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                recs.parse(&buf[..n], from);
            }
        }
    }
    recs.found()
}

#[derive(Default)]
struct Records {
    instances: Vec<String>,
    srv: HashMap<String, (String, u16)>,
    a: HashMap<String, Ipv4Addr>,
    /// Who answered for an instance, in case there is no A record.
    from: HashMap<String, Ipv4Addr>,
    txt: HashMap<String, HashMap<String, String>>,
}

impl Records {
    fn parse(&mut self, p: &[u8], from: SocketAddr) {
        let Some(rrs) = parse_rrs(p) else { return };
        let src = if let SocketAddr::V4(a) = from { Some(*a.ip()) } else { None };
        for rr in rrs {
            match rr.kind {
                12 if rr.name.eq_ignore_ascii_case(SERVICE) => {
                    if let Some((inst, _)) = read_name(p, rr.rdata) {
                        if let Some(ip) = src {
                            self.from.insert(inst.clone(), ip);
                        }
                        if !self.instances.contains(&inst) {
                            self.instances.push(inst);
                        }
                    }
                }
                33 if rr.rdlen >= 6 => {
                    let port = u16::from_be_bytes([p[rr.rdata + 4], p[rr.rdata + 5]]);
                    if let Some((target, _)) = read_name(p, rr.rdata + 6) {
                        self.srv.insert(rr.name.clone(), (target, port));
                    }
                }
                16 => {
                    let mut kv = HashMap::new();
                    let mut i = rr.rdata;
                    while i < rr.rdata + rr.rdlen {
                        let n = p[i] as usize;
                        let s = String::from_utf8_lossy(&p[i + 1..(i + 1 + n).min(rr.rdata + rr.rdlen)]).into_owned();
                        if let Some((k, v)) = s.split_once('=') {
                            kv.insert(k.to_ascii_lowercase(), v.to_string());
                        }
                        i += 1 + n;
                    }
                    self.txt.insert(rr.name.clone(), kv);
                }
                1 if rr.rdlen == 4 => {
                    let d = &p[rr.rdata..rr.rdata + 4];
                    self.a.insert(rr.name.to_ascii_lowercase(), Ipv4Addr::new(d[0], d[1], d[2], d[3]));
                }
                _ => {}
            }
        }
    }

    fn found(&self) -> Vec<Found> {
        let mut out: Vec<Found> = Vec::new();
        for inst in &self.instances {
            let name = inst.strip_suffix(&format!(".{SERVICE}")).unwrap_or(inst).to_string();
            let (hostname, port) = self.srv.get(inst).cloned().unwrap_or_default();
            let ip = self.a.get(&hostname.to_ascii_lowercase()).or(self.from.get(inst)).map(|i| i.to_string()).unwrap_or_default();
            let port = if port == 0 { 5900 } else { port };
            let resolvable = !hostname.is_empty() && (hostname.as_str(), port).to_socket_addrs().is_ok_and(|mut a| a.next().is_some());
            let mut host = if resolvable { hostname.clone() } else { ip.clone() };
            if host.is_empty() {
                continue;
            }
            if port != 5900 {
                host = format!("{host}:{port}");
            }
            if !out.iter().any(|f| f.host == host) {
                let os = self.txt.get(inst).and_then(|t| t.get("os")).cloned().unwrap_or_default();
                out.push(Found { name, host, hostname, ip, port, os });
            }
        }
        out
    }
}

struct Rr {
    name: String,
    kind: u16,
    rdata: usize,
    rdlen: usize,
}

fn parse_rrs(p: &[u8]) -> Option<Vec<Rr>> {
    if p.len() < 12 || p[2] & 0x80 == 0 {
        return None; // not a response
    }
    let cnt = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]) as usize;
    let (qd, total) = (cnt(4), cnt(6) + cnt(8) + cnt(10));
    let mut off = 12;
    for _ in 0..qd {
        off = read_name(p, off)?.1 + 4;
    }
    let mut out = Vec::new();
    for _ in 0..total {
        let (name, o) = read_name(p, off)?;
        if o + 10 > p.len() {
            return None;
        }
        let kind = u16::from_be_bytes([p[o], p[o + 1]]);
        let rdlen = u16::from_be_bytes([p[o + 8], p[o + 9]]) as usize;
        let rdata = o + 10;
        if rdata + rdlen > p.len() {
            return None;
        }
        out.push(Rr { name, kind, rdata, rdlen });
        off = rdata + rdlen;
    }
    Some(out)
}

/// Decode a (possibly compressed) name at `off`: (dotted name, offset after it).
fn read_name(p: &[u8], mut off: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *p.get(off)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(off + 1)));
        }
        if len & 0xc0 == 0xc0 {
            let ptr = ((len & 0x3f) << 8) | *p.get(off + 1)? as usize;
            end.get_or_insert(off + 2);
            off = ptr;
            continue;
        }
        let l = p.get(off + 1..off + 1 + len)?;
        labels.push(String::from_utf8_lossy(l).into_owned());
        off += 1 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(out: &mut Vec<u8>, n: &str) {
        for l in n.split('.') {
            out.push(l.len() as u8);
            out.extend_from_slice(l.as_bytes());
        }
        out.push(0);
    }

    #[test]
    fn parses_mac_response() {
        // PTR + SRV + A, the way macOS answers (SRV target uses compression back to "local").
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 3, 0, 0, 0, 0];
        let svc_off = p.len();
        name(&mut p, SERVICE);
        p.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 120]);
        let mut rd = Vec::new();
        rd.push(15);
        rd.extend_from_slice("Alex's Mac mini".as_bytes());
        rd.extend_from_slice(&[0xc0, svc_off as u8]);
        p.extend_from_slice(&(rd.len() as u16).to_be_bytes());
        let inst_off = p.len();
        p.extend_from_slice(&rd);
        // SRV: name = pointer to instance
        p.extend_from_slice(&[0xc0, inst_off as u8, 0, 33, 0x80, 1, 0, 0, 0, 120]);
        let mut srv = vec![0, 0, 0, 0, 0x17, 0x0c]; // port 5900
        name(&mut srv, "mac-mini.local");
        p.extend_from_slice(&(srv.len() as u16).to_be_bytes());
        p.extend_from_slice(&srv);
        // A
        name(&mut p, "mac-mini.local");
        p.extend_from_slice(&[0, 1, 0x80, 1, 0, 0, 0, 120, 0, 4, 192, 168, 0, 55]);

        let mut r = Records::default();
        r.parse(&p, "192.168.0.55:5353".parse().unwrap());
        assert_eq!(r.instances, vec!["Alex's Mac mini._rfb._tcp.local".to_string()]);
        assert_eq!(r.srv.values().next(), Some(&("mac-mini.local".to_string(), 5900)));
        assert_eq!(r.a.get("mac-mini.local"), Some(&Ipv4Addr::new(192, 168, 0, 55)));
        let f = r.found();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "Alex's Mac mini");
        assert_eq!(f[0].ip, "192.168.0.55");
    }
}
