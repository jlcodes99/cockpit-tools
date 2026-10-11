// Read the installed application archive without executing vendor code or
// reading any user profile. Output only source hashes and structural evidence.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import ts from 'typescript';

const archive = process.argv[2] || '/Applications/Claude.app/Contents/Resources/app.asar';
const bytes = fs.readFileSync(archive);
const headerSize = bytes.readUInt32LE(4);
const jsonSize = bytes.readUInt32LE(12);
if (jsonSize > 16 * 1024 * 1024 || headerSize + 8 > bytes.length) throw Error('INVALID_ARCHIVE');
const header = JSON.parse(bytes.subarray(16, 16 + jsonSize).toString('utf8'));
const members = [];
function walk(node, prefix = '') {
  for (const [name, item] of Object.entries(node.files || {})) {
    const member = prefix ? `${prefix}/${name}` : name;
    if (item.files) walk(item, member);
    else if (!item.unpacked && item.offset !== undefined) {
      const start = 8 + headerSize + Number(item.offset);
      if (!Number.isSafeInteger(start) || start < 8 + headerSize || start + item.size > bytes.length) throw Error('INVALID_MEMBER');
      members.push({ member, source: bytes.subarray(start, start + item.size).toString('utf8') });
    }
  }
}
walk(header);
const pkg = JSON.parse(members.find(item => item.member === 'package.json').source);
const main = members.find(item => item.member.startsWith('.vite/build/') && item.source.includes('async loadSessionRecords()'));
if (!main) throw Error('SESSION_LOADER_NOT_FOUND');
const parsed = ts.createSourceFile(main.member, main.source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
const methods = new Map();
function visit(node) {
  if (ts.isMethodDeclaration(node)) methods.set(node.name.getText(parsed), node.getText(parsed));
  ts.forEachChild(node, visit);
}
visit(parsed);
const normalizerName = main.source.match(/const [^;]+?\bn=require\("\.\/(index\.chunk-[^"]+)"\)/)?.[1];
const normalizer = members.find(item => item.member === `.vite/build/${normalizerName}`);
if (!normalizer) throw Error('RECORD_PROJECTION_NOT_FOUND');
const normalizerParsed = ts.createSourceFile(normalizer.member, normalizer.source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
const projectionCandidates=[];
function findProjection(node) {
  if(ts.isReturnStatement(node)&&node.expression&&ts.isObjectLiteralExpression(node.expression)) {
    const properties=node.expression.properties.filter(ts.isPropertyAssignment);
    const names=properties.map(p=>p.name.getText(normalizerParsed).replace(/^["']|["']$/g,''));
    if(names.length>=50&&names.includes('sessionId')&&names.includes('cliSessionId')) {
      let owner=node.parent;while(owner&&!ts.isFunctionDeclaration(owner))owner=owner.parent;
      if(owner)projectionCandidates.push({owner,returned:node.expression,names});
    }
  }
  ts.forEachChild(node, findProjection);
}
findProjection(normalizerParsed);
if(projectionCandidates.length!==1)throw Error('RECORD_PROJECTION_NOT_UNIQUE');
const {owner:projection,names:projectionNames}=projectionCandidates[0];
const fields=new Set(projectionNames);
const loader = methods.get('loadSessionRecords') || '';
const filePath = methods.get('getSessionFilePath') || '';
const checks = {
  packageVersionPresent: typeof pkg.version === 'string',
  syntaxDiagnostics: parsed.parseDiagnostics.length + normalizerParsed.parseDiagnostics.length,
  namespaceIncludesAccountAndOrg: /currentAccountId/.test(methods.get('getStorageDir') || '') && /currentOrgId/.test(methods.get('getStorageDir') || ''),
  loadsLocalJsonRows: /startsWith\("local_"\)/.test(loader) && /endsWith\("\.json"\)/.test(loader),
  mapKeyIsPersistedSessionId: /this\.sessions\.set\(\w+\.sessionId,/.test(loader),
  filePathUsesDesktopSessionId: /\$\{e\}\.json/.test(filePath),
  separateDesktopAndTranscriptIdentity: fields.has('sessionId') && fields.has('cliSessionId'),
  nativeForkCopiesTranscript: /copyFile/.test(methods.get('forkSession') || ''),
  nativeForkChecksAccountIdentity: /currentAccountId/.test(methods.get('registerForkedSession') || '') && /currentOrgId/.test(methods.get('registerForkedSession') || ''),
};
const sha = input => createHash('sha256').update(input).digest('hex');
const result = { schema: 1, observedAt: new Date().toISOString(), version: pkg.version,
  archiveSha256: sha(bytes), mainMember: main.member, mainSha256: sha(main.source),
  projectionMember: normalizer.member, projectionSha256: sha(normalizer.source), persistedFieldCount: fields.size,
  persistedFields:[...fields].sort(),projectionFunctionSha256:sha(projection.getText(normalizerParsed)),
  checks, verdict: checks.syntaxDiagnostics === 0 && Object.entries(checks).every(([k,v]) => k === 'syntaxDiagnostics' || v)
    ? 'static_local_record_contract_pass_desktop_pending' : 'fail',
  limitation: 'Structural inspection only. No user data read, app startup, account switch, alias opening, transcript lease or continuation test.' };
console.log(JSON.stringify(result, null, 2));
if (result.verdict === 'fail') process.exitCode = 1;
