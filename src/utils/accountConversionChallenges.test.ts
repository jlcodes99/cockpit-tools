import assert from 'node:assert/strict';
import test from 'node:test';
import {
  challengeExpiresInMinutes,
  challengeStatusLabel,
  canConfirmConversionChallenge,
  conversionChallengeNeedsMfaSelection,
  emailsFromMfaLabel,
  matchingMfaRecords,
} from './accountConversionChallenges';
import type { MfaRecord } from './mfaVault';

const records: MfaRecord[] = [
  {
    id: 'one',
    accountName: 'Google: Person.One@gmail.com',
    secret: 'JBSWY3DPEHPK3PXP',
    remark: '',
    time: 1,
  },
  {
    id: 'two',
    accountName: 'other@gmail.com',
    secret: 'JBSWY3DPEHPK3PXQ',
    remark: 'backup for person.one@gmail.com',
    time: 2,
  },
];

test('extracts normalized complete emails from a non-secret MFA label', () => {
  assert.deepEqual(emailsFromMfaLabel('Google: Person.One@GMAIL.com'), [
    'person.one@gmail.com',
  ]);
});

test('localizes challenge state and reports a bounded expiry countdown', () => {
  assert.equal(challengeStatusLabel('queued'), '等待处理');
  assert.equal(challengeStatusLabel('user_confirmed'), '用户已确认');
  assert.equal(
    challengeExpiresInMinutes(
      { expiresAt: new Date(120_000).toISOString() },
      0,
    ),
    2,
  );
  assert.equal(
    challengeExpiresInMinutes({ expiresAt: new Date(0).toISOString() }, 1),
    0,
  );
});

test('matches a full email and reports multiple records without guessing', () => {
  assert.deepEqual(
    matchingMfaRecords(records, 'PERSON.ONE@gmail.com').map((record) => record.id),
    ['one', 'two'],
  );
  assert.deepEqual(matchingMfaRecords(records, 'missing@gmail.com'), []);
});

test('requires an explicit full-email MFA selection before confirming MFA challenges', () => {
  assert.equal(conversionChallengeNeedsMfaSelection('totp'), true);
  assert.equal(conversionChallengeNeedsMfaSelection('authenticator_setup'), true);
  assert.equal(conversionChallengeNeedsMfaSelection('password_current'), false);
  assert.equal(canConfirmConversionChallenge('totp', ''), false);
  assert.equal(canConfirmConversionChallenge('authenticator_setup', ''), false);
  assert.equal(canConfirmConversionChallenge('totp', 'selected-record'), true);
  assert.equal(canConfirmConversionChallenge('password_current', ''), true);
});
