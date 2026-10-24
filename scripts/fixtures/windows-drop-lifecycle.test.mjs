import test from 'node:test';
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import fs from 'node:fs';
import {observeExit,explorerCleanupComplete} from './windows-drop-lifecycle.mjs';

const childStub = () => Object.assign(new EventEmitter().on('error',()=>{}), {exitCode:null, signalCode:null});

test('live child remains unconfirmed until an exit is observed', () => {
  const child=childStub(), state=observeExit(child);
  assert.equal(state.stopped,false);
  child.emit('error',new Error('kill was refused')); // An error is not an exit.
  assert.equal(state.stopped,false);
});

test('Windows signal-only termination is stopped and records actual exit event', () => {
  const child=childStub(), state=observeExit(child);
  child.signalCode='SIGTERM';
  child.emit('exit',null,'SIGTERM');
  assert.equal(child.exitCode,null);
  assert.equal(state.stopped,true);
  assert.deepEqual(state.result,{code:null,signal:'SIGTERM',exit_event_observed:true});
});

test('normal and early process exits are retained without waiting again', () => {
  const child=childStub(), state=observeExit(child);
  child.exitCode=0; child.emit('exit',0,null);
  assert.equal(state.stopped,true);
  assert.deepEqual(state.result,{code:0,signal:null,exit_event_observed:true});
});

test('exit properties can confirm stop before the event, without inventing an event', () => {
  const child=childStub(), state=observeExit(child);
  child.signalCode='SIGTERM';
  assert.equal(state.stopped,true);
  assert.deepEqual(state.result,{code:null,signal:'SIGTERM',exit_event_observed:false});
});

test('Explorer absence is valid after early failure without a close request', () => {
  assert.equal(explorerCleanupComplete({explorer_creation_attempted:false,explorer_state:'not-created',explorer_close_requested:false,no_owned_window_confirmed:true}),true);
  assert.equal(explorerCleanupComplete({explorer_creation_attempted:true,explorer_state:'not-present',explorer_close_requested:false,no_owned_window_confirmed:true}),true);
  assert.equal(explorerCleanupComplete({explorer_creation_attempted:true,explorer_state:'closed',explorer_close_requested:true,no_owned_window_confirmed:true}),true);
});

test('Explorer cleanup fails closed for missing, contradictory or unconfirmed receipts', () => {
  for(const receipt of [{},{explorer_close_requested:true},{explorer_state:'not-created',explorer_creation_attempted:true,no_owned_window_confirmed:true,explorer_close_requested:false},{explorer_state:'closed',explorer_creation_attempted:true,no_owned_window_confirmed:false,explorer_close_requested:true},{explorer_state:'not-created',explorer_creation_attempted:false,no_owned_window_confirmed:true,explorer_close_requested:false,error:'Enumeration failed'}])assert.equal(explorerCleanupComplete(receipt),false);
});

// Source-only checks are intentional: this suite must not load user32, dispatch
// input, create windows, or import the executing CI entry point on the host.
test('ownership checks target pinned home HWND and retain process/session guards', () => {
  const controller=fs.readFileSync(new URL('../windows-native-drop-control.ps1',import.meta.url),'utf8');
  const own=controller.match(/function Own-App \{([\s\S]*?)\n\}/)[1];
  assert.match(own,/-Operation inspect/); // Includes existing canonical-path validation.
  assert.match(own,/\$state\.creation_ticks -ne \$CreationTicks/);
  assert.match(own,/\$state\.pid -ne \$AppPid/);
  assert.match(own,/\$state\.session_id -ne \(Get-Process -Id \$PID\)\.SessionId/);
  assert.match(own,/AssertHomeWindow\(\$AppPid,\$MainHwnd\)/);
  assert.doesNotMatch(own,/\$state\.hwnd|MainWindowHandle/);
  const driver=fs.readFileSync(new URL('./windows-drop-driver.cs',import.meta.url),'utf8');
  const query=driver.match(/public static void AssertHomeWindow[^\{]*\{([\s\S]*?)\n \}/)[1];
  for(const guard of ['IsWindow(h)','GetWindowThreadProcessId(h,out actual)','actual!=pid','itda_home','IsWindowVisible(h)','GetWindowTextW(h,title,title.Capacity)'])assert.ok(query.includes(guard),guard);
  assert.doesNotMatch(query,/SendInput|SetWindowPos|SetForegroundWindow|PostMessage/);
});

test('cleanup never marks a live/error-only controller quiescent or turns its failure into pass', () => {
  const entry=fs.readFileSync(new URL('../windows-native-drop-ci-e2e.mjs',import.meta.url),'utf8');
  assert.match(entry,/assert\(controllerState\.stopped,'[^']+'\);controllerQuiesced=true/);
  assert.match(entry,/if\(result\.code!==0\|\|result\.signal\|\|result\.error\)\{report\.status='failed';report\.error\?\?='Controller exited abnormally';process\.exitCode=1;/);
  assert.match(entry,/if\(!controllerQuiesced\)errors\.push/);
});
