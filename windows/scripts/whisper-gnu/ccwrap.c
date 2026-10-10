/* ccwrap.c — a compiler shim for building whisper.cpp through whisper-rs-sys on this
 * machine. Why it exists: whisper-rs-sys' build.rs adds the MSVC-only flag `/utf-8`
 * whenever the host is Windows, and it does not check the target toolchain. Under the
 * GNU (MinGW-w64) toolchain this repo builds with — where there is no MSVC at all —
 * gcc reads `/utf-8` as a file name and the configure step dies with
 * "linker input file not found: No such file or directory".
 *
 * The shim forwards every argument to the real compiler, dropping only that flag, and
 * passes the exit code and the standard streams through unchanged, so CMake and Ninja
 * see a normal compiler.
 *
 * Build (two copies, one per language, so CMake's C and C++ paths both work):
 *   gcc -O2 -DREAL_COMPILER='"C:/path/to/gcc.exe"' -o oczi-cc.exe  ccwrap.c -lshell32
 *   gcc -O2 -DREAL_COMPILER='"C:/path/to/g++.exe"' -o oczi-cxx.exe ccwrap.c -lshell32
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <shellapi.h>
#include <wchar.h>

#ifndef REAL_COMPILER
#error "compile with -DREAL_COMPILER=\"path to the real gcc or g++\""
#endif

/* Six characters plus terminator; kept as a wide literal like the arguments we compare. */
static int is_utf8_flag(const wchar_t *arg) {
    return _wcsicmp(arg, L"/utf-8") == 0;
}

/* Windows quoting rule: wrap in quotes only when the argument has whitespace, but
 * escape embedded quotes ALWAYS. Ninja hands us CMake's `-DGGML_VERSION="0.9.5"` with
 * real quotes inside the argument; copying that verbatim makes the child's CRT parser
 * strip them, so the macro body becomes the bare token 0.9.5 and the compile dies with
 * "too many decimal points in number". Escaping as \\" makes the child see the same
 * argument, quotes included, that we received. */
static void append_quoted(wchar_t *dst, size_t cap, const wchar_t *arg) {
    size_t len = wcslen(dst);
    if (len + 4 >= cap) return;
    int need = (wcschr(arg, L' ') != NULL) || (wcschr(arg, L'\t') != NULL) || arg[0] == L'\0';
    if (need) {
        dst[len++] = L'"';
        dst[len] = L'\0';
    }
    for (const wchar_t *p = arg; *p; ++p) {
        if (len + 4 >= cap) break;
        if (*p == L'\\') {
            size_t run = 0;
            while (p[run] == L'\\') run++;
            int before_quote = (p[run] == L'"' || (need && p[run] == L'\0'));
            for (size_t i = 0; i < run * (before_quote ? 2 : 1); i++) dst[len++] = L'\\';
            p += run - 1;
        } else if (*p == L'"') {
            dst[len++] = L'\\';
            dst[len++] = L'"';
        } else {
            dst[len++] = *p;
        }
        dst[len] = L'\0';
    }
    if (need) {
        dst[len++] = L'"';
        dst[len] = L'\0';
    }
}

int main(void) {
    int argc = 0;
    LPWSTR *argv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (!argv) return 127;

    size_t cap = 32768;
    wchar_t *cmd = (wchar_t *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, cap * sizeof(wchar_t));
    if (!cmd) return 127;

    append_quoted(cmd, cap, L"" REAL_COMPILER);
    for (int i = 1; i < argc; i++) {
        if (is_utf8_flag(argv[i])) continue;
        wcscat(cmd, L" ");
        append_quoted(cmd, cap, argv[i]);
    }

    STARTUPINFOW si = {0};
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
    si.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
    si.hStdError = GetStdHandle(STD_ERROR_HANDLE);

    PROCESS_INFORMATION pi = {0};
    if (!CreateProcessW(NULL, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi)) {
        fwprintf(stderr, L"ccwrap: cannot start %hs (%lu)\n", REAL_COMPILER, GetLastError());
        return 127;
    }
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 127;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    LocalFree(argv);
    HeapFree(GetProcessHeap(), 0, cmd);
    return (int)code;
}
