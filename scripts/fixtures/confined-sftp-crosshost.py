"""Destination-only generated fixture. Importing defines confinement, starts nothing.
Serve only an explicit empty owned temp root; no shell, forwarding or host-file access.
The exact payload/folder allowlist plus staging-only writes bounds the native copy.
"""
import errno, hmac, json, os, pathlib, re, secrets, signal, socket, stat, tempfile, threading, time
PDF = '정보공개 결정통지서(접수번호_00000000).pdf'
FOLDER = '테스트 폴더'
FILES = {PDF, 'payload.bin', FOLDER + '/메모.txt'}
# Pinned v1.0.6 store::new_id(): six random bytes, exactly twelve hex digits.
STAGE = re.compile(r'\.itda-[a-f0-9]{12}\.partial\Z')
MAX_FILE = 2 * 1024 * 1024

class Confined:
    def __init__(self, root):
        self.root = pathlib.Path(root).absolute()
        self.lock = threading.RLock()
        self._plain(self.root)
        if self.root.resolve() != self.root: raise PermissionError('Noncanonical root')
    @staticmethod
    def _plain(p):
        s = p.lstat()
        if stat.S_ISLNK(s.st_mode) or getattr(s, 'st_file_attributes', 0) & 0x400:
            raise PermissionError('Link/reparse denied')
    def path(self, value):
        if not isinstance(value, str) or any(ord(c) < 32 for c in value) or '\\' in value or ':' in value or value.startswith('//'):
            raise PermissionError('Invalid path syntax')
        if value in ('', '.', '/'): parts = []
        else:
            parts = value.removeprefix('/').split('/')
            if any(p in ('', '.', '..') or p.endswith((' ', '.')) or re.fullmatch(r'(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?',p,re.IGNORECASE) for p in parts): raise PermissionError('Invalid path component')
        self._plain(self.root)
        current = self.root
        for part in parts:
            current = current / part
            if current.exists() or current.is_symlink(): self._plain(current)
        if not current.is_relative_to(self.root): raise PermissionError('Outside root')
        return current
    def logical(self, value): return self.path(value).relative_to(self.root).as_posix()
    def staging(self, value):
        p = self.path(value)
        if p.parent not in (self.root, self.root / FOLDER) or not STAGE.fullmatch(p.name): raise PermissionError('Only own staging writes allowed')
        return p
    def create(self, value):
        # O_EXCL refuses replacing an existing file/link. No concurrent untrusted
        # writer is allowed in this fixture workspace; recheck every operation.
        with self.lock:
            p = self.staging(value)
            if p.exists(): raise FileExistsError(errno.EEXIST,'Existing staging file')
            if len([q for q in self.root.rglob('*') if q.is_file()]) >= 3:
                raise PermissionError('Only three generated destination files permitted')
            return os.fdopen(os.open(p, os.O_RDWR | os.O_CREAT | os.O_EXCL | getattr(os, 'O_NOFOLLOW', 0), 0o600), 'w+b')
    @staticmethod
    def check_write(offset, length):
        if offset < 0 or length < 0 or offset + length > MAX_FILE:
            raise PermissionError('File write outside fixture byte limit')
    def mkdir(self, value):
        p = self.path(value)
        if p != self.root / FOLDER: raise PermissionError('Unknown directory')
        p.mkdir(mode=0o700)
    def rename(self, source, destination):
        with self.lock:
            a, b = self.staging(source), self.path(destination)
            if self.logical(destination) not in FILES or a.parent != b.parent or b.exists(): raise PermissionError('Unexpected/overwrite rename')
            # Serialize all fixture renames; no external writer is allowed here.
            a.rename(b)
    def remove(self, value): self.staging(value).unlink()

def serve(workspace):
    import paramiko
    workspace = pathlib.Path(workspace).absolute()
    if workspace.parent.resolve() != pathlib.Path(tempfile.gettempdir()).resolve() or not workspace.name.startswith('itda-quality-sftp-'):
        raise PermissionError('Expected owned temporary workspace')
    Confined._plain(workspace)
    if workspace.resolve() != workspace or list(workspace.iterdir()): raise PermissionError('Expected canonical empty workspace')
    root = workspace / 'files'; root.mkdir(); bound = Confined(root)
    stop = threading.Event(); lock = threading.Lock(); transports = []
    def audit(event, **fields):
        with lock:
            with (workspace / 'audit.jsonl').open('a', encoding='utf8') as f: f.write(json.dumps(dict(event=event, unix_ms=time.time_ns()//1000000, **fields), ensure_ascii=False) + '\n')
    def perform(fn):
        try: return fn()
        except PermissionError: audit('denied'); return paramiko.SFTP_PERMISSION_DENIED
        except OSError as e: return paramiko.SFTPServer.convert_errno(e.errno or errno.EIO)
    user, password = 'itda-destination', secrets.token_hex(24)
    key = paramiko.RSAKey.generate(2048)
    class Auth(paramiko.ServerInterface):
        def check_auth_password(self, username, supplied):
            return paramiko.AUTH_SUCCESSFUL if hmac.compare_digest(username, user) and hmac.compare_digest(supplied, password) else paramiko.AUTH_FAILED
        def get_allowed_auths(self, username): return 'password'
        def check_channel_request(self, kind, chanid): return paramiko.OPEN_SUCCEEDED if kind == 'session' else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
    class Handle(paramiko.SFTPHandle):
        def write(self, offset, data):
            try: bound.check_write(offset,len(data))
            except PermissionError: return paramiko.SFTP_PERMISSION_DENIED
            result = super().write(offset, data)
            if result == paramiko.SFTP_OK: audit('write', path=self.logical, offset=offset, bytes=len(data))
            return result
        def stat(self): return paramiko.SFTPAttributes.from_stat(os.fstat(self.writefile.fileno()))
    class Files(paramiko.SFTPServerInterface):
        def canonicalize(self, path):
            # REALPATH . is needed by SFTP; reject traversal rather than normalize it.
            try: return '/' if bound.logical(path) == '.' else '/' + bound.logical(path)
            except PermissionError: return '/invalid-denied'
        def list_folder(self, path):
            def run():
                result=[]
                for p in bound.path(path).iterdir():
                    bound.path('/' + p.relative_to(root).as_posix())
                    a=paramiko.SFTPAttributes.from_stat(p.stat());a.filename=p.name;result.append(a)
                audit('list', path=path);return result
            return perform(run)
        def stat(self, path): return perform(lambda: paramiko.SFTPAttributes.from_stat(bound.path(path).stat()))
        lstat = stat
        def open(self, path, flags, attr):
            def run():
                if not flags & os.O_CREAT or not flags & os.O_EXCL or not flags & (os.O_WRONLY | os.O_RDWR): raise PermissionError('Expected exclusive staging create')
                f=bound.create(path);h=Handle(flags);h.writefile=f;h.logical=path;audit('create', path=path);return h
            return perform(run)
        def mkdir(self, path, attr):
            def run(): bound.mkdir(path);audit('mkdir', path=path);return paramiko.SFTP_OK
            return perform(run)
        def rename(self, oldpath, newpath):
            def run(): bound.rename(oldpath,newpath);audit('rename', source=oldpath,destination=newpath);return paramiko.SFTP_OK
            return perform(run)
        def remove(self, path):
            def run(): bound.remove(path);audit('remove-stage', path=path);return paramiko.SFTP_OK
            return perform(run)
        def chattr(self, path, attr):
            def run():
                p=bound.path(path)
                if p != root / FOLDER: bound.staging(path)
                if any(getattr(attr,k,None) is not None for k in ('st_size','st_uid','st_gid')): raise PermissionError('Unexpected metadata mutation')
                atime,mtime=getattr(attr,'st_atime',None),getattr(attr,'st_mtime',None)
                if atime is not None and mtime is not None: os.utime(p,(atime,mtime))
                audit('metadata',path=path);return paramiko.SFTP_OK
            return perform(run)
    sock=socket.socket();sock.bind(('127.0.0.1',0));sock.listen(8);sock.settimeout(1)
    manifest=dict(pid=os.getpid(),workspace=str(workspace),host='127.0.0.1',port=sock.getsockname()[1],user=user,password=password,host_key_type=key.get_name(),host_key_base64=key.get_base64(),paramiko_version=paramiko.__version__)
    receipt=workspace/'connection.json';receipt.write_text(json.dumps(manifest),encoding='utf8');os.chmod(receipt,0o600);print(str(receipt),flush=True)
    def connection(client):
        t=paramiko.Transport(client)
        with lock: transports.append(t)
        try:
            t.add_server_key(key);t.set_subsystem_handler('sftp',paramiko.SFTPServer,Files);t.start_server(server=Auth())
            while t.is_active() and not stop.wait(.2): pass
        except Exception as e: audit('connection-error',kind=type(e).__name__)
        finally: t.close()
    for sig in (signal.SIGINT,signal.SIGTERM): signal.signal(sig,lambda *_:stop.set())
    try:
        while not stop.is_set():
            try: client,_=sock.accept()
            except socket.timeout: continue
            threading.Thread(target=connection,args=(client,),daemon=True).start()
    finally:
        stop.set();sock.close()
        for t in transports: t.close()
        audit('stopped');receipt.unlink(missing_ok=True)
if __name__ == '__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('--workspace',required=True);serve(parser.parse_args().workspace)
