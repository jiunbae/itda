"""Local generated-temp-file tests only; no server, sockets, app or native input."""
import importlib.util, pathlib, tempfile, unittest
spec=importlib.util.spec_from_file_location('fixture',pathlib.Path(__file__).with_name('confined-sftp-crosshost.py'))
f=importlib.util.module_from_spec(spec);spec.loader.exec_module(f)
class Confinement(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='itda-quality-confinement-');self.root=pathlib.Path(self.tmp.name).resolve();self.c=f.Confined(self.root)
    def tearDown(self): self.tmp.cleanup()
    def test_traversal_and_windows_syntax(self):
        for value in ['../outside','/../outside','/x/../../outside','/a/./b','/a//b','//server/share','C:/file','/file:stream','\\\\server\\share','/a\\b','/a\0b','/name.','/name ']:
            with self.subTest(value=value),self.assertRaises(PermissionError): self.c.path(value)
        self.assertEqual(self.c.path('.'),self.root);self.assertEqual(self.c.path('/'),self.root)
    def test_staging_copy_and_no_overwrite(self):
        self.c.mkdir('/'+f.FOLDER)
        for i, name in enumerate(sorted(f.FILES)):
            parent=pathlib.PurePosixPath(name).parent.as_posix();parent='' if parent=='.' else parent+'/'
            stage='/'+parent+'.itda-'+format(i+1,'032x')+'.partial'
            with self.c.create(stage) as out: out.write(b'generated fixture')
            with self.assertRaises(FileExistsError): self.c.create(stage)
            self.c.rename(stage,'/'+name)
            self.assertEqual(self.c.path('/'+name).read_bytes(),b'generated fixture')
            if i == 0:
                with self.c.create(stage) as out: out.write(b'not replacement')
                with self.assertRaises(PermissionError): self.c.rename(stage,'/'+name)
                self.c.remove(stage)
        self.assertEqual(len(list(self.root.rglob('*.partial'))),0)
        with self.assertRaises(PermissionError): self.c.create('/.itda-'+'f'*32+'.partial')
    def test_write_byte_bounds(self):
        self.c.check_write(0,f.MAX_FILE);self.c.check_write(f.MAX_FILE-1,1)
        for offset,length in [(-1,1),(0,-1),(f.MAX_FILE,1),(1,f.MAX_FILE)]:
            with self.assertRaises(PermissionError): self.c.check_write(offset,length)
    def test_reparse_attribute_rejected(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        import stat
        with patch.object(pathlib.Path,'lstat',return_value=SimpleNamespace(st_mode=stat.S_IFDIR,st_file_attributes=0x400)):
            with self.assertRaises(PermissionError): self.c.path('/payload.bin')
    def test_unknown_or_final_writes_rejected(self):
        for value in ['/payload.bin','/unknown','/other/.itda-'+ 'a'*32+'.partial']:
            with self.assertRaises(PermissionError): self.c.create(value)
        with self.assertRaises(PermissionError): self.c.mkdir('/other')
        with self.assertRaises(PermissionError): self.c.remove('/payload.bin')
    def test_link_component_and_link_root_rejected(self):
        # Both roots are generated owned temp folders; no host/user file is read.
        with tempfile.TemporaryDirectory(prefix='itda-quality-outside-') as other:
            outside=pathlib.Path(other).resolve();link=self.root/'link'
            try: link.symlink_to(outside,target_is_directory=True)
            except (OSError,NotImplementedError): self.skipTest('Unprivileged symlink unavailable; Windows reparse check remains source-reviewed')
            with self.assertRaises(PermissionError): self.c.path('/link/file')
            with self.assertRaises(PermissionError): f.Confined(link)
            stage=self.root/('.itda-'+'b'*32+'.partial');stage.symlink_to(outside/'new')
            with self.assertRaises(PermissionError): self.c.create('/'+stage.name)
            self.assertEqual(list(outside.iterdir()),[])
if __name__=='__main__': unittest.main()
