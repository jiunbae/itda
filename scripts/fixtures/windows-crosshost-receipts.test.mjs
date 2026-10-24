// Pure state/receipt regression: importing the harness starts no server, app or input.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mayTeardownResources,sanitizeWindowObservation,sanitizeTreeObservation,safeFixturePath} from '../windows-native-crosshost-ci-e2e.mjs';
const sha=s=>createHash('sha256').update(s).digest('hex');
const complete={controllerQuiescent:true,appStopped:true,ownedAppProcessCount:0,observationsPersisted:true};
test('controller completion alone cannot authorize dependency/resource teardown',()=>{
 assert.equal(mayTeardownResources(complete),true);
 for(const change of [
  {appStopped:false},{appStopped:undefined}, // close timeout or exit observation lost
  {ownedAppProcessCount:1},{ownedAppProcessCount:null},{ownedAppProcessCount:'0'}, // live WebView or failed/ambiguous probe
  {controllerQuiescent:false},{controllerQuiescent:undefined},
  {observationsPersisted:false},{observationsPersisted:undefined},
 ])assert.equal(mayTeardownResources({...complete,...change}),false,JSON.stringify(change));
});
test('failure receipt retains useful route/job/prompt facts but hashes every opaque identity',()=>{
 const lease='a'.repeat(32),gesture='b'.repeat(32),page='c'.repeat(32),window='d'.repeat(32),password='super-secret-password';
 const raw={identity:{host:'crosshost-source',window,page,title:'QA source — 파일',password},qa:{connected:true,remote_path:'/',password,
  begins:[{id:'crosshost-source',window,source_window:window,gesture,sequence:7,side:'remote',path:'/',items:[{name:'payload.bin',dir:false}],password}],
  ends:[{id:'crosshost-source',window,gesture,dropped:false,error:password,trace:{phase:'failed',entered_ole:true,query_count:4,password}}],
  drops:[{window,origin:{id:'crosshost-source',window,drag:lease,side:'remote',path:'/',names:['payload.bin'],password},paths:['C:/private/'+password]}],
  transfers:[{id:'crosshost-destination',window,from_id:'crosshost-source',from_window:page,from_drag:lease,req:gesture,from:'remote',to:'remote',source:'/',destination:'/',move:false,policy:'keep',engine:'auto',names:['payload.bin'],password}],
  jobs:[{id:'crosshost-destination',window,job:lease,req:gesture,state:'failed',error:password,completed_names:['payload.bin'],password}],releases:[{drag:lease,password}]},
  dialogs:[{text:password,password}],prompts:[{title:'파일 복사',text:'3개 항목 '+password,policy:'keep',buttons:['복사','취소',password],password}],config:{password}};
 const clean=sanitizeWindowObservation(raw),encoded=JSON.stringify(clean);
 for(const value of [lease,gesture,page,window,password,'C:/private/'])assert.equal(encoded.includes(value),false,'Raw private field escaped allowlist');
 assert.equal(clean.begins.count,1);assert.equal(clean.begins.last[0].sequence,7);assert.equal(clean.ends.last[0].trace.entered_ole,true);
 assert.equal(clean.drops.last[0].origin.drag_sha256,sha(lease));assert.equal(clean.begins.last[0].gesture_sha256,sha(gesture));
 assert.equal(clean.transfers.last[0].from_drag_sha256,sha(lease));assert.equal(clean.transfers.last[0].to,'remote');assert.equal(clean.jobs.last[0].state,'failed');
 assert.equal(clean.prompt_count,1);assert.equal(clean.prompts[0].title,'파일 복사');assert.equal(clean.dialog_count,1);
});
test('unknown names, paths and stage nonces remain diagnosable without publishing them',()=>{
 const token='e'.repeat(32),stage='/.itda-'+token+'.partial',privatePath='C:/Users/private-name/secret.txt';
 const entries=sanitizeTreeObservation([{path:'payload.bin',dir:false,sha256:sha('data'),password:'never'}, {path:stage,dir:false,sha256:sha('partial')},{path:privatePath,link:true,secret:'never'}]);
 assert.equal(entries[0].path.value,'payload.bin');assert.equal(entries[0].sha256,sha('data'));
 assert.equal(entries[1].path.kind,'stage');assert.equal(entries[1].path.sha256,sha(stage));assert.equal(entries[2].path.kind,'unexpected');assert.equal(entries[2].link,true);
 const text=JSON.stringify(entries);for(const value of [token,privatePath,'private-name','never'])assert.equal(text.includes(value),false);
 assert.equal(safeFixturePath('테스트 폴더/메모.txt'.normalize('NFD')).kind,'fixture');
 assert.equal(safeFixturePath('/테스트 폴더/.itda-'+token+'.partial').kind,'stage');
});
test('unavailable final windows and empty fixtures are recorded explicitly',()=>{
 const clean=sanitizeWindowObservation({closed:true,unavailable:true});assert.equal(clean.closed,true);assert.equal(clean.unavailable,true);assert.equal(clean.prompt_count,null);
 assert.deepEqual(sanitizeTreeObservation([]),[]);
});
