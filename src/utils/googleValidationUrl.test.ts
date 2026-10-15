import assert from 'node:assert/strict';
import test from 'node:test';
import { formatGoogleValidationUrl } from './googleValidationUrl';

test('formatGoogleValidationUrl injects authuser, login_hint, and Email when authuser is empty', () => {
  const raw =
    'https://accounts.google.com/signin/continue?sarp=1&scc=1&continue=https%3A%2F%2Fdevelopers.google.com%2Fgemini-code-assist%2Fauth%2Fauth_success_gemini&plt=AKgnsbtp&flowName=GlifWebSignIn&authuser';
  const email = 'target@gmail.com';
  const result = formatGoogleValidationUrl(raw, email);

  const u = new URL(result);
  assert.equal(u.searchParams.get('authuser'), 'target@gmail.com');
  assert.equal(u.searchParams.get('login_hint'), 'target@gmail.com');
  assert.equal(u.searchParams.get('Email'), 'target@gmail.com');
  assert.equal(u.searchParams.get('plt'), 'AKgnsbtp');
  assert.equal(
    u.searchParams.get('continue'),
    'https://developers.google.com/gemini-code-assist/auth/auth_success_gemini'
  );
});

test('formatGoogleValidationUrl replaces numeric authuser index with email', () => {
  const raw =
    'https://accounts.google.com/signin/continue?authuser=0&continue=https%3A%2F%2Fdevelopers.google.com';
  const result = formatGoogleValidationUrl(raw, 'target@gmail.com');

  const u = new URL(result);
  assert.equal(u.searchParams.get('authuser'), 'target@gmail.com');
  assert.equal(u.searchParams.get('login_hint'), 'target@gmail.com');
  assert.equal(u.searchParams.get('Email'), 'target@gmail.com');
});

test('formatGoogleValidationUrl does not overwrite existing non-empty non-numeric authuser', () => {
  const raw =
    'https://accounts.google.com/signin/continue?authuser=existing%40gmail.com&continue=https%3A%2F%2Fdevelopers.google.com';
  const result = formatGoogleValidationUrl(raw, 'target@gmail.com');

  const u = new URL(result);
  assert.equal(u.searchParams.get('authuser'), 'existing@gmail.com');
  assert.equal(u.searchParams.get('login_hint'), 'target@gmail.com');
});

test('formatGoogleValidationUrl returns original url if email is empty or null', () => {
  const raw = 'https://accounts.google.com/signin/continue?authuser';
  assert.equal(formatGoogleValidationUrl(raw, ''), raw);
  assert.equal(formatGoogleValidationUrl(raw, null), raw);
  assert.equal(formatGoogleValidationUrl(raw, undefined), raw);
});

test('formatGoogleValidationUrl does not modify non-google urls', () => {
  const raw = 'https://example.com/verify?code=123';
  assert.equal(formatGoogleValidationUrl(raw, 'target@gmail.com'), raw);

  const fakeHost = 'https://evil-google.com/signin?authuser';
  assert.equal(formatGoogleValidationUrl(fakeHost, 'target@gmail.com'), fakeHost);

  const querySubstr = 'https://attacker.com/login?redirect=accounts.google.com&authuser';
  assert.equal(formatGoogleValidationUrl(querySubstr, 'target@gmail.com'), querySubstr);
});
