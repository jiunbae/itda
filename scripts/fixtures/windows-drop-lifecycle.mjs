// Pure lifecycle/receipt checks; importing this helper launches nothing.
export function observeExit(child) {
  let result;
  child.once('exit', (code, signal) => { result = {code, signal}; });
  return {
    get stopped() { return result !== undefined || child.exitCode !== null || child.signalCode !== null; },
    get result() { return {...(result ?? {code: child.exitCode, signal: child.signalCode}), exit_event_observed: result !== undefined}; },
  };
}

export function explorerCleanupComplete(receipt) {
  if (receipt.error || receipt.no_owned_window_confirmed !== true) return false;
  if (receipt.explorer_state === 'closed') return receipt.explorer_creation_attempted === true && receipt.explorer_close_requested === true;
  if (receipt.explorer_state === 'not-created') return receipt.explorer_creation_attempted === false && receipt.explorer_close_requested === false;
  return receipt.explorer_state === 'not-present' && receipt.explorer_creation_attempted === true && receipt.explorer_close_requested === false;
}
