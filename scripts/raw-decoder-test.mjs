// Fragmented socket input, pixel conversion and batched drawing against the real decoder.
import fs from 'node:fs';
import assert from 'node:assert/strict';
const {default:Raw}=await import('data:text/javascript;base64,'+fs.readFileSync(new URL('../web/viewer/novnc-src/raw.js',import.meta.url)).toString('base64'));
for(const depth of [8,24])for(const fragmented of [false,true]){
    const w=3,h=259,size=depth===8?1:4;
    const data=Uint8Array.from({length:w*h*size},(_,i)=>i%256);
    let available=fragmented?0:data.length,pos=0,calls=0;
    const out=new Uint8Array(w*h*4),decoder=new Raw();
    const sock={rQwait:(_name,n)=>available-pos<n,rQlen:()=>available-pos,
        rQshiftBytes:n=>{const a=data.subarray(pos,pos+n);pos+=n;return a;}};
    const display={blitImage:(x,y,width,height,arr)=>{
        calls++;out.set(arr.subarray(0,width*height*4),y*w*4);
    }};
    for(;available<data.length;available=Math.min(data.length,available+17))
        assert.equal(decoder.decodeRect(0,0,w,h,sock,display,depth),false);
    assert.equal(decoder.decodeRect(0,0,w,h,sock,display,depth),true);
    assert.equal(pos,data.length);
    for(let i=0;i<w*h;i++){
        assert.equal(out[i*4+3],255);
        const rgb=depth===24?[...data.slice(i*4,i*4+3)]:
            [((data[i]>>0)&3)*255/3,((data[i]>>2)&3)*255/3,((data[i]>>4)&3)*255/3];
        assert.deepEqual([...out.slice(i*4,i*4+3)],rgb);
    }
    if(!fragmented)assert.equal(calls,3,'259 rows should require three drawing calls');
    console.log('PASS Raw',depth,'bit,',fragmented?'fragmented':'complete',calls,'draw calls');
}
