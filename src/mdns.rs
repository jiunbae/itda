//! Bonjour (mDNS / DNS-SD) on the local network, both ways.
//! - Browse: a one-shot query for `_rfb._tcp` (Screen Sharing on Macs, "Share this PC" in Itda,
//!   other VNC servers) and `_itda._tcp` (any running Itda), sent from every local IPv4
//!   interface. One entry per device, whatever it answered for.
//! - Register: this machine's own services, through the system's responder (Windows 10 1809+
//!   DnsServiceRegister; macOS mDNSResponder).

use serde::Serialize;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
#[cfg(windows)]
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Found {
    /// Bonjour instance name ("Alex's Mac mini").
    pub name: String,
    /// What to connect to: the .local name when it resolves, else the IP; ":port" when the
    /// screen is shared on another port than 5900.
    pub host: String,
    pub hostname: String,
    pub ip: String,
    pub port: u16,
    /// From the TXT records: "windows" | "mac"; empty for a Mac without Itda.
    pub os: String,
    /// Itda runs there.
    pub itda: bool,
    /// The screen is shared over VNC (Screen Sharing, "Share this PC").
    pub share: bool,
    /// Windows Remote Desktop is on (said by Itda on that PC).
    pub rdp: bool,
}

const RFB: &str = "_rfb._tcp.local";
const ITDA: &str = "_itda._tcp.local";
const SERVICES: [&str; 2] = [RFB, ITDA];

fn query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, SERVICES.len() as u8, 0, 0, 0, 0, 0, 0];
    for s in SERVICES {
        for label in s.split('.') {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.extend_from_slice(&[0, 0, 12, 0x80, 1]); // PTR, IN with the unicast-response bit
    }
    q
}

#[cfg(windows)]
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

/// macOS: every interface's IPv4 address, as ifconfig lists them. Asking on each one matters
/// when a VPN holds the default route: the OS would send the query only there.
#[cfg(not(windows))]
fn local_ipv4() -> Vec<Ipv4Addr> {
    let out = std::process::Command::new("/sbin/ifconfig").output().map(|o| o.stdout).unwrap_or_default();
    let mut v: Vec<Ipv4Addr> = String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("inet ")?.split_whitespace().next()?.parse().ok())
        .filter(|ip: &Ipv4Addr| lan(*ip))
        .collect();
    if v.is_empty() {
        v.push(Ipv4Addr::UNSPECIFIED);
    }
    v
}

/// Worth asking on: not loopback, link-local or Tailscale's 100.64/10 (no multicast there).
#[cfg(not(windows))]
fn lan(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !ip.is_loopback() && !ip.is_link_local() && !(o[0] == 100 && o[1] & 0xc0 == 64)
}

#[cfg(not(windows))]
fn set_multicast_if(s: &UdpSocket, ip: Ipv4Addr) {
    use std::os::fd::AsRawFd;
    extern "C" {
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const core::ffi::c_void, len: u32) -> i32;
    }
    const IPPROTO_IP: i32 = 0;
    const IP_MULTICAST_IF: i32 = 9;
    if ip.is_unspecified() {
        return;
    }
    let addr = u32::from_ne_bytes(ip.octets());
    unsafe { setsockopt(s.as_raw_fd(), IPPROTO_IP, IP_MULTICAST_IF, (&addr as *const u32).cast(), 4) };
}

#[cfg(windows)]
fn set_multicast_if(s: &UdpSocket, ip: Ipv4Addr) {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{setsockopt, IPPROTO_IP, IP_MULTICAST_IF};
    let addr = u32::from_ne_bytes(ip.octets());
    unsafe { setsockopt(s.as_raw_socket() as usize, IPPROTO_IP, IP_MULTICAST_IF, &addr as *const u32 as *const u8, 4) };
}

/// Browse for about `wait`, returning every device that answered.
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
    // A busy network never goes quiet, so keep to the deadline even mid-burst.
    while Instant::now() < end {
        for s in &socks {
            while Instant::now() < end {
                let Ok((n, from)) = s.recv_from(&mut buf) else { break };
                recs.parse(&buf[..n], from);
            }
        }
    }
    recs.found()
}

#[derive(Default)]
struct Records {
    /// (instance, service) in the order they answered.
    instances: Vec<(String, &'static str)>,
    srv: HashMap<String, (String, u16)>,
    a: HashMap<String, Ipv4Addr>,
    /// Who answered for an instance, in case there is no A record.
    from: HashMap<String, Ipv4Addr>,
    txt: HashMap<String, HashMap<String, String>>,
}

/// One device while answers are merged.
#[derive(Default)]
struct Dev {
    name: String,
    hostname: String,
    ip: String,
    port: u16,
    txt: HashMap<String, String>,
    share: bool,
    itda: bool,
}

impl Records {
    fn parse(&mut self, p: &[u8], from: SocketAddr) {
        let Some(rrs) = parse_rrs(p) else { return };
        let src = if let SocketAddr::V4(a) = from { Some(*a.ip()) } else { None };
        for rr in rrs {
            match rr.kind {
                12 => {
                    let Some(svc) = SERVICES.into_iter().find(|s| rr.name.eq_ignore_ascii_case(s)) else { continue };
                    if let Some((inst, _)) = read_name(p, rr.rdata) {
                        if let Some(ip) = src {
                            self.from.insert(inst.clone(), ip);
                        }
                        if !self.instances.iter().any(|(i, _)| *i == inst) {
                            self.instances.push((inst, svc));
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
        // Merge what each device answered (Screen Sharing and Itda) by its host name or IP.
        let mut devs: Vec<Dev> = Vec::new();
        for (inst, svc) in &self.instances {
            let name = inst.strip_suffix(&format!(".{svc}")).unwrap_or(inst).to_string();
            let (hostname, port) = self.srv.get(inst).cloned().unwrap_or_default();
            let ip = self.a.get(&hostname.to_ascii_lowercase()).or(self.from.get(inst)).map(|i| i.to_string()).unwrap_or_default();
            if hostname.is_empty() && ip.is_empty() {
                continue;
            }
            let same = |d: &Dev| (!hostname.is_empty() && d.hostname.eq_ignore_ascii_case(&hostname)) || (!ip.is_empty() && d.ip == ip);
            let i = match devs.iter().position(same) {
                Some(i) => i,
                None => {
                    devs.push(Dev { name: name.clone(), hostname: hostname.clone(), ip: ip.clone(), ..Default::default() });
                    devs.len() - 1
                }
            };
            let d = &mut devs[i];
            if let Some(t) = self.txt.get(inst) {
                for (k, v) in t {
                    d.txt.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            if *svc == RFB {
                d.share = true;
                d.port = if port == 0 { 5900 } else { port };
                d.name = name; // the Screen Sharing name is the one people know
            } else {
                d.itda = true;
            }
            if d.hostname.is_empty() {
                d.hostname = hostname;
            }
            if d.ip.is_empty() {
                d.ip = ip;
            }
        }
        devs.into_iter()
            .map(|d| {
                // macOS always resolves .local names (and a lookup for a device that just left
                // can block for a long time), so only Windows needs to be asked.
                #[cfg(windows)]
                let resolvable = !d.hostname.is_empty() && (d.hostname.as_str(), 5900).to_socket_addrs().is_ok_and(|mut a| a.next().is_some());
                #[cfg(not(windows))]
                let resolvable = !d.hostname.is_empty();
                let mut host = if resolvable { d.hostname.trim_end_matches('.').to_string() } else { d.ip.clone() };
                if d.share && d.port != 5900 {
                    host = format!("{host}:{}", d.port);
                }
                let flag = |k: &str| d.txt.get(k).is_some_and(|v| v == "1");
                Found {
                    name: d.name,
                    host,
                    hostname: d.hostname,
                    ip: d.ip,
                    port: if d.share { d.port } else { 0 },
                    os: d.txt.get("os").cloned().unwrap_or_default(),
                    itda: d.itda || flag("itda"),
                    share: d.share,
                    rdp: flag("rdp"),
                }
            })
            .collect()
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

// ---------------------------------------------------------------- registering our own

/// A service this machine announces; withdrawn when dropped.
pub struct Registration {
    #[cfg(windows)]
    req: Box<windows_sys::Win32::NetworkManagement::Dns::DNS_SERVICE_REGISTER_REQUEST>,
    #[cfg(target_os = "macos")]
    sd: dnssd::Ref,
}

// The handles are only used to withdraw the service, from whichever thread drops it.
unsafe impl Send for Registration {}

/// This machine's name as other devices see it in Bonjour.
pub fn this_name() -> String {
    #[cfg(windows)]
    return std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into());
    #[cfg(not(windows))]
    return std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Mac".into());
}

#[cfg(windows)]
unsafe extern "system" fn registered(status: u32, _ctx: *const core::ffi::c_void, inst: *const windows_sys::Win32::NetworkManagement::Dns::DNS_SERVICE_INSTANCE) {
    crate::log!("bonjour registration status {status}");
    if !inst.is_null() {
        windows_sys::Win32::NetworkManagement::Dns::DnsServiceFreeInstance(inst);
    }
}

/// Announce `service` ("_rfb._tcp" or "_itda._tcp") on `port` with TXT key=values.
#[cfg(windows)]
pub fn register(service: &str, port: u16, txt: &[(&str, &str)]) -> Option<Registration> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::DNS_REQUEST_PENDING;
    use windows_sys::Win32::NetworkManagement::Dns::*;
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let pc = this_name();
    let name = wide(&format!("{pc}.{service}.local"));
    let host = wide(&format!("{pc}.local"));
    let keys: Vec<Vec<u16>> = txt.iter().map(|(k, _)| wide(k)).collect();
    let vals: Vec<Vec<u16>> = txt.iter().map(|(_, v)| wide(v)).collect();
    let kp: Vec<*const u16> = keys.iter().map(|k| k.as_ptr()).collect();
    let vp: Vec<*const u16> = vals.iter().map(|v| v.as_ptr()).collect();
    unsafe {
        let inst = DnsServiceConstructInstance(name.as_ptr(), host.as_ptr(), null(), null(), port, 0, 0, txt.len() as u32, kp.as_ptr(), vp.as_ptr());
        if inst.is_null() {
            return None;
        }
        let mut req: Box<DNS_SERVICE_REGISTER_REQUEST> = Box::new(std::mem::zeroed());
        req.Version = DNS_QUERY_REQUEST_VERSION1;
        req.pServiceInstance = inst;
        req.pRegisterCompletionCallback = Some(registered);
        let r = DnsServiceRegister(&*req, null_mut());
        if r != DNS_REQUEST_PENDING as u32 {
            crate::log!("bonjour register {service} failed: {r}");
            DnsServiceFreeInstance(inst);
            return None;
        }
        Some(Registration { req })
    }
}

#[cfg(windows)]
impl Drop for Registration {
    fn drop(&mut self) {
        use windows_sys::Win32::NetworkManagement::Dns::*;
        unsafe {
            DnsServiceDeRegister(&*self.req, std::ptr::null_mut());
            DnsServiceFreeInstance(self.req.pServiceInstance);
        }
    }
}

#[cfg(target_os = "macos")]
mod dnssd {
    use std::ffi::{c_char, c_void};
    pub type Ref = *mut c_void;
    extern "C" {
        pub fn DNSServiceRegister(
            sd: *mut Ref,
            flags: u32,
            interface: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            host: *const c_char,
            port_be: u16,
            txt_len: u16,
            txt: *const c_void,
            callback: *const c_void,
            context: *mut c_void,
        ) -> i32;
        pub fn DNSServiceRefDeallocate(sd: Ref);
    }
}

/// Announce `service` ("_rfb._tcp" or "_itda._tcp") on `port` with TXT key=values.
#[cfg(target_os = "macos")]
pub fn register(service: &str, port: u16, txt: &[(&str, &str)]) -> Option<Registration> {
    use std::ffi::CString;
    let mut rec = Vec::new();
    for (k, v) in txt {
        let kv = format!("{k}={v}");
        rec.push(kv.len().min(255) as u8);
        rec.extend_from_slice(&kv.as_bytes()[..kv.len().min(255)]);
    }
    let name = CString::new(this_name()).ok()?;
    let regtype = CString::new(service).ok()?;
    let mut sd: dnssd::Ref = std::ptr::null_mut();
    // No callback: mDNSResponder keeps the registration for as long as the reference lives.
    let r = unsafe {
        dnssd::DNSServiceRegister(&mut sd, 0, 0, name.as_ptr(), regtype.as_ptr(), std::ptr::null(), std::ptr::null(), port.to_be(), rec.len() as u16, rec.as_ptr().cast(), std::ptr::null(), std::ptr::null_mut())
    };
    if r != 0 || sd.is_null() {
        crate::log!("bonjour register {service} failed: {r}");
        return None;
    }
    Some(Registration { sd })
}

#[cfg(target_os = "macos")]
impl Drop for Registration {
    fn drop(&mut self) {
        unsafe { dnssd::DNSServiceRefDeallocate(self.sd) };
    }
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

    fn rr(p: &mut Vec<u8>, owner: &str, kind: u16, rdata: &[u8]) {
        name(p, owner);
        p.extend_from_slice(&kind.to_be_bytes());
        p.extend_from_slice(&[0x80, 1, 0, 0, 0, 120]);
        p.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        p.extend_from_slice(rdata);
    }

    fn response(rrs: impl FnOnce(&mut Vec<u8>) -> u16) -> Vec<u8> {
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let n = rrs(&mut p);
        p[7] = n as u8;
        p
    }

    #[test]
    fn parses_mac_response() {
        // PTR + SRV + A, the way macOS answers (SRV target uses compression back to "local").
        let mut p = vec![0, 0, 0x84, 0, 0, 0, 0, 3, 0, 0, 0, 0];
        let svc_off = p.len();
        name(&mut p, RFB);
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
        assert_eq!(r.instances, vec![("Alex's Mac mini._rfb._tcp.local".to_string(), RFB)]);
        assert_eq!(r.srv.values().next(), Some(&("mac-mini.local".to_string(), 5900)));
        assert_eq!(r.a.get("mac-mini.local"), Some(&Ipv4Addr::new(192, 168, 0, 55)));
        let f = r.found();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "Alex's Mac mini");
        assert_eq!(f[0].ip, "192.168.0.55");
        assert!(f[0].share && !f[0].itda);
    }

    #[test]
    fn itda_and_screen_sharing_merge_into_one_device() {
        let txt = |kv: &[&str]| kv.iter().flat_map(|s| std::iter::once(s.len() as u8).chain(s.bytes())).collect::<Vec<u8>>();
        let srv = |port: u16, target: &str| {
            let mut v = vec![0, 0, 0, 0];
            v.extend_from_slice(&port.to_be_bytes());
            name(&mut v, target);
            v
        };
        let ptr = |inst: &str| {
            let mut v = Vec::new();
            name(&mut v, inst);
            v
        };
        let mut r = Records::default();
        // A PC with Itda running and sharing off, Remote Desktop on.
        let p = response(|p| {
            rr(p, ITDA, 12, &ptr("STUDY-PC._itda._tcp.local"));
            rr(p, "STUDY-PC._itda._tcp.local", 33, &srv(8780, "STUDY-PC.local"));
            rr(p, "STUDY-PC._itda._tcp.local", 16, &txt(&["itda=1", "os=windows", "share=0", "rdp=1"]));
            rr(p, "STUDY-PC.local", 1, &[192, 168, 0, 40]);
            4
        });
        r.parse(&p, "192.168.0.40:5353".parse().unwrap());
        // A Mac sharing its screen with Itda running too.
        let p = response(|p| {
            rr(p, RFB, 12, &ptr("Mac mini._rfb._tcp.local"));
            rr(p, "Mac mini._rfb._tcp.local", 33, &srv(5900, "mac-mini.local"));
            rr(p, ITDA, 12, &ptr("Mac mini._itda._tcp.local"));
            rr(p, "Mac mini._itda._tcp.local", 33, &srv(8780, "mac-mini.local"));
            rr(p, "Mac mini._itda._tcp.local", 16, &txt(&["itda=1", "os=mac"]));
            rr(p, "mac-mini.local", 1, &[192, 168, 0, 55]);
            6
        });
        r.parse(&p, "192.168.0.55:5353".parse().unwrap());
        let f = r.found();
        assert_eq!(f.len(), 2, "{f:?}");
        let pc = f.iter().find(|d| d.ip == "192.168.0.40").unwrap();
        assert!(pc.itda && !pc.share && pc.rdp && pc.os == "windows" && pc.port == 0);
        let mac = f.iter().find(|d| d.ip == "192.168.0.55").unwrap();
        assert!(mac.itda && mac.share && mac.os == "mac" && mac.name == "Mac mini" && mac.port == 5900);
    }
}
