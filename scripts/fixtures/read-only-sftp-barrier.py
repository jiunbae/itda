"""Read-only loopback SFTP fixture. Exposes only generated files, never host files.
Run in /tmp/itda-quality-sftp-venv; prints the private receipt path (not its password).
A hold-read file pauses payload.bin after the first 2 MiB returned; release-read resumes it.
No product acceptance credit by itself; this is private transfer lifecycle infrastructure.
"""
import argparse, base64, hashlib, hmac, json, os, pathlib, posixpath, secrets, signal, socket, stat, tempfile, threading, time, unicodedata
import paramiko
args=argparse.ArgumentParser();args.add_argument('--workspace');options=args.parse_args()
workspace = pathlib.Path(options.workspace) if options.workspace else pathlib.Path(tempfile.mkdtemp(prefix='itda-quality-sftp-'))
if workspace.parent.resolve()!=pathlib.Path(tempfile.gettempdir()).resolve() or not workspace.name.startswith('itda-quality-sftp-') or not workspace.is_dir() or list(workspace.iterdir()): raise ValueError('Expected own empty fixture workspace')
os.chmod(workspace, 0o700)
root = workspace / 'files'; root.mkdir()
name = unicodedata.normalize('NFD', '정보공개 결정통지서(접수번호_17780155).pdf')
# A small valid synthetic PDF; no user document content.
objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>', b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R >>', b'<< /Length 0 >>\nstream\n\nendstream']
pdf = bytearray(b'%PDF-1.4\n'); offsets = [0]
for i, obj in enumerate(objects, 1):
    offsets.append(len(pdf)); pdf.extend(f'{i} 0 obj\n'.encode() + obj + b'\nendobj\n')
xref = len(pdf); pdf.extend(b'xref\n0 5\n0000000000 65535 f \n')
for offset in offsets[1:]: pdf.extend(f'{offset:010} 00000 n \n'.encode())
pdf.extend(f'trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()); (root / name).write_bytes(pdf)
(root / 'payload.bin').write_bytes(bytes(range(256)) * 16384)
folder = root / unicodedata.normalize('NFD', '테스트 폴더'); folder.mkdir(); (folder / unicodedata.normalize('NFD', '메모.txt')).write_text('Synthetic native virtual-file folder fixture\n', encoding='utf8')
user = 'itda-fixture'; password = secrets.token_hex(24)
key = paramiko.RSAKey.generate(2048)
sock = socket.socket(); sock.bind(('127.0.0.1', 0)); sock.listen(8); sock.settimeout(1)
stop = threading.Event(); transports = []; lock = threading.Lock()
def audit(event, **fields):
    with lock:
        with (workspace / 'audit.jsonl').open('a') as f: f.write(json.dumps({'event':event, **fields}, ensure_ascii=False) + '\n')
class Auth(paramiko.ServerInterface):
    def check_auth_password(self, username, supplied):
        return paramiko.AUTH_SUCCESSFUL if hmac.compare_digest(username, user) and hmac.compare_digest(supplied, password) else paramiko.AUTH_FAILED
    def get_allowed_auths(self, username): return 'password'
    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == 'session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
class Handle(paramiko.SFTPHandle):
    def read(self, offset, length):
        # Requests already buffered by SFTP must not touch the fixture after abort.
        if self.aborted or stop.is_set() or not self.transport.is_active():
            audit('read-rejected', path=self.logical, offset=offset)
            return paramiko.SFTP_FAILURE
        if self.logical == '/payload.bin' and offset >= (2 << 20) and not self.held and (workspace / 'hold-read').is_file():
            self.held = True
            audit('read-blocked', path=self.logical, offset=offset)
            deadline = time.monotonic() + 120
            while not (workspace / 'release-read').is_file():
                stopped = stop.wait(.05)
                reason = 'shutdown' if stopped else 'transport_closed' if not self.transport.is_active() else 'timeout' if time.monotonic() > deadline else None
                if reason:
                    self.aborted = True
                    audit('read-aborted', path=self.logical, offset=offset, reason=reason)
                    return paramiko.SFTP_FAILURE
        data = super().read(offset, length)
        if isinstance(data, bytes): audit('read-bytes', path=self.logical, offset=offset, length=len(data))
        return data
    def stat(self): return paramiko.SFTPAttributes.from_stat(os.fstat(self.readfile.fileno()))
class Files(paramiko.SFTPServerInterface):
    def __init__(self, server, *args, **kwargs):
        super().__init__(server, *args, **kwargs)
        self.transport = server.transport
    def canonicalize(self, path): return posixpath.normpath('/' + path.lstrip('/'))
    def local(self, path):
        candidate = (root / self.canonicalize(path).lstrip('/')).resolve()
        if root.resolve() != candidate and root.resolve() not in candidate.parents: raise PermissionError('Outside fixture')
        return candidate
    def list_folder(self, path):
        try:
            result=[]
            for p in self.local(path).iterdir():
                a=paramiko.SFTPAttributes.from_stat(p.stat()); a.filename=p.name; result.append(a)
            audit('list', path=path); return result
        except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
    def stat(self, path):
        try: return paramiko.SFTPAttributes.from_stat(self.local(path).stat())
        except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
    lstat = stat
    def open(self, path, flags, attr):
        if flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND): return paramiko.SFTP_PERMISSION_DENIED
        try:
            h=Handle(flags); h.logical=path; h.held=False; h.aborted=False; h.transport=self.transport; h.readfile=self.local(path).open('rb'); audit('read-open', path=path); return h
        except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno)
def connection(client):
    t=paramiko.Transport(client); transports.append(t)
    try:
        t.add_server_key(key); t.set_subsystem_handler('sftp', paramiko.SFTPServer, Files); auth=Auth(); auth.transport=t; t.start_server(server=auth)
        while t.is_active() and not stop.wait(.2): pass
    except Exception as e: audit('connection-error', kind=type(e).__name__)
    finally: t.close()
manifest = {'pid':os.getpid(), 'workspace':str(workspace), 'host':'127.0.0.1', 'port':sock.getsockname()[1], 'user':user, 'password':password, 'host_key_type':key.get_name(), 'host_key_base64':key.get_base64(), 'paramiko_version':paramiko.__version__, 'files':[{'path':'/'+p.relative_to(root).as_posix(),'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'size':p.stat().st_size} for p in root.rglob('*') if p.is_file()]}
receipt=workspace/'connection.json'; receipt.write_text(json.dumps(manifest, ensure_ascii=False, indent=2)); os.chmod(receipt,0o600)
print(str(receipt), flush=True)
for sig in (signal.SIGINT, signal.SIGTERM): signal.signal(sig, lambda *_: stop.set())
try:
    while not stop.is_set():
        try: client,_=sock.accept()
        except socket.timeout: continue
        threading.Thread(target=connection,args=(client,),daemon=True).start()
finally:
    stop.set(); sock.close()
    for t in transports: t.close()
    audit('stopped')
    receipt.unlink(missing_ok=True)
