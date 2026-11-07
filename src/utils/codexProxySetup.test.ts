import assert from 'node:assert/strict';
import test from 'node:test';
import type { EngineInstallStatus } from '../services/codexProxyEngineService';
import { proxyEngineReadiness, proxySetupStage } from './codexProxySetup';

const missing: EngineInstallStatus = {
  supported: true, version: '1.19.31', installedVersion: null, assetName: 'archive.gz', archiveBytes: 100,
  jobId: null, phase: 'idle', receivedBytes: 0, totalBytes: null, error: null,
};
const installed = { ...missing, installedVersion: '1.19.31', phase: 'completed' as const };

test('first-use guidance never confuses unknown status or an unreadable catalog with empty configuration', () => {
  assert.equal(proxyEngineReadiness(null, false), 'loading');
  assert.equal(proxyEngineReadiness(null, true), 'unknown');
  assert.equal(proxyEngineReadiness(installed, true), 'unknown');
  assert.equal(proxySetupStage('ready', null), 'pending');
  assert.equal(proxySetupStage('unknown', true), 'prepare');
});

test('proxy configuration can exist before installation and does not imply engine readiness', () => {
  assert.equal(proxyEngineReadiness(missing, false), 'missing');
  assert.equal(proxySetupStage('missing', true), 'prepare');
  assert.equal(proxySetupStage('missing', false), 'prepare');
  assert.equal(proxyEngineReadiness({ ...installed, installedVersion: 'old-version' }, false), 'missing');
});

test('active jobs and failed repairs keep progress or errors visible even with a previous installation', () => {
  for (const phase of ['downloading', 'importing', 'verifying', 'extracting', 'checking', 'installing'] as const) {
    assert.equal(proxyEngineReadiness({ ...installed, phase }, false), 'installing');
    assert.equal(proxySetupStage('installing', true), 'prepare');
  }
  assert.equal(proxyEngineReadiness({ ...installed, phase: 'failed', error: 'ENGINE_INSTALL_VERIFY' }, false), 'failed');
  assert.equal(proxyEngineReadiness({ ...missing, phase: 'cancelled' }, false), 'missing');
});

test('only supported, completed setup suggests assignment; an empty or unsupported catalog still needs configuration', () => {
  assert.equal(proxyEngineReadiness({ ...installed, supported: false }, false), 'unsupported');
  assert.equal(proxyEngineReadiness(installed, false), 'ready');
  assert.equal(proxySetupStage('ready', false), 'add');
  assert.equal(proxySetupStage('ready', true), 'assign');
  assert.equal(proxySetupStage('unsupported', true), 'prepare');
});
