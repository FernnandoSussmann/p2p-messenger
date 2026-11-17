#!/usr/bin/env node
// npm run release: builds the end-user binary. Node builtins only.
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { constants as fsc } from 'node:fs';
import {
  mkdtemp, realpath, lstat, rm, readFile, readdir, copyFile, rename, chmod, mkdir, stat, writeFile,
} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = await realpath(path.join(path.dirname(fileURLToPath(import.meta.url)), '..'));
const appDir = path.join(repo, 'app');
const tauriDir = path.join(appDir, 'src-tauri');
const releaseDir = path.join(repo, 'release');
const isWin = process.platform === 'win32';
let tempTarget = null; // set only from mkdtemp in this run
const partials = []; // the only files this run may delete in release/
let child = null;
let interrupted = false;

for (const sig of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
  process.on(sig, () => {
    interrupted = true;
    child?.kill(sig);
  });
}

const checkInterrupted = () => {
  if (interrupted) throw new Error('interrupted');
};

function run(cmd, args, { cwd = repo, env = process.env } = {}) {
  checkInterrupted();
  return new Promise((resolve, reject) => {
    child = spawn(cmd, args, { cwd, env, shell: false, stdio: 'inherit' });
    child.on('error', reject);
    child.on('close', (code, signal) => {
      child = null;
      if (interrupted) reject(new Error('interrupted'));
      else if (code === 0) resolve();
      else reject(new Error(`${path.basename(cmd)} ${args[0] ?? ''} failed (${signal ?? code})`));
    });
  });
}

function probe(cmd, args) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', shell: false, maxBuffer: 64 * 1024 * 1024 });
  return r.error || r.status !== 0 ? null : r.stdout;
}

const npm = (args, opts) => run(process.execPath, [process.env.npm_execpath, ...args], opts);
const tauri = (args, opts) =>
  run(process.execPath, [path.join(appDir, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), ...args], opts);

// ---- pure checks (no I/O), reusable on bad inputs ----

export const FORBIDDEN_NAMES = [
  'FileKeychain', 'MemoryContactStore', 'FailingContactStore', 'MemoryMessageStore',
  'FailingMessageStore', 'ChatPeer', 'RecordingHistoryEraser', 'messenger_headless', 'messenger-headless',
];

export function findForbidden(buf, needles) {
  return needles.filter((n) => buf.indexOf(n) !== -1);
}

export function builderPathNeedles(paths) {
  const out = new Set();
  for (const p of paths) {
    if (!p || p === '/' || p === path.parse(p).root) continue;
    out.add(p);
    if (isWin) out.add(p.replaceAll('\\', '/'));
  }
  return [...out];
}

export function exportedSqlite(nmOutput) {
  return nmOutput.split('\n').filter((l) => /\ssqlite3_/.test(l));
}

export function hasDebugSections(names) {
  return names.some((n) => n === '.symtab' || n.startsWith('.debug_') || n.startsWith('.debug'));
}

export function peSectionNames(buf) {
  if (buf.length < 0x40 || buf.readUInt16LE(0) !== 0x5a4d) throw new Error('not a PE file');
  const pe = buf.readUInt32LE(0x3c);
  if (buf.toString('latin1', pe, pe + 4) !== 'PE\0\0') throw new Error('bad PE signature');
  const count = buf.readUInt16LE(pe + 6);
  const optSize = buf.readUInt16LE(pe + 20);
  let off = pe + 24 + optSize;
  const names = [];
  for (let i = 0; i < count; i++, off += 40) {
    names.push(buf.toString('latin1', off, off + 8).replace(/\0.*$/, ''));
  }
  return names;
}

export function devtoolsInManifests(cargoToml, tauriCargoToml) {
  const hits = [];
  for (const line of cargoToml.split('\n')) if (/^\s*tauri\s*=/.test(line) && line.includes('devtools')) hits.push(line.trim());
  for (const line of tauriCargoToml.split('\n')) if (line.includes('devtools')) hits.push(line.trim());
  return hits;
}

export function bakedEnvHits(source) {
  return source.split('\n').filter((l) => /\b(option_env|env)!\s*\(\s*"(MESSENGER_PROFILE|MESSENGER_RELAY_URL)"/.test(l));
}

export function versionAtLeast(have, want) {
  const a = have.split('.').map(Number);
  const b = want.split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0);
  }
  return true;
}

export function releaseName(version, platform, arch) {
  const os_ = { linux: 'linux', darwin: 'macos', win32: 'windows' }[platform];
  const arch_ = { x64: 'x86_64', arm64: 'aarch64' }[arch];
  if (!os_ || !arch_) throw new Error(`unsupported platform ${platform}/${arch}`);
  return `p2p-messenger-${version}-${os_}-${arch_}${platform === 'win32' ? '.exe' : ''}`;
}

// ---- preflight ----

async function walk(dir) {
  const out = [];
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...(await walk(p)));
    else out.push(p);
  }
  return out;
}

async function manifestChecks() {
  const root = await readFile(path.join(repo, 'Cargo.toml'), 'utf8');
  const hits = devtoolsInManifests(root, await readFile(path.join(tauriDir, 'Cargo.toml'), 'utf8'));
  if (hits.length) throw new Error('tauri devtools feature is enabled in a manifest');
  for (const crate of ['protocol/src', 'core/src', 'app/src-tauri/src']) {
    for (const f of (await walk(path.join(repo, crate))).filter((p) => p.endsWith('.rs'))) {
      if (bakedEnvHits(await readFile(f, 'utf8')).length) {
        throw new Error(`${path.relative(repo, f)} bakes a runtime env var at compile time`);
      }
    }
  }
  return root;
}

function missing(what, hint) {
  return new Error(`missing prerequisite: ${what}. ${hint}`);
}

async function preflight() {
  if (!process.env.npm_execpath) throw new Error('run `npm run release` from the repository root');
  const root = await manifestChecks();
  const want = root.match(/^rust-version\s*=\s*"([\d.]+)"/m)?.[1];
  const have = (probe('rustc', ['--version']) ?? '').match(/rustc (\d+\.\d+\.\d+)/)?.[1];
  if (!have) throw missing('rustc', 'install Rust via https://rustup.rs');
  if (want && !versionAtLeast(have, want)) throw new Error(`rustc ${have} is older than rust-version ${want}`);
  if (!probe('cargo', ['--version'])) throw missing('cargo', 'install Rust via https://rustup.rs');

  const prereq = 'https://v2.tauri.app/start/prerequisites/';
  if (process.platform === 'linux') {
    if (probe('pkg-config', ['--exists', 'webkit2gtk-4.1', 'gtk+-3.0', 'libsoup-3.0']) === null) {
      throw missing('WebKitGTK 4.1 / GTK 3 / libsoup 3 development packages and pkg-config', prereq);
    }
    if (probe('nm', ['--version']) === null || probe('readelf', ['--version']) === null) {
      throw missing('binutils (nm, readelf)', prereq);
    }
  } else if (process.platform === 'darwin') {
    if (probe('xcode-select', ['-p']) === null) throw missing('Xcode command line tools', 'run xcode-select --install');
  } else if (isWin) {
    console.log(`release: on Windows you need the MSVC Build Tools and the WebView2 runtime (${prereq})`);
  }
}

// ---- build ----

function releaseEnv(target) {
  const env = { ...process.env };
  for (const k of Object.keys(env)) {
    if (/^CARGO_PROFILE_RELEASE_/i.test(k)) delete env[k];
  }
  for (const k of ['MESSENGER_PROFILE', 'MESSENGER_RELAY_URL', 'RUSTFLAGS', 'CARGO_BUILD_RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']) {
    delete env[k];
  }
  const cargoHome = process.env.CARGO_HOME || path.join(os.homedir(), '.cargo');
  const rustupHome = process.env.RUSTUP_HOME || path.join(os.homedir(), '.rustup');
  // rustc applies the last matching remap: most specific prefixes last
  env.CARGO_ENCODED_RUSTFLAGS = [
    [cargoHome, '/cargo'], [rustupHome, '/rustup'], [target, '/target'], [repo, '/src'],
  ].map(([from, to]) => `--remap-path-prefix=${from}=${to}`).join('\x1f');
  env.CARGO_TARGET_DIR = target;
  if ('MESSENGER_PROFILE' in env || 'MESSENGER_RELAY_URL' in env) throw new Error('runtime env leaked into build env');
  return env;
}

// ---- verify ----

async function verify(bin) {
  await manifestChecks();
  const bytes = await readFile(bin);
  console.log(`release: binary ${(bytes.length / 1e6).toFixed(1)} MB (expect about 19.5 MB)`);

  const forbidden = findForbidden(bytes, FORBIDDEN_NAMES);
  if (forbidden.length) throw new Error(`test-only code in binary: ${forbidden.join(', ')}`);

  const home = os.homedir();
  if (home === '/' || home === path.parse(home).root) console.log('release: home path check skipped (home is the filesystem root)');
  const paths = builderPathNeedles([home, repo]);
  const leaked = findForbidden(bytes, paths);
  if (leaked.length) throw new Error('builder paths embedded in binary (home or repo)');

  if (process.platform === 'linux') {
    const nm = probe('nm', ['-D', '--defined-only', bin]);
    if (nm === null) throw new Error('nm failed');
    if (exportedSqlite(nm).length) throw new Error('binary exports sqlite3_* symbols');
    const sections = probe('readelf', ['-S', '-W', bin]);
    if (sections === null) throw new Error('readelf failed');
    const names = [...sections.matchAll(/\]\s+(\.\S+)/g)].map((m) => m[1]);
    if (hasDebugSections(names)) throw new Error('binary is not stripped');
  } else if (process.platform === 'darwin') {
    const otool = probe('otool', ['-l', bin]);
    if (otool === null) throw new Error('otool failed');
    if (otool.includes('__DWARF')) throw new Error('binary is not stripped');
  } else if (isWin) {
    if (hasDebugSections(peSectionNames(bytes))) throw new Error('binary is not stripped');
  }

  const assets = await readdir(path.join(appDir, 'dist', 'assets'));
  if (!assets.length) throw new Error('no frontend assets found in app/dist/assets');
  const absent = assets.filter((a) => bytes.indexOf(a) === -1);
  if (absent.length) throw new Error(`frontend assets not embedded: ${absent.join(', ')}`);
  return bytes;
}

// ---- publish ----

const sha = (b) => createHash('sha256').update(b).digest('hex');

async function exists(p) {
  try {
    await lstat(p);
    return true;
  } catch {
    return false;
  }
}

async function publishAtomic(srcBytes, finalPath, mode, srcFile) {
  const partial = path.join(releaseDir, `.${path.basename(finalPath)}.partial-${randomBytes(6).toString('hex')}`);
  partials.push(partial);
  if (srcFile) await copyFile(srcFile, partial, fsc.COPYFILE_EXCL);
  else await writeFile(partial, srcBytes, { flag: 'wx' });
  if (mode !== undefined && !isWin) await chmod(partial, mode);
  if (sha(await readFile(partial)) !== sha(srcBytes)) throw new Error('copied file does not match the verified binary');
  await rename(partial, finalPath);
  partials.splice(partials.indexOf(partial), 1);
}

async function publish(bin, bytes) {
  checkInterrupted();
  const conf = JSON.parse(await readFile(path.join(tauriDir, 'tauri.conf.json'), 'utf8'));
  const name = releaseName(conf.version, process.platform, process.arch);
  const finalBin = path.join(releaseDir, name);
  const finalSum = `${finalBin}.sha256`;
  await mkdir(releaseDir, { recursive: true });
  for (const f of [finalBin, finalSum]) {
    if (await exists(f)) {
      throw new Error(`release/${path.basename(f)} already exists; move or remove it yourself. Nothing was published`);
    }
  }
  const hex = sha(bytes);
  await publishAtomic(bytes, finalBin, 0o755, bin);
  await publishAtomic(Buffer.from(`${hex}  ${name}\n`), finalSum);
  console.log(`release: ${path.relative(repo, finalBin)} (${(await stat(finalBin)).size} bytes)`);
  console.log(`release: sha256 ${hex}`);
}

// ---- cleanup ----

async function cleanup() {
  for (const p of partials) await rm(p, { force: true }).catch(() => {});
  if (tempTarget === null) return;
  try {
    const st = await lstat(tempTarget);
    if (!st.isDirectory() || st.isSymbolicLink()) throw new Error('not a real directory');
    if (path.dirname(tempTarget) !== (await realpath(os.tmpdir()))) throw new Error('not directly in the temp dir');
    if (!path.basename(tempTarget).startsWith('p2pm-release-')) throw new Error('unexpected name');
    const protectedDirs = [path.parse(tempTarget).root, os.homedir(), repo, releaseDir,
      path.join(repo, 'target'), path.join(appDir, 'node_modules')];
    if (protectedDirs.includes(tempTarget)) throw new Error('protected directory');
    await rm(tempTarget, { recursive: true, force: true, maxRetries: 3 });
  } catch (e) {
    console.error(`release: refusing to delete the temp dir (${e.message}); remove it yourself: ${tempTarget}`);
    process.exitCode = 1;
  }
}

async function main() {
  await preflight();
  await npm(['ci', '--no-audit', '--no-fund'], { cwd: appDir });
  tempTarget = await realpath(await mkdtemp(path.join(os.tmpdir(), 'p2pm-release-')));
  await tauri(['build', '--no-bundle', '--ci', '--', '--locked'], { cwd: appDir, env: releaseEnv(tempTarget) });
  const bin = path.join(tempTarget, 'release', isWin ? 'app.exe' : 'app');
  const bytes = await verify(bin);
  await publish(bin, bytes);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await main();
  } catch (e) {
    console.error(`release: ${e.message}`);
    process.exitCode = interrupted ? 130 : 1;
  } finally {
    await cleanup();
  }
}
