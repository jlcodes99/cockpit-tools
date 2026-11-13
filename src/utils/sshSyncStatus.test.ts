import assert from 'node:assert/strict';
import test from 'node:test';
import { sshSyncSucceeded } from './sshSyncStatus';

test('verified remote files with a connection refresh error remain a warning',()=>{
  assert.equal(sshSyncSucceeded({verified:true,error:null}),true);
  assert.equal(sshSyncSucceeded({verified:true,error:'ssh_connection_refresh_failed: account_mismatch'}),false);
  assert.equal(sshSyncSucceeded({verified:false,error:'ssh_remote_write_failed'}),false);
});
