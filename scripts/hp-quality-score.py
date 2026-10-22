# Decode the last accepted frame and compare RGB PSNR/SSIM with ffmpeg.
# Usage: python3 scripts/hp-quality-score.py /path/to/benchmark-results
# Directory names: <device>-<scale>-<rate>; native baseline must end in -2-6.
import json,pathlib,subprocess,re,sys
root=pathlib.Path(sys.argv[1]).resolve(); rows=[]
def run(args):
 p=subprocess.run(['ffmpeg','-hide_banner','-loglevel','info',*args],capture_output=True,text=True)
 if p.returncode:raise RuntimeError(p.stderr[-2000:])
 return p.stderr
for d in sorted(root.iterdir()):
 if not d.is_dir() or not (d/'result.json').exists(): continue
 m=json.loads((d/'result.json').read_text()); w,h=m['width'],m['height'];stride=m['reference_stride']; raw=(d/'reference.bgra').read_bytes();(d/'packed.bgra').write_bytes(b''.join(raw[y*stride:y*stride+w*4] for y in range(h)))
 run(['-y','-f','rawvideo','-pixel_format','bgra','-video_size',f'{w}x{h}','-i',str(d/'packed.bgra'),'-frames:v','1',str(d/'reference.png')])
 run(['-y','-i',str(d/'encoded.h264'),'-vf',f"select=eq(n\\,{m['frames']-1})",'-frames:v','1','-fps_mode','vfr',str(d/'decoded.png')])
 for metric in ['psnr','ssim']:
  log=run(['-i',str(d/'reference.png'),'-i',str(d/'decoded.png'),'-lavfi',f'[0:v]format=gbrp[a];[1:v]format=gbrp[b];[a][b]{metric}', '-f','null','-'])
  key='average:' if metric=='psnr' else 'All:'
  m[metric]=float(re.findall(key+r'([\d.]+)',log)[-1])
 m['fixture']=d.name; rows.append(m)
# Compare every mode at the same native-pixel size: bilinear point-mode upscale
# models display scaling. Crop excludes most whitespace around the text.
for m in rows:
 d=root/m['fixture'];native=root/(m['fixture'].rsplit('-',2)[0]+'-2-6');s=m['scale'];w=m['width']*2//s;h=m['height']*2//s
 for metric in ['psnr','ssim']:
  log=run(['-i',str(native/'reference.png'),'-i',str(d/'decoded.png'),'-lavfi',f'[0:v]crop=1800:800:0:0,format=gbrp[a];[1:v]scale={w}:{h}:flags=bilinear,crop=1800:800:0:0,format=gbrp[b];[a][b]{metric}', '-f','null','-'])
  m['native_text_'+metric]=float(re.findall(('average:' if metric=='psnr' else 'All:')+r'([\d.]+)',log)[-1])
(root/'scores.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows,indent=2))
