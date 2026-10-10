// Builds the voice variant of Oczi, and only that variant.
//
// Why a script and not just `cargo build --features voice`: on Windows the GNU toolchain
// (the one this project builds with — no MSVC anywhere) gets four different things wrong in
// whisper-rs-sys' build script, and each one has to be corrected from outside the crate:
//
//   1. It adds the MSVC flag `/utf-8` on any Windows host, whatever the compiler. MinGW's
//      g++ reads it as a file name and CMake's compiler test dies. -> `bin/gcc.exe` and
//      `bin/g++.exe` are a shim (whisper-gnu/ccwrap.c) that forwards the arguments minus
//      that flag, and the toolchain file below hands them to CMake as the compilers.
//   2. It relies on bindgen, which needs libclang; its fallback (WHISPER_DONT_GENERATE_
//      BINDINGS) uses bindings generated against glibc and fails to compile on MinGW.
//      -> libclang is provisioned into target/whisper-gnu from the `libclang` wheel, with
//      the mingw include directories given to clang through BINDGEN_EXTRA_CLANG_ARGS.
//   3. It expects the CMake install prefix layout of an MSVC build. Under GNU the archives
//      land in <out>/lib while the crate searches <out>, and ggml strips the `lib` prefix
//      from its archives on any WIN32, which ld on MinGW refuses to match. -> the toolchain
//      file pins CMAKE_INSTALL_LIBDIR, and `fixArchives` renames what is left over between
//      two cargo runs (cargo re-runs the failed rustc but not the build script, so the
//      copies survive).
//   4. It links libstdc++ dynamically, so the exe would need libstdc++-6.dll on the user's
//      machine — where PATH resolves it to Git-for-Windows' incompatible copy and the app
//      dies at startup with STATUS_ENTRYPOINT_NOT_FOUND. -> static libstdc++/libgcc.
//
// Usage:
//   node scripts/whisper-gnu.mjs build     # cargo build the voice variant (release)
//   node scripts/whisper-gnu.mjs check     # cargo check, same env (fast iteration)
//   node scripts/whisper-gnu.mjs dev       # cargo build without --release
//   node scripts/whisper-gnu.mjs model     # download the model the installer ships
//
// Nothing here touches the ordinary build: without `--features voice` the crate is not even
// compiled, and `npm run pack` is untouched.

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, copyFileSync, readdirSync, writeFileSync, statSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));          // windows/scripts
const windowsDir = resolve(here, '..');                        // windows
const tauriDir = join(windowsDir, 'src-tauri');
// cargo puts its output at the workspace root, which is windows/ (windows/Cargo.toml), not
// under src-tauri/ — looking in the wrong tree is why the archive rename once did nothing.
const targetDir = existsSync(join(windowsDir, 'target'))
  ? join(windowsDir, 'target')
  : join(tauriDir, 'target');
const toolsDir = join(targetDir, 'whisper-gnu');               // disposable, git-ignored
const binDir = join(toolsDir, 'bin');
const shimSource = join(here, 'whisper-gnu', 'ccwrap.c');
const toolchainFile = join(toolsDir, 'toolchain.cmake');
const modelFile = join(windowsDir, 'models', 'ggml-small.bin');
const MODEL_URL = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin';

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options });
  if (result.error) throw new Error(`${command}: ${result.error.message}`);
  if (result.status !== 0 && !options.stdio) {
    // Loud on purpose: a silent helper failure here looks like "cargo refuses to build".
    process.stderr.write(
      `voice: ${command} ${args.join(' ')} exited ${result.status}\n${result.stdout ?? ''}${result.stderr ?? ''}`,
    );
  }
  return result;
}

function which(name) {
  const found = run('where.exe', [name]);
  if (found.status !== 0) return null;
  return found.stdout.split(/\r?\n/).find((line) => line.trim().length > 0) ?? null;
}

// The compilers this project already builds with. The WinLibs gcc that ships with the
// toolchain is the one we want; `where.exe` returns PATH order, and PATH here starts with
// Git-for-Windows' mingw64 — whose gcc is an older one that mis-links libstdc++ — so the
// WinLibs copy is preferred when it can be found.
function findCompiler(name) {
  const all = (run('where.exe', [name]).stdout ?? '')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  const winlibs = all.find((p) => /WinLibs/i.test(p));
  return winlibs ?? all[0] ?? null;
}

/** Compile the compiler shim that drops whisper-rs-sys' `/utf-8` flag. */
function buildShim(gcc, gxx) {
  mkdirSync(binDir, { recursive: true });
  for (const [target, real] of [['gcc.exe', gcc], ['g++.exe', gxx]]) {
    const dest = join(binDir, target);
    // Skip the rebuild when the binary is already newer than the source. Compared with
    // statSync rather than by shelling out to PowerShell: a child process with no console
    // attached inherits a stdin that is not a tty, and that is where this script's silent
    // "stdin is not a tty" death came from.
    if (existsSync(dest) && existsSync(shimSource)) {
      const built = statSync(dest).mtimeMs;
      const source = statSync(shimSource).mtimeMs;
      if (built >= source) continue;
    }
    const native = real.split(sep).join('/');
    const result = run(gcc, ['-O2', `-DREAL_COMPILER="${native}"`, '-o', dest, shimSource, '-lshell32'], {
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    process.stdout.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
    if (result.status !== 0) throw new Error(`ccwrap for ${target} failed to build`);
  }
}

/** The CMake toolchain: our shims as the compilers, and the install libdir the crate expects. */
function writeToolchain() {
  const body = `# Generated by scripts/whisper-gnu.mjs — do not edit, and do not commit.
# See that file for why each line is here: the compilers are shims that strip the MSVC-only
# /utf-8 flag whisper-rs-sys passes on Windows, and the install libdir keeps the archives
# where whisper-rs-sys looks for them (it assumes an MSVC layout).
set(CMAKE_C_COMPILER   "${join(binDir, 'gcc.exe').split(sep).join('/')}"  CACHE FILEPATH "" FORCE)
set(CMAKE_CXX_COMPILER "${join(binDir, 'g++.exe').split(sep).join('/')}" CACHE FILEPATH "" FORCE)
set(CMAKE_INSTALL_LIBDIR "." CACHE PATH "" FORCE)
`;
  writeFileSync(toolchainFile, body);
}

/** libclang, needed by bindgen. A wheel is enough and needs no admin rights. */
function libclangDir() {
  if (process.env.LIBCLANG_PATH && existsSync(join(process.env.LIBCLANG_PATH, 'libclang.dll'))) {
    return process.env.LIBCLANG_PATH;
  }
  const venv = join(toolsDir, 'libclang-venv');
  const native = join(venv, 'Lib', 'site-packages', 'clang', 'native');
  if (existsSync(join(native, 'libclang.dll'))) return native;

  console.log('voice: provisioning libclang into target/whisper-gnu (bindgen needs it)…');
  const uv = which('uv.exe') ?? which('uv');
  // Piped rather than inherited: a spawned child here has no console attached, and an
  // inherited non-tty stdin is what made uv/uv pip abort with "stdin is not a tty".
  const piped = { stdio: ['ignore', 'pipe', 'pipe'] };
  const echo = (result) => {
    process.stdout.write(result.stdout ?? '');
    process.stderr.write(result.stderr ?? '');
    return result;
  };
  const created = uv
    ? echo(run(uv, ['venv', venv], piped))
    : echo(run('python', ['-m', 'venv', venv], piped));
  if (created.status !== 0) {
    throw new Error('cannot create a venv for libclang; install uv or python and rerun');
  }
  const installer = uv
    ? echo(run(uv, ['pip', 'install', '--python', join(venv, 'Scripts', 'python.exe'), 'libclang'], piped))
    : echo(run(join(venv, 'Scripts', 'python.exe'), ['-m', 'pip', 'install', 'libclang'], piped));
  if (installer.status !== 0 || !existsSync(join(native, 'libclang.dll'))) {
    throw new Error('libclang wheel did not land; set LIBCLANG_PATH to a directory holding libclang.dll');
  }
  return native;
}

/** The include directories clang needs to parse whisper.h against MinGW headers. */
function bindgenArgs(gcc) {
  const builtin = run(gcc, ['-print-file-name=include']).stdout.trim();
  const mingw = resolve(dirname(gcc), '..');
  const dirs = [builtin, join(mingw, 'x86_64-w64-mingw32', 'include'), join(mingw, 'include')]
    .filter((dir) => dir && existsSync(dir))
    .map((dir) => `-I${dir.split(sep).join('/')}`);
  return ['--target=x86_64-w64-windows-gnu', ...dirs].join(' ');
}

function voiceEnv(gcc) {
  return {
    ...process.env,
    PATH: `${binDir};${process.env.PATH}`,
    CC: join(binDir, 'gcc.exe'),
    CXX: join(binDir, 'g++.exe'),
    CMAKE_GENERATOR: 'Ninja',
    CMAKE_TOOLCHAIN_FILE: toolchainFile,
    LIBCLANG_PATH: libclangDir(),
    BINDGEN_EXTRA_CLANG_ARGS: bindgenArgs(gcc),
    // The static C++ runtime flags are in windows/.cargo/config.toml rather than in RUSTFLAGS
    // here: cargo takes the FIRST of RUSTFLAGS / target.*.rustflags it finds, so setting the
    // variable would silently drop the target's own flags (the export-ordinal fix this
    // project needs for its cdylib).
    WHISPER_DONT_GENERATE_BINDINGS: undefined,
  };
}

/** ggml names its archives without the `lib` prefix on Windows; ld on MinGW wants it. */
function fixArchives() {
  let fixed = 0;
  // A plain depth-capped walk from target/: the sys crate's directory sits at
  // target/<profile>/build/whisper-rs-sys-<hash>/, and an earlier version of this only
  // descended into directories named after a profile, which is why it fixed nothing.
  const walk = (dir, depth) => {
    if (depth > 6) return;
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      const full = join(dir, entry.name);
      if (entry.name.startsWith('whisper-rs-sys-')) {
        // The crate links from the install prefix and, failing that, from the prefix's lib
        // dir; both are searched here so the rename lands wherever the linker looks.
        for (const sub of [full, join(full, 'out'), join(full, 'out', 'lib')]) {
          if (!existsSync(sub)) continue;
          for (const file of readdirSync(sub)) {
            if (!file.endsWith('.a') || file.startsWith('lib')) continue;
            const copy = join(sub, `lib${file}`);
            if (!existsSync(copy)) {
              copyFileSync(join(sub, file), copy);
              fixed += 1;
            }
          }
        }
        continue;
      }
      if (entry.name === 'deps' || entry.name === 'incremental') continue;
      walk(full, depth + 1);
    }
  };
  // Both possible roots: the workspace target dir, and the crate-local one if this project
  // ever moves its workspace file.
  for (const root of [targetDir, join(tauriDir, 'target')]) {
    if (existsSync(root)) walk(root, 0);
  }
  return fixed;
}

function cargo(args, env) {
  const result = run('cargo', args, { cwd: tauriDir, env, encoding: 'utf8' });
  process.stdout.write(result.stdout ?? '');
  process.stderr.write(result.stderr ?? '');
  return result;
}

function build(cargoArgs) {
  const gcc = findCompiler('gcc.exe') ?? findCompiler('gcc');
  const gxx = findCompiler('g++.exe') ?? findCompiler('g++');
  if (!gcc || !gxx) throw new Error('no gcc/g++ on PATH — the GNU toolchain this project uses is missing');
  mkdirSync(toolsDir, { recursive: true });

  buildShim(gcc, gxx);
  writeToolchain();
  const env = voiceEnv(gcc);

  const args = [...cargoArgs, '--features', 'voice'];
  let result = cargo(args, env);
  if (result.status !== 0 && /could not find native static library/.test(result.stderr ?? '')) {
    // Expected on a build that has not linked whisper.cpp before: the archives exist by now,
    // they are just named the way MSVC would name them.
    const fixed = fixArchives();
    console.log(`voice: renamed ${fixed} archive(s) for MinGW and linking again`);
    result = cargo(args, env);
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function downloadModel() {
  if (existsSync(modelFile)) {
    console.log(`voice: model already at ${modelFile}`);
    return;
  }
  mkdirSync(dirname(modelFile), { recursive: true });
  console.log(`voice: downloading ${MODEL_URL} — 466 MB, once`);
  const curl = which('curl.exe') ?? which('curl');
  if (!curl) throw new Error('curl not found');
  const result = run(curl, ['-L', '--fail', '--retry', '2', '-o', modelFile, MODEL_URL], {
    stdio: 'inherit',
  });
  if (result.status !== 0) throw new Error('model download failed');
  console.log(`voice: model ready at ${modelFile}`);
}

const action = process.argv[2] ?? 'build';
switch (action) {
  case 'build':
    build(['build', '--release']);
    break;
  case 'dev':
    build(['build']);
    break;
  case 'check':
    build(['check', '--all-targets']);
    break;
  case 'model':
    downloadModel();
    break;
  case 'env':
    // Writes the environment for a plain `cargo` run, as shell exports:
    //   node scripts/whisper-gnu.mjs env > /tmp/voice-env.sh && . /tmp/voice-env.sh
    //   cargo build --release --features voice
    // The long build is then owned by cargo, not by this script — which matters when the
    // script has to run somewhere that cannot start node (a background job with no tty).
    {
      const gcc = findCompiler('gcc.exe') ?? findCompiler('gcc');
      const gxx = findCompiler('g++.exe') ?? findCompiler('g++');
      if (!gcc || !gxx) throw new Error('no gcc/g++ on PATH — the GNU toolchain this project uses is missing');
      mkdirSync(toolsDir, { recursive: true });
      buildShim(gcc, gxx);
      writeToolchain();
      const env = voiceEnv(gcc);
      // PATH goes out in MSYS form with the shim directory prepended: a Windows PATH
      // written as `C:\a;D:\b` is one unusable entry to bash, and the long build is driven
      // from a shell. The rest are native values for a native cargo.
      const shimUnix = '/' + binDir.replace(/^([A-Za-z]):\\/, (_m, drive) => `${drive.toLowerCase()}/`).split(sep).join('/');
      process.stdout.write(`export PATH='${shimUnix}':"$PATH"\n`);
      for (const key of [
        'CC',
        'CXX',
        'CMAKE_GENERATOR',
        'CMAKE_TOOLCHAIN_FILE',
        'LIBCLANG_PATH',
        'BINDGEN_EXTRA_CLANG_ARGS',
      ]) {
        process.stdout.write(`export ${key}='${String(env[key]).replace(/'/g, "'\\''")}'\n`);
      }
      process.stdout.write(
        `# archives to rename after a first failed link, if any: ${fixArchives() ? 'already done' : 'none needed yet'}\n`,
      );
    }
    break;
  case 'stage':
    // Everything the voice installer carries beyond the ordinary one: the speech model
    // and the three MinGW runtime DLLs the exe imports. Both live outside the repository
    // (466 MB of model, binaries from the toolchain), which is why this copies them into
    // place rather than committing them.
    {
      const model = join(windowsDir, 'models', 'ggml-small.bin');
      mkdirSync(dirname(model), { recursive: true });
      mkdirSync(join(windowsDir, 'runtime'), { recursive: true });
      downloadModel();
      const gcc = findCompiler('gcc.exe') ?? findCompiler('gcc');
      if (!gcc) throw new Error('no gcc on PATH — cannot find the runtime DLLs to ship');
      const bin = dirname(gcc);
      // libstdc++ pulls the other two, and the exe links libstdc++ dynamically: whisper-rs-sys
      // asks for it with an explicit -lstdc++, which no static flag removes. Shipping them
      // beside the exe is what makes the installer work on a machine that has no MinGW.
      for (const dll of ['libstdc++-6.dll', 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll']) {
        const from = join(bin, dll);
        if (!existsSync(from)) throw new Error(`${dll} missing next to ${gcc}`);
        copyFileSync(from, join(windowsDir, 'runtime', dll));
      }
      console.log('voice: model and runtime DLLs staged');
    }
    break;
  default:
    console.error(`unknown action ${action}`);
    process.exit(2);
}
