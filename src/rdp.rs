//! RDP → RFB bridge. Connects to a Windows PC over RDP (TLS + NLA/CredSSP, IronRDP) and
//! looks to the caller like a VNC server that has already authenticated and sent
//! ServerInit, so the web gateway relays it to noVNC exactly like a Mac.
//!
//! A bridge thread owns the RDP session and one end of a loopback socket pair; it decodes
//! RDP graphics into a framebuffer, answers the browser's update requests with the same
//! tile-diff + ZRLE encoder the "Share this PC" server uses, and turns RFB key/pointer
//! events into RDP fast-path input.
//!
//! Dynamic resolution: the desktop starts at the size the viewer asked for, and an RFB
//! SetDesktopSize (noVNC's "resize session") becomes a Display Control monitor layout, so
//! the Windows desktop follows the window. Its answer is an ExtendedDesktopSize update.

use crate::rfb::{self, Raw, Rx, ENC_CURSOR, ENC_DESKTOP_SIZE, ENC_EXT_DESKTOP_SIZE, ENC_ZRLE, PIXEL_FORMAT};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::DrdynvcClient;
use crate::demand::{self, Demand};
use crate::enc::{Damage, Encoder, PixelFormat, Update};
use crate::outbox::Outbox;
use ironrdp_blocking::Framed;
use ironrdp_connector::sspi;
use ironrdp_connector::connection_activation::{ConnectionActivationFactory, ConnectionActivationState};
use ironrdp_connector::{ClientConnector, ConnectorErrorKind, Credentials, DesktopSize, Sequence};
use ironrdp_graphics::image_processing::PixelFormat as RdpPixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::mouse::{MousePdu, PointerFlags};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_pdu::WriteBuf;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// The desktop we ask for when the viewer doesn't say; the browser scales it to fit.
const DESKTOP: DesktopSize = DesktopSize { width: 1920, height: 1080 };
/// Display Control limits (MS-RDPEDISP): 200…8192, even width.
fn clamp_size(w: u32, h: u32) -> (u32, u32) {
    ((w.clamp(200, 8192)) & !1, h.clamp(200, 8192))
}
/// Windows wants a breath between layout changes; the latest request wins.
const RESIZE_GAP: Duration = Duration::from_millis(300);
/// How long one RDP read waits before the bridge looks at browser input again.
const POLL: Duration = Duration::from_millis(10);
/// At most one framebuffer update per this interval (~60 fps).
const FRAME: Duration = Duration::from_millis(16);

/// Log in to an RDP host ("name" or "name:port"), asking for a `size` desktop (else 1920×1080).
/// On success the returned `Raw` behaves like a VNC connection right after ServerInit (32bpp,
/// no auth left to do).
/// Also gives the server's key (SHA-256 of its public key, hex). With `expect`, a server
/// showing another key is refused before the login is sent.
pub fn open(host: &str, user: &str, pass: &str, size: Option<(u32, u32)>, expect: Option<&str>) -> io::Result<(Raw, String)> {
    crate::log!("rdp open {host} {size:?}");
    let r = connect(host, user, pass, size, expect);
    if let Err(e) = &r {
        crate::log!("rdp failed: {e} [{}]", rfb::classify(e));
    }
    r
}

fn connect(host: &str, user: &str, pass: &str, size: Option<(u32, u32)>, expect: Option<&str>) -> io::Result<(Raw, String)> {
    let (name, port) = split(host);
    if user.trim().is_empty() {
        return Err(rfb::denied("사용자 이름을 입력하세요 (Windows 계정 이름)"));
    }
    let addrs: Vec<SocketAddr> = (name.as_str(), port).to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return rfb::err("호스트를 찾을 수 없습니다");
    }
    let mut last = io::Error::other("연결할 수 없습니다");
    let mut tcp = None;
    for a in &addrs {
        match TcpStream::connect_timeout(a, Duration::from_secs(10)) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = e,
        }
    }
    let Some(tcp) = tcp else { return Err(last) };
    tcp.set_nodelay(true)?;
    tcp.set_read_timeout(Some(Duration::from_secs(20)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(20)))?;
    let sock = tcp.try_clone()?;

    let (username, domain) = match user.split_once('\\') {
        Some((d, u)) if !d.is_empty() && d != "." => (u.to_string(), Some(d.to_string())),
        Some((_, u)) => (u.to_string(), None),
        None => (user.to_string(), None),
    };
    let client_name: String = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Itda".into()).chars().take(15).collect();
    let config = ironrdp_connector::Config {
        credentials: Credentials::UsernamePassword { username, password: pass.to_string() },
        domain,
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: size.map_or(DESKTOP, |(w, h)| {
            let (w, h) = clamp_size(w, h);
            DesktopSize { width: w as u16, height: h as u16 }
        }),
        bitmap: None,
        client_build: 0,
        client_name,
        client_dir: "C:\\Windows\\System32\\mstscax.dll".into(),
        platform: MajorPlatformType::WINDOWS,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        compression_type: None,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::DISABLE_FULLWINDOWDRAG | PerformanceFlags::DISABLE_MENUANIMATIONS | PerformanceFlags::ENABLE_FONT_SMOOTHING,
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    };

    let local = tcp.local_addr()?;
    let mut framed = Framed::new(tcp);
    // Display Control: lets the desktop follow the viewer's size.
    let dvc = DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())));
    let mut connector = ClientConnector::new(config, local).with_static_channel(dvc);
    let upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector).map_err(conn_err)?;
    let (tls, server_key) = tls_upgrade(framed.into_inner_no_leftover(), &name)?;
    let key_id = key_id(&server_key);
    if expect.is_some_and(|e| e != key_id) {
        return Err(rfb::denied("이 주소의 원격 데스크톱은 저장된 기기가 아닙니다 (서버 키가 다름)"));
    }
    let upgraded = ironrdp_blocking::mark_as_upgraded(upgrade, &mut connector);
    let mut framed = Framed::new(tls);
    let result = ironrdp_blocking::connect_finalize(upgraded, connector, &mut framed, &mut NoKdc, name.clone().into(), server_key, None)
        .map_err(conn_err)?;
    let (w, h) = (result.desktop_size.width, result.desktop_size.height);
    crate::log!("rdp {host}: connected {w}x{h}");

    // Loopback pair: the caller gets one end, the bridge thread speaks RFB on the other.
    let l = TcpListener::bind("127.0.0.1:0")?;
    let ours = TcpStream::connect(l.local_addr()?)?;
    let theirs = loop {
        let (s, peer) = l.accept()?;
        if peer == ours.local_addr()? {
            break s;
        }
    };
    drop(l);
    ours.set_nodelay(true)?;
    theirs.set_nodelay(true)?;

    let title = format!("{name} (RDP)");
    let mut server_init = Vec::new();
    server_init.extend_from_slice(&w.to_be_bytes());
    server_init.extend_from_slice(&h.to_be_bytes());
    server_init.extend_from_slice(&PIXEL_FORMAT);
    server_init.extend_from_slice(&(title.len() as u32).to_be_bytes());
    server_init.extend_from_slice(title.as_bytes());

    // Per handle on Windows: set it on the stream the bridge reads, not a clone.
    framed.get_inner_mut().0.sock.set_read_timeout(Some(POLL))?;
    let host = host.to_string();
    std::thread::spawn(move || {
        let r = theirs.try_clone().and_then(|c| Bridge::new(framed, result, c)).and_then(Bridge::run);
        let _ = theirs.shutdown(Shutdown::Both);
        let _ = sock.shutdown(Shutdown::Both);
        match r {
            Ok(()) => crate::log!("rdp {host}: closed"),
            Err(e) => crate::log!("rdp {host}: {e}"),
        }
    });

    Ok((Raw {
        reader: BufReader::with_capacity(1 << 20, Rx(ours.try_clone()?)),
        writer: ours,
        width: w,
        height: h,
        name: title,
        server_init,
    }, key_id))
}

/// SHA-256 of the server's public key, hex: what identifies an RDP server between connections.
fn key_id(key: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(key).iter().map(|b| format!("{b:02x}")).collect()
}

/// "host[:port]" with RDP's default port; bracketed IPv6 allowed.
fn split(host: &str) -> (String, u16) {
    let host = host.trim();
    if let Some(rest) = host.strip_prefix('[') {
        if let Some((h, p)) = rest.split_once(']') {
            return (h.to_string(), p.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(3389));
        }
    }
    match host.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(3389)),
        _ => (host.to_string(), 3389),
    }
}

/// Connector errors → io errors that `rfb::classify` explains to the person.
fn conn_err(e: ironrdp_connector::ConnectorError) -> io::Error {
    crate::log!("rdp connector: {e:?}");
    match e.kind() {
        ConnectorErrorKind::AccessDenied => rfb::denied("이 계정은 원격 데스크톱으로 로그인할 수 없습니다 (원격 데스크톱 사용자 권한 확인)"),
        ConnectorErrorKind::Credssp(_) => rfb::denied("RDP 로그인 실패: 사용자 이름 또는 암호가 맞지 않습니다"),
        ConnectorErrorKind::Negotiation(f) => io::Error::other(format!("원격 데스크톱 보안 협상 실패: {f}")),
        _ => {
            // Keep the socket error's kind (reset, timeout…) when there is one underneath.
            let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
            while let Some(s) = src {
                if let Some(io) = s.downcast_ref::<io::Error>() {
                    return io::Error::new(io.kind(), format!("RDP 연결 오류: {io}"));
                }
                src = s.source();
            }
            io::Error::other(format!("RDP 연결 오류: {e}"))
        }
    }
}

/// Kerberos would need a KDC; NTLM inside CredSSP is what home and work PCs accept.
struct NoKdc;

impl sspi::network_client::NetworkClient for NoKdc {
    fn send(&self, _: &sspi::generator::NetworkRequest) -> sspi::Result<Vec<u8>> {
        Err(sspi::Error::new(sspi::ErrorKind::NoAuthenticatingAuthority, "Kerberos is not supported"))
    }
}

fn tls_upgrade(tcp: TcpStream, name: &str) -> io::Result<(Tls, Vec<u8>)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(io::Error::other)?
        .dangerous()
        // RDP hosts use self-signed certificates; NLA binds the session to this key instead.
        .with_custom_certificate_verifier(Arc::new(AnyCert))
        .with_no_client_auth();
    config.resumption = rustls::client::Resumption::disabled(); // CredSSP forbids resumption
    let server_name = rustls::pki_types::ServerName::try_from(name.to_string())
        .unwrap_or_else(|_| rustls::pki_types::ServerName::try_from("rdp").expect("static name"));
    let conn = rustls::ClientConnection::new(Arc::new(config), server_name).map_err(io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    tls.flush()?;
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock)?;
    }
    let cert = tls.conn.peer_certificates().and_then(|c| c.first()).ok_or_else(|| io::Error::other("RDP 서버 인증서가 없습니다"))?;
    use x509_cert::der::Decode;
    let cert = x509_cert::Certificate::from_der(cert).map_err(io::Error::other)?;
    let key = cert.tbs_certificate.subject_public_key_info.subject_public_key.as_bytes().ok_or_else(|| io::Error::other("bad server key"))?.to_vec();
    Ok((tls, key))
}

#[derive(Debug)]
struct AnyCert;

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

// ---------------------------------------------------------------- bridge

enum Msg {
    Format(PixelFormat),
    Encodings(Vec<i32>),
    Request { incremental: bool },
    Key { down: bool, sym: u32 },
    Pointer { mask: u8, x: u16, y: u16 },
    /// SetDesktopSize: the viewer would like this desktop size.
    Resize { w: u16, h: u16 },
    /// EnableContinuousUpdates, Fence.
    Ext(demand::Ext),
}

/// Browser → bridge: parse RFB client messages (after ClientInit) into `Msg`s.
fn read_client(mut r: BufReader<TcpStream>, tx: Sender<Msg>) -> io::Result<()> {
    fn rd<const N: usize>(r: &mut impl Read) -> io::Result<[u8; N]> {
        let mut b = [0u8; N];
        r.read_exact(&mut b)?;
        Ok(b)
    }
    loop {
        let m = match rd::<1>(&mut r)?[0] {
            0 => Msg::Format(PixelFormat::parse(&rd::<19>(&mut r)?[3..])),
            2 => {
                let b: [u8; 3] = rd(&mut r)?;
                let n = u16::from_be_bytes([b[1], b[2]]) as usize;
                let mut e = vec![0u8; n * 4];
                r.read_exact(&mut e)?;
                Msg::Encodings(e.chunks_exact(4).map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect())
            }
            3 => Msg::Request { incremental: rd::<9>(&mut r)?[0] != 0 },
            4 => {
                let b: [u8; 7] = rd(&mut r)?;
                Msg::Key { down: b[0] != 0, sym: u32::from_be_bytes([b[3], b[4], b[5], b[6]]) }
            }
            5 => {
                let b: [u8; 5] = rd(&mut r)?;
                Msg::Pointer { mask: b[0], x: u16::from_be_bytes([b[1], b[2]]), y: u16::from_be_bytes([b[3], b[4]]) }
            }
            6 => {
                // Clipboard text: not carried over RDP yet; skip it.
                let b: [u8; 7] = rd(&mut r)?;
                let n = i32::from_be_bytes([b[3], b[4], b[5], b[6]]).unsigned_abs().min(16 << 20) as u64;
                io::copy(&mut (&mut r).take(n), &mut io::sink())?;
                continue;
            }
            251 => {
                let b: [u8; 7] = rd(&mut r)?;
                io::copy(&mut (&mut r).take(b[5] as u64 * 16), &mut io::sink())?;
                Msg::Resize { w: u16::from_be_bytes([b[1], b[2]]), h: u16::from_be_bytes([b[3], b[4]]) }
            }
            t @ (demand::MSG_CONTINUOUS | demand::MSG_FENCE) => Msg::Ext(demand::read(t, &mut r)?),
            other => return Err(io::Error::other(format!("unknown client message {other}"))),
        };
        if tx.send(m).is_err() {
            return Ok(());
        }
    }
}

struct Bridge {
    framed: Framed<Tls>,
    stage: ActiveStage,
    factory: ConnectionActivationFactory,
    image: DecodedImage,
    rx: Receiver<Msg>,
    /// Everything for the viewer goes through here, in order.
    out: Outbox,
    /// RFB state
    pf: PixelFormat,
    encs: Vec<i32>,
    demand: Demand,
    frames: Frames,
    damage: Damage,
    enc: Encoder,
    update: Update,
    resized: bool,
    /// ExtendedDesktopSize: told the viewer it may ask for sizes; a requested size waiting
    /// for its turn; the answer owed to the viewer (status: 0 = done, 1 = not allowed).
    ext_told: bool,
    pending_size: Option<(u32, u32)>,
    last_resize: Instant,
    resize_reply: Option<u16>,
    cursor: Option<Cursor>,
    last_sent: Instant,
    /// Input state
    keys: Keys,
    buttons: u8,
    pos: (u16, u16),
}

/// A pointer shape waiting to go out: (w, h, hotspot x, hotspot y, 0x00RRGGBB pixels, alpha).
struct Cursor {
    w: u16,
    h: u16,
    hx: u16,
    hy: u16,
    px: Vec<u32>,
    opaque: Vec<bool>,
}

impl Bridge {
    fn new(framed: Framed<Tls>, r: ironrdp_connector::ConnectionResult, client: TcpStream) -> io::Result<Bridge> {
        let (tx, rx) = channel();
        let reader = BufReader::new(client.try_clone()?);
        std::thread::spawn(move || {
            let _ = read_client(reader, tx);
        });
        let image = DecodedImage::new(RdpPixelFormat::BgrX32, r.desktop_size.width, r.desktop_size.height);
        let stage = ActiveStageBuilder {
            static_channels: r.static_channels,
            user_channel_id: r.user_channel_id,
            io_channel_id: r.io_channel_id,
            message_channel_id: r.message_channel_id,
            share_id: r.share_id,
            compression_type: r.compression_type,
            enable_server_pointer: r.enable_server_pointer,
            pointer_software_rendering: r.pointer_software_rendering,
        }
        .build();
        Ok(Bridge {
            framed,
            stage,
            factory: r.activation_factory,
            image,
            rx,
            out: Outbox::new(client)?,
            pf: PixelFormat::parse(&PIXEL_FORMAT),
            encs: Vec::new(),
            demand: Demand::default(),
            frames: Frames::default(),
            damage: Damage::default(),
            enc: Encoder::new(),
            update: Update::default(),
            resized: false,
            ext_told: false,
            pending_size: None,
            last_resize: Instant::now() - RESIZE_GAP,
            resize_reply: None,
            cursor: Some(Cursor::arrow()),
            last_sent: Instant::now() - FRAME,
            keys: Keys::default(),
            buttons: 0,
            pos: (0, 0),
        })
    }

    fn run(mut self) -> io::Result<()> {
        let r = self.pump();
        // Say goodbye to the RDP host so the session is disconnected (not left hanging).
        if let Ok(outs) = self.stage.graceful_shutdown() {
            for o in outs {
                if let ActiveStageOutput::ResponseFrame(f) = o {
                    let _ = self.framed.write_all(&f);
                }
            }
        }
        r
    }

    fn pump(&mut self) -> io::Result<()> {
        loop {
            if self.out.dead() {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "viewer connection lost"));
            }
            // RDP → framebuffer
            match self.framed.read_pdu() {
                Ok((action, payload)) => {
                    let outs = self.stage.process(&mut self.image, action, &payload).map_err(|e| io::Error::other(format!("RDP: {e}")))?;
                    if !self.outputs(outs)? {
                        return Ok(());
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => return Err(e),
            }
            // Browser → RDP
            loop {
                match self.rx.try_recv() {
                    Ok(m) => self.client_msg(m)?,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            self.resize_now()?;
            self.send_update()?;
        }
    }

    /// Handle session outputs; false = the host ended the session.
    fn outputs(&mut self, outs: Vec<ActiveStageOutput>) -> io::Result<bool> {
        for o in outs {
            match o {
                ActiveStageOutput::ResponseFrame(f) => self.framed.write_all(&f)?,
                ActiveStageOutput::GraphicsUpdate(r) => {
                    let (w, h) = (self.image.width() as usize, self.image.height() as usize);
                    let (x, y) = ((r.left as usize).min(w), (r.top as usize).min(h));
                    self.damage.add(x, y, (r.right as usize + 1).min(w).saturating_sub(x), (r.bottom as usize + 1).min(h).saturating_sub(y));
                }
                ActiveStageOutput::PointerBitmap(p) => self.cursor = Some(Cursor::from_rdp(&p)),
                ActiveStageOutput::PointerDefault => self.cursor = Some(Cursor::arrow()),
                ActiveStageOutput::PointerHidden => self.cursor = Some(Cursor::hidden()),
                ActiveStageOutput::Terminate(reason) => {
                    crate::log!("rdp: host ended the session ({reason:?})");
                    return Ok(false);
                }
                ActiveStageOutput::DeactivateAll => self.reactivate()?,
                _ => {}
            }
        }
        Ok(true)
    }

    /// Deactivation-Reactivation (the host changed resolution, reconnected a session…).
    fn reactivate(&mut self) -> io::Result<()> {
        let mut seq = self.factory.create();
        let mut buf = WriteBuf::new();
        loop {
            if let ConnectionActivationState::Finalized { desktop_size, share_id, .. } = seq.connection_activation_state() {
                self.stage.set_share_id(share_id);
                if (desktop_size.width, desktop_size.height) != (self.image.width(), self.image.height()) {
                    crate::log!("rdp: desktop is now {}x{}", desktop_size.width, desktop_size.height);
                    self.image = DecodedImage::new(RdpPixelFormat::BgrX32, desktop_size.width, desktop_size.height);
                    self.resized = true;
                    self.frames.prev.clear();
                }
                self.damage.everything();
                return Ok(());
            }
            buf.clear();
            let written = match seq.next_pdu_hint() {
                Some(hint) => {
                    let pdu = loop {
                        match self.framed.read_by_hint(hint) {
                            Ok(p) => break p,
                            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
                            Err(e) => return Err(e),
                        }
                    };
                    seq.step(&pdu, &mut buf)
                }
                None => seq.step_no_input(&mut buf),
            }
            .map_err(|e| io::Error::other(format!("RDP reactivation: {e}")))?;
            if let Some(n) = written.size() {
                self.framed.write_all(&buf[..n])?;
            }
        }
    }

    fn client_msg(&mut self, m: Msg) -> io::Result<()> {
        let events = match m {
            Msg::Format(p) => {
                self.pf = p;
                self.frames.prev.clear();
                self.damage.everything();
                return Ok(());
            }
            Msg::Encodings(e) => {
                let mut reply = Vec::new();
                self.demand.encodings(&e, &mut reply);
                self.encs = e;
                return self.out.send(reply);
            }
            Msg::Request { incremental } => {
                self.demand.request(incremental);
                return Ok(());
            }
            Msg::Ext(m) => {
                let (w, h) = (self.image.width() as usize, self.image.height() as usize);
                let mut reply = Vec::new();
                if self.demand.ext(m, w, h, &mut reply) {
                    self.frames.prev.clear();
                    self.damage.everything();
                }
                return self.out.send(reply);
            }
            Msg::Resize { w, h } => {
                let size = clamp_size(w as u32, h as u32);
                if size == (self.image.width() as u32, self.image.height() as u32) {
                    self.resize_reply = Some(0);
                } else {
                    self.pending_size = Some(size);
                }
                return Ok(());
            }
            Msg::Key { down, sym } => self.keys.key(down, sym),
            Msg::Pointer { mask, x, y } => self.pointer(mask, x, y),
        };
        if events.is_empty() {
            return Ok(());
        }
        let outs = self.stage.process_fastpath_input(&mut self.image, &events).map_err(|e| io::Error::other(format!("RDP input: {e}")))?;
        self.outputs(outs).map(|_| ())
    }

    /// Send the waiting size request once Display Control is up (and not too soon after the
    /// last one). The new size arrives later as a reactivation.
    fn resize_now(&mut self) -> io::Result<()> {
        let Some((w, h)) = self.pending_size else { return Ok(()) };
        if self.last_resize.elapsed() < RESIZE_GAP {
            return Ok(());
        }
        let ready = self
            .stage
            .get_dvc::<DisplayControlClient>()
            .and_then(|d| d.channel_processor_downcast_ref::<DisplayControlClient>())
            .is_some_and(DisplayControlClient::ready);
        if !ready {
            // The host never offered Display Control (older Windows, policy): say no.
            if self.last_resize.elapsed() > Duration::from_secs(5) {
                self.pending_size = None;
                self.resize_reply = Some(1);
            }
            return Ok(());
        }
        self.pending_size = None;
        self.last_resize = Instant::now();
        match self.stage.encode_resize(w, h, None, None) {
            Some(Ok(frame)) => {
                crate::log!("rdp: asking for {w}x{h}");
                self.framed.write_all(&frame)
            }
            Some(Err(e)) => Err(io::Error::other(format!("RDP resize: {e}"))),
            None => {
                self.resize_reply = Some(1);
                Ok(())
            }
        }
    }

    /// One ExtendedDesktopSize rectangle: reason (0 server, 1 this client), status, one screen.
    fn ext_size(out: &mut Vec<u8>, reason: u16, status: u16, w: u16, h: u16) {
        crate::enc::ext_desktop_size(out, reason, status, w, h, &[crate::enc::Screen { id: 0, x: 0, y: 0, w, h }]);
    }

    fn pointer(&mut self, mask: u8, x: u16, y: u16) -> Vec<FastPathInputEvent> {
        let x = x.min(self.image.width().saturating_sub(1));
        let y = y.min(self.image.height().saturating_sub(1));
        let ev = |flags: PointerFlags, wheel: i16| FastPathInputEvent::MouseEvent(MousePdu { flags, number_of_wheel_rotation_units: wheel, x_position: x, y_position: y });
        let mut v = Vec::new();
        if (x, y) != self.pos {
            v.push(ev(PointerFlags::MOVE, 0));
            self.pos = (x, y);
        }
        let changed = mask ^ self.buttons;
        for (bit, flag) in [(1u8, PointerFlags::LEFT_BUTTON), (2, PointerFlags::MIDDLE_BUTTON_OR_WHEEL), (4, PointerFlags::RIGHT_BUTTON)] {
            if changed & bit != 0 {
                v.push(ev(if mask & bit != 0 { flag | PointerFlags::DOWN } else { flag }, 0));
            }
        }
        let press = |bit: u8| changed & bit != 0 && mask & bit != 0;
        if press(8) {
            v.push(ev(PointerFlags::VERTICAL_WHEEL, 120));
        }
        if press(16) {
            v.push(ev(PointerFlags::VERTICAL_WHEEL, -120));
        }
        if press(32) {
            v.push(ev(PointerFlags::HORIZONTAL_WHEEL, -120));
        }
        if press(64) {
            v.push(ev(PointerFlags::HORIZONTAL_WHEEL, 120));
        }
        self.buttons = mask;
        v
    }

    /// Answer a pending FramebufferUpdateRequest (or, with Continuous Updates, push) what
    /// changed (tiles), the new size and pointer shape.
    fn send_update(&mut self) -> io::Result<()> {
        if self.demand.stalled() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "viewer stopped acknowledging updates"));
        }
        let Some(incremental) = self.demand.due() else { return Ok(()) };
        if !self.out.room() {
            return Ok(()); // the viewer is behind: changes keep piling up as damage
        }
        let cursor_due = self.cursor.is_some() && self.encs.contains(&ENC_CURSOR);
        let ext = self.encs.contains(&ENC_EXT_DESKTOP_SIZE);
        let ext_due = ext && (!self.ext_told || self.resize_reply.is_some());
        if (incremental && self.damage.is_empty() && !cursor_due && !self.resized && !ext_due) || self.last_sent.elapsed() < FRAME {
            return Ok(());
        }
        let (w, h) = (self.image.width() as usize, self.image.height() as usize);
        let full = !incremental || self.frames.prev.len() != w * h;
        let whole = self.frames.refresh(self.image.data(), w, h, &self.damage, full);
        let u = &mut self.update;
        u.start();
        if ext && (self.resized || ext_due) {
            // A new size we asked for is the answer to the viewer's request.
            let reason = if self.resized || self.resize_reply.is_some() { 1 } else { 0 };
            Self::ext_size(&mut u.buf, reason, self.resize_reply.take().unwrap_or(0), w as u16, h as u16);
            self.ext_told = true;
            u.rects += 1;
        } else if self.resized && self.encs.contains(&ENC_DESKTOP_SIZE) {
            u.buf.extend_from_slice(&[0, 0, 0, 0]);
            u.buf.extend_from_slice(&(w as u16).to_be_bytes());
            u.buf.extend_from_slice(&(h as u16).to_be_bytes());
            u.buf.extend_from_slice(&ENC_DESKTOP_SIZE.to_be_bytes());
            u.rects += 1;
        }
        self.resized = false;
        if cursor_due {
            if let Some(c) = self.cursor.take() {
                c.encode(&mut u.buf, &self.pf);
                u.rects += 1;
            }
        }
        let (demand, damage) = (&self.demand, &self.damage);
        let want = |x, y, tw, th| demand.covers(x, y, tw, th) && (whole || damage.hits(x, y, tw, th));
        self.enc.changes(u, &self.frames.cur, &self.frames.prev, w, h, full, want, &self.pf, self.encs.contains(&ENC_ZRLE))?;
        if !u.finish() {
            self.damage.clear(); // nothing visible changed: keep the request pending
            return Ok(());
        }
        self.demand.sent(u.buf.len(), &mut u.buf);
        self.out.send(std::mem::replace(&mut u.buf, self.out.buffer()))?;
        self.frames.sent(w, &self.damage, whole);
        self.damage.clear();
        self.last_sent = Instant::now();
        Ok(())
    }
}

/// The framebuffer as last sent to the viewer and as it is now (0x00RRGGBB). `cur` is
/// converted from the RDP image only where the host redrew it.
#[derive(Default)]
struct Frames {
    prev: Vec<u32>,
    cur: Vec<u32>,
}

impl Frames {
    /// Bring `cur` up to date with the w×h BgrX32 `img`: everything when `full` (or when
    /// damage can't say), otherwise the damaged rectangles. True when it was everything.
    fn refresh(&mut self, img: &[u8], w: usize, h: usize, damage: &Damage, full: bool) -> bool {
        let whole = full || damage.is_all() || self.cur.len() != w * h;
        if whole {
            self.cur.resize(w * h, 0);
            to_rgb(img, w, &mut self.cur, (0, 0, w, h));
        } else {
            for &r in damage.rects() {
                to_rgb(img, w, &mut self.cur, r);
            }
        }
        whole
    }

    /// The update went out: what was sent is now `prev` too.
    fn sent(&mut self, w: usize, damage: &Damage, whole: bool) {
        if whole || self.prev.len() != self.cur.len() {
            self.prev.clone_from(&self.cur);
            return;
        }
        for &(x, y, rw, rh) in damage.rects() {
            for row in y..y + rh {
                let a = row * w + x;
                self.prev[a..a + rw].copy_from_slice(&self.cur[a..a + rw]);
            }
        }
    }
}

/// Rectangle (x, y, w, h) of a BgrX32 image `w` pixels wide into `cur` as 0x00RRGGBB.
fn to_rgb(img: &[u8], w: usize, cur: &mut [u32], (x, y, rw, rh): (usize, usize, usize, usize)) {
    for row in y..y + rh {
        let a = row * w + x;
        for (d, p) in cur[a..a + rw].iter_mut().zip(img[a * 4..(a + rw) * 4].chunks_exact(4)) {
            *d = u32::from_le_bytes([p[0], p[1], p[2], 0]);
        }
    }
}

impl Cursor {
    fn from_rdp(p: &DecodedPointer) -> Cursor {
        // RGBA, straight alpha.
        let px = p.bitmap_data.chunks_exact(4).map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32).collect();
        let opaque = p.bitmap_data.chunks_exact(4).map(|c| c[3] >= 0x80).collect();
        Cursor { w: p.width, h: p.height, hx: p.hotspot_x, hy: p.hotspot_y, px, opaque }
    }

    fn hidden() -> Cursor {
        Cursor { w: 0, h: 0, hx: 0, hy: 0, px: Vec::new(), opaque: Vec::new() }
    }

    /// Windows' default arrow, for "use the system pointer".
    fn arrow() -> Cursor {
        const ART: [&str; 17] = [
            "X..........",
            "XX.........",
            "X#X........",
            "X##X.......",
            "X###X......",
            "X####X.....",
            "X#####X....",
            "X######X...",
            "X#######X..",
            "X########X.",
            "X#####XXXXX",
            "X##X##X....",
            "X#X.X##X...",
            "XX..X##X...",
            "X....X##X..",
            ".....X##X..",
            "......XX...",
        ];
        let px = ART.iter().flat_map(|r| r.bytes()).map(|b| if b == b'#' { 0xffffff } else { 0 }).collect();
        let opaque = ART.iter().flat_map(|r| r.bytes()).map(|b| b != b'.').collect();
        Cursor { w: 11, h: 17, hx: 0, hy: 0, px, opaque }
    }

    /// RFB Cursor pseudo-encoding rectangle.
    fn encode(&self, out: &mut Vec<u8>, pf: &PixelFormat) {
        for v in [self.hx, self.hy, self.w, self.h] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&ENC_CURSOR.to_be_bytes());
        let b = pf.bytes();
        for &p in &self.px {
            pf.put(p, out, b, 0);
        }
        let row = (self.w as usize).div_ceil(8);
        for y in 0..self.h as usize {
            let mut bits = vec![0u8; row];
            for x in 0..self.w as usize {
                if self.opaque[y * self.w as usize + x] {
                    bits[x / 8] |= 0x80 >> (x % 8);
                }
            }
            out.extend_from_slice(&bits);
        }
    }
}

// ---------------------------------------------------------------- keyboard

#[derive(Clone, Copy)]
enum Pressed {
    Scan(u8, bool),
    Unicode(u16),
}

#[derive(Default)]
struct Keys {
    /// keysym → what we sent for it, so the release matches.
    down: HashMap<u32, Pressed>,
}

impl Keys {
    fn key(&mut self, down: bool, sym: u32) -> Vec<FastPathInputEvent> {
        if !down {
            return self.down.remove(&sym).map(|p| vec![event(p, false)]).unwrap_or_default();
        }
        let p = if let Some((sc, ext)) = special(sym) {
            Pressed::Scan(sc, ext)
        } else {
            let cp = if sym >= 0x0100_0000 { sym & 0x00ff_ffff } else { sym };
            let Some(ch) = char::from_u32(cp) else { return Vec::new() };
            match us_scancode(ch) {
                // Shortcuts (Ctrl+C, Alt+F…) and space need real keys, not characters.
                Some(sc) if ch == ' ' || self.modifiers_held() => Pressed::Scan(sc, false),
                _ => {
                    let mut b = [0u16; 2];
                    let s = ch.encode_utf16(&mut b);
                    if s.len() != 1 {
                        // Outside the BMP: the surrogate pair as one keystroke.
                        return s.iter().flat_map(|&u| [event(Pressed::Unicode(u), true), event(Pressed::Unicode(u), false)]).collect();
                    }
                    Pressed::Unicode(s[0])
                }
            }
        };
        self.down.insert(sym, p);
        vec![event(p, true)]
    }

    fn modifiers_held(&self) -> bool {
        self.down.values().any(|p| matches!(p, Pressed::Scan(0x1d | 0x38, _) | Pressed::Scan(0x5b | 0x5c, true)))
    }
}

fn event(p: Pressed, down: bool) -> FastPathInputEvent {
    let up = if down { KeyboardFlags::empty() } else { KeyboardFlags::RELEASE };
    match p {
        Pressed::Scan(sc, ext) => FastPathInputEvent::KeyboardEvent(if ext { up | KeyboardFlags::EXTENDED } else { up }, sc),
        Pressed::Unicode(u) => FastPathInputEvent::UnicodeKeyboardEvent(up, u),
    }
}

/// X11 keysym → (set-1 scancode, extended) for keys that are not characters.
fn special(sym: u32) -> Option<(u8, bool)> {
    const KP: [u8; 10] = [0x52, 0x4f, 0x50, 0x51, 0x4b, 0x4c, 0x4d, 0x47, 0x48, 0x49];
    Some(match sym {
        0xff08 => (0x0e, false), // BackSpace
        0xff09 => (0x0f, false), // Tab
        0xff0d => (0x1c, false), // Return
        0xff8d => (0x1c, true),  // KP_Enter
        0xff14 => (0x46, false), // Scroll_Lock
        0xff1b => (0x01, false), // Escape
        0xffff => (0x53, true),  // Delete
        0xff50 => (0x47, true),  // Home
        0xff51 => (0x4b, true),  // Left
        0xff52 => (0x48, true),  // Up
        0xff53 => (0x4d, true),  // Right
        0xff54 => (0x50, true),  // Down
        0xff55 => (0x49, true),  // Prior
        0xff56 => (0x51, true),  // Next
        0xff57 => (0x4f, true),  // End
        0xff61 => (0x37, true),  // Print
        0xff63 => (0x52, true),  // Insert
        0xff67 => (0x5d, true),  // Menu
        0xff7f => (0x45, false), // Num_Lock
        0xff31 => (0x72, false), // Hangul
        0xff34 => (0x71, false), // Hangul_Hanja
        0xffaa => (0x37, false), // KP_Multiply
        0xffab => (0x4e, false), // KP_Add
        0xffad => (0x4a, false), // KP_Subtract
        0xffae => (0x53, false), // KP_Decimal
        0xffaf => (0x35, true),  // KP_Divide
        0xffb0..=0xffb9 => (KP[(sym - 0xffb0) as usize], false),
        0xffbe..=0xffc7 => (0x3b + (sym - 0xffbe) as u8, false), // F1–F10
        0xffc8 => (0x57, false),                                  // F11
        0xffc9 => (0x58, false),                                  // F12
        0xffe1 => (0x2a, false),                                  // Shift_L
        0xffe2 => (0x36, false),                                  // Shift_R
        0xffe3 => (0x1d, false),                                  // Control_L
        0xffe4 => (0x1d, true),                                   // Control_R
        0xffe5 => (0x3a, false),                                  // Caps_Lock
        0xffe7 | 0xffe9 => (0x38, false),                         // Meta_L, Alt_L
        0xffe8 | 0xffea => (0x38, true),                          // Meta_R, Alt_R
        0xffeb => (0x5b, true),                                   // Super_L
        0xffec => (0x5c, true),                                   // Super_R
        _ => return None,
    })
}

/// The US-layout key that types `ch` (shifted symbols map to their key).
fn us_scancode(ch: char) -> Option<u8> {
    const ROW_Q: &[u8] = b"qwertyuiop";
    const ROW_A: &[u8] = b"asdfghjkl";
    const ROW_Z: &[u8] = b"zxcvbnm";
    let c = ch.to_ascii_lowercase();
    if !c.is_ascii() {
        return None;
    }
    let b = c as u8;
    let pos = |row: &[u8]| row.iter().position(|&x| x == b).map(|i| i as u8);
    Some(match b {
        b'a'..=b'z' => pos(ROW_Q).map(|i| 0x10 + i).or_else(|| pos(ROW_A).map(|i| 0x1e + i)).or_else(|| pos(ROW_Z).map(|i| 0x2c + i))?,
        b'1'..=b'9' => 0x02 + (b - b'1'),
        b'0' => 0x0b,
        b'!' => 0x02,
        b'@' => 0x03,
        b'#' => 0x04,
        b'$' => 0x05,
        b'%' => 0x06,
        b'^' => 0x07,
        b'&' => 0x08,
        b'*' => 0x09,
        b'(' => 0x0a,
        b')' => 0x0b,
        b'-' | b'_' => 0x0c,
        b'=' | b'+' => 0x0d,
        b'[' | b'{' => 0x1a,
        b']' | b'}' => 0x1b,
        b';' | b':' => 0x27,
        b'\'' | b'"' => 0x28,
        b'`' | b'~' => 0x29,
        b'\\' | b'|' => 0x2b,
        b',' | b'<' => 0x33,
        b'.' | b'>' => 0x34,
        b'/' | b'?' => 0x35,
        b' ' => 0x39,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converting and comparing only damaged areas sends exactly what a full conversion and
    /// full comparison would, over random redraws (some skipped updates included).
    #[test]
    fn damage_tracks_full_diff() {
        let (w, h) = (200, 150);
        let mut img = vec![0u8; w * h * 4];
        let mut fr = Frames::default();
        let mut dmg = Damage::default();
        let mut seed = 9u64;
        let mut rnd = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        let pf = PixelFormat::parse(&PIXEL_FORMAT);
        let (mut a, mut b) = (Encoder::new(), Encoder::new());
        let (mut ua, mut ub) = (Update::default(), Update::default());
        let mut reference: Vec<u32> = Vec::new(); // what a full-frame bridge would have sent
        for step in 0..300 {
            for _ in 0..rnd(4) {
                let (x, y) = (rnd(w), rnd(h));
                let (rw, rh) = (1 + rnd(w - x), 1 + rnd(h - y));
                let c = rnd(5) as u8 * 50;
                for row in y..y + rh {
                    for p in img[(row * w + x) * 4..(row * w + x + rw) * 4].chunks_exact_mut(4) {
                        p.copy_from_slice(&[c, c / 2, 255 - c, 0x5a]);
                    }
                }
                dmg.add(x, y, rw, rh);
            }
            if step % 7 == 3 {
                continue; // window full: damage piles up
            }
            let full = fr.prev.len() != w * h || step % 50 == 0;
            let whole = fr.refresh(&img, w, h, &dmg, full);
            ua.start();
            a.changes(&mut ua, &fr.cur, &fr.prev, w, h, full, |x, y, tw, th| whole || dmg.hits(x, y, tw, th), &pf, false).unwrap();
            ua.finish();
            let mut now = vec![0u32; w * h];
            to_rgb(&img, w, &mut now, (0, 0, w, h));
            assert_eq!(fr.cur, now, "step {step}: cur mirrors the image");
            ub.start();
            let rfull = full || reference.len() != now.len();
            b.changes(&mut ub, &now, &reference, w, h, rfull, |_, _, _, _| true, &pf, false).unwrap();
            ub.finish();
            assert_eq!(ua.buf, ub.buf, "step {step}: same update as a full diff");
            fr.sent(w, &dmg, whole);
            dmg.clear();
            reference = now;
            assert_eq!(fr.prev, reference, "step {step}");
        }
    }

    /// End-to-end against a test RDP server: ITDA_RDP_TEST="host:port|user|pass".
    /// The server draws R = x & 255, G = y & 255, B = 0x80 on an 800x600 desktop.
    #[test]
    #[ignore]
    fn bridge_e2e() {
        let cfg = std::env::var("ITDA_RDP_TEST").expect("ITDA_RDP_TEST");
        let mut it = cfg.split('|');
        let (host, user, pass) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        crate::log::init("rdptest");

        let e = open(host, user, "wrong-password", None, None).err().expect("bad password must fail");
        assert_eq!(rfb::classify(&e), "auth", "{e}");
        // Another server key than the one expected: refused before the login.
        let e = open(host, user, pass, None, Some("00")).err().expect("a different server key must fail");
        assert!(e.to_string().contains("서버 키"), "{e}");

        let (raw, key) = open(host, user, pass, None, None).expect("open");
        assert_eq!(key.len(), 64);
        drop(raw);
        let (raw, _) = open(host, user, pass, None, Some(&key)).expect("open with its own key");
        assert_eq!((raw.width, raw.height), (800, 600));
        assert_eq!(&raw.server_init[..4], &[3, 32, 2, 88]);
        assert_eq!(&raw.server_init[4..20], &PIXEL_FORMAT);
        let (mut r, mut w) = (raw.reader, raw.writer);
        // SetEncodings: Raw, Cursor, DesktopSize
        let mut m = vec![2, 0, 0, 3];
        for e in [0i32, ENC_CURSOR, ENC_DESKTOP_SIZE] {
            m.extend_from_slice(&e.to_be_bytes());
        }
        w.write_all(&m).unwrap();
        let mut fb = vec![0u32; 800 * 600];
        let want = |x: usize, y: usize| ((x & 255) << 16 | (y & 255) << 8 | 0x80) as u32;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut incremental = 0u8;
        let mut cursor = false;
        // The test server sends RemoteFX (lossy): allow small colour error.
        let near = |fb: &[u32], x: usize, y: usize| {
            let (a, b) = (fb[y * 800 + x], want(x, y));
            (0..3).all(|i| ((a >> (8 * i)) & 255).abs_diff((b >> (8 * i)) & 255) <= 12)
        };
        let points = [(300, 50), (10, 10), (700, 500), (450, 300), (128, 590)];
        while !points.iter().all(|&(x, y)| near(&fb, x, y)) {
            assert!(Instant::now() < deadline, "pattern never arrived");
            w.write_all(&[3, incremental, 0, 0, 0, 0, 3, 32, 2, 88]).unwrap();
            incremental = 1;
            assert_eq!(rfb::rd_u8(&mut r).unwrap(), 0);
            rfb::rd_u8(&mut r).unwrap();
            for _ in 0..rfb::rd_u16(&mut r).unwrap() {
                let (x, y, rw, rh) = (rfb::rd_u16(&mut r).unwrap() as usize, rfb::rd_u16(&mut r).unwrap() as usize, rfb::rd_u16(&mut r).unwrap() as usize, rfb::rd_u16(&mut r).unwrap() as usize);
                match rfb::rd_u32(&mut r).unwrap() as i32 {
                    0 => {
                        for row in y..y + rh {
                            for col in x..x + rw {
                                let p = rfb::rd_vec(&mut r, 4).unwrap();
                                fb[row * 800 + col] = u32::from_le_bytes([p[0], p[1], p[2], 0]);
                            }
                        }
                    }
                    ENC_CURSOR => {
                        rfb::rd_vec(&mut r, rw * rh * 4 + rw.div_ceil(8) * rh).unwrap();
                        cursor = true;
                    }
                    other => panic!("unexpected encoding {other}"),
                }
            }
        }
        assert!(cursor, "pointer shape was not sent");
        // Input: 'a', Ctrl+C, a move and a left click, a wheel notch.
        let key = |down: u8, sym: u32| {
            let mut m = vec![4, down, 0, 0];
            m.extend_from_slice(&sym.to_be_bytes());
            m
        };
        for m in [key(1, 'a' as u32), key(0, 'a' as u32), key(1, 0xffe3), key(1, 'c' as u32), key(0, 'c' as u32), key(0, 0xffe3), key(1, 0xac00), key(0, 0xac00)] {
            w.write_all(&m).unwrap();
        }
        for (mask, x, y) in [(0u8, 10u16, 20u16), (1, 10, 20), (0, 10, 20), (8, 10, 20), (0, 10, 20)] {
            let mut m = vec![5, mask];
            m.extend_from_slice(&x.to_be_bytes());
            m.extend_from_slice(&y.to_be_bytes());
            w.write_all(&m).unwrap();
        }
        std::thread::sleep(Duration::from_millis(500));
        // Closing our end must end the RDP session too.
        w.shutdown(Shutdown::Both).unwrap();
        std::thread::sleep(Duration::from_millis(500));
    }

    /// Dynamic resolution against the test server in resize mode:
    /// ITDA_RDP_TEST="host:port|user|pass" with `rdpsrv <port> <user> <pass> resize`.
    #[test]
    #[ignore]
    fn bridge_resize() {
        let cfg = std::env::var("ITDA_RDP_TEST").expect("ITDA_RDP_TEST");
        let mut it = cfg.split('|');
        let (host, user, pass) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
        crate::log::init("rdptest");

        // (Windows starts at the size asked for here; the IronRDP test server keeps its own.)
        let (raw, _) = open(host, user, pass, Some((1001, 701)), None).expect("open");
        let (w0, h0) = (raw.width as usize, raw.height as usize);
        let (mut r, mut w) = (raw.reader, raw.writer);
        let mut m = vec![2, 0, 0, 3];
        for e in [0i32, ENC_EXT_DESKTOP_SIZE, ENC_DESKTOP_SIZE] {
            m.extend_from_slice(&e.to_be_bytes());
        }
        w.write_all(&m).unwrap();
        // Read updates until an ExtendedDesktopSize arrives: (reason, status, w, h).
        let mut size = (w0, h0);
        let next_ext = |w: &mut TcpStream, r: &mut BufReader<Rx>, size: &mut (usize, usize)| -> (u16, u16, usize, usize) {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                assert!(Instant::now() < deadline, "no ExtendedDesktopSize");
                w.write_all(&[3, 1, 0, 0, 0, 0, (size.0 >> 8) as u8, size.0 as u8, (size.1 >> 8) as u8, size.1 as u8]).unwrap();
                assert_eq!(rfb::rd_u8(r).unwrap(), 0);
                rfb::rd_u8(r).unwrap();
                let mut found = None;
                for _ in 0..rfb::rd_u16(r).unwrap() {
                    let (x, y, rw, rh) = (rfb::rd_u16(r).unwrap(), rfb::rd_u16(r).unwrap(), rfb::rd_u16(r).unwrap() as usize, rfb::rd_u16(r).unwrap() as usize);
                    match rfb::rd_u32(r).unwrap() as i32 {
                        0 => {
                            rfb::rd_vec(r, rw * rh * 4).unwrap();
                        }
                        ENC_EXT_DESKTOP_SIZE => {
                            let n = rfb::rd_u8(r).unwrap() as usize;
                            rfb::rd_vec(r, 3 + n * 16).unwrap();
                            *size = (rw, rh);
                            found = Some((x, y, rw, rh));
                        }
                        other => panic!("unexpected encoding {other}"),
                    }
                }
                if let Some(f) = found {
                    return f;
                }
            }
        };
        let first = next_ext(&mut w, &mut r, &mut size);
        assert_eq!(first, (0, 0, w0, h0), "first update says resizing is possible");
        // SetDesktopSize 1281x800 → the host reactivates at 1280x800 (even width).
        let mut req = vec![251, 0];
        req.extend_from_slice(&1281u16.to_be_bytes());
        req.extend_from_slice(&800u16.to_be_bytes());
        req.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 3, 32, 0, 0, 0, 0]);
        w.write_all(&req).unwrap();
        let done = next_ext(&mut w, &mut r, &mut size);
        assert_eq!(done, (1, 0, 1280, 800), "resized as asked");
        // Asking for the current size is answered at once.
        let mut req = vec![251, 0];
        req.extend_from_slice(&1280u16.to_be_bytes());
        req.extend_from_slice(&800u16.to_be_bytes());
        req.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 3, 32, 0, 0, 0, 0]);
        w.write_all(&req).unwrap();
        assert_eq!(next_ext(&mut w, &mut r, &mut size), (1, 0, 1280, 800));
        w.shutdown(Shutdown::Both).unwrap();
    }

    #[test]
    fn host_port() {
        assert_eq!(split("pc"), ("pc".into(), 3389));
        assert_eq!(split("pc:3390"), ("pc".into(), 3390));
        assert_eq!(split("[fe80::1]:3391"), ("fe80::1".into(), 3391));
        assert_eq!(split("fe80::1"), ("fe80::1".into(), 3389));
    }

    #[test]
    fn scancodes() {
        assert_eq!(us_scancode('a'), Some(0x1e));
        assert_eq!(us_scancode('Q'), Some(0x10));
        assert_eq!(us_scancode('m'), Some(0x32));
        assert_eq!(us_scancode('0'), Some(0x0b));
        assert_eq!(us_scancode('한'), None);
        assert_eq!(special(0xffc9), Some((0x58, false)));
        assert_eq!(special(0xffbe + 9), Some((0x44, false)));
    }
}
