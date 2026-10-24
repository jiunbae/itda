import test from 'node:test';
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
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
