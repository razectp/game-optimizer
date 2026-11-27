/*
 * Wine helper for posting window messages without modifying desktop focus.
 * Rebuild on Linux with MinGW-w64:
 * x86_64-w64-mingw32-gcc -Os -s -fno-stack-protector -nostdlib -Wl,-e,entry,--no-insert-timestamp \
 *   assets/wine-macro-bridge.c -o assets/wine-macro-bridge.exe -luser32 -lkernel32
 */
#include <windows.h>

struct request {
    DWORD xid;
    DWORD message;
    DWORD wparam;
    DWORD lparam;
};

struct target {
    DWORD xid;
    HWND window;
};

static struct request held[128];

static BOOL CALLBACK find_window(HWND window, LPARAM arg)
{
    struct target *target = (struct target *)arg;
    if ((ULONG_PTR)GetPropW(window, L"__wine_x11_whole_window") != target->xid)
        return TRUE;
    target->window = window;
    return FALSE;
}

static BOOL post(const struct request *request)
{
    struct target target = { request->xid, NULL };
    LPARAM position = (LPARAM)(LONG)request->lparam;
    EnumWindows(find_window, (LPARAM)&target);
    if (!target.window)
        return FALSE;

    if (request->message == WM_LBUTTONDOWN || request->message == WM_LBUTTONUP ||
        request->message == WM_RBUTTONDOWN || request->message == WM_RBUTTONUP) {
        RECT rect;
        if (!GetClientRect(target.window, &rect))
            return FALSE;
        position = MAKELPARAM(rect.right / 2, rect.bottom / 2);
    }
    return PostMessageW(target.window, request->message, request->wparam, position);
}

static UINT release_message(UINT message)
{
    switch (message) {
    case WM_KEYDOWN: return WM_KEYUP;
    case WM_LBUTTONDOWN: return WM_LBUTTONUP;
    case WM_RBUTTONDOWN: return WM_RBUTTONUP;
    default: return 0;
    }
}

static void track(const struct request *request)
{
    UINT release = release_message(request->message);
    for (unsigned i = 0; i < ARRAYSIZE(held); i++) {
        if (release && !held[i].xid) {
            held[i] = *request;
            held[i].message = release;
            if (release == WM_KEYUP)
                held[i].lparam |= 0xc0000000;
            else
                held[i].wparam = 0;
            return;
        }
        if (!release && held[i].xid == request->xid &&
            held[i].message == request->message && held[i].wparam == request->wparam) {
            held[i].xid = 0;
            return;
        }
    }
}

static BOOL receive(HANDLE input, struct request *request)
{
    DWORD total = 0;
    while (total < sizeof(*request)) {
        DWORD count;
        if (!ReadFile(input, (char *)request + total, sizeof(*request) - total, &count, NULL) || !count)
            return FALSE;
        total += count;
    }
    return TRUE;
}

void entry(void)
{
    HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
    HANDLE output = GetStdHandle(STD_OUTPUT_HANDLE);
    struct request request;
    DWORD written;
    // Initialize the Wine window driver before notifying the parent.
    struct target probe = { 0xffffffff, NULL };
    EnumWindows(find_window, (LPARAM)&probe);
    if (WriteFile(output, "R", 1, &written, NULL)) {
        while (receive(input, &request)) {
            char result = post(&request) ? '1' : '0';
            if (result == '1')
                track(&request);
            if (!WriteFile(output, &result, 1, &written, NULL))
                break;
        }
    }
    for (unsigned i = 0; i < ARRAYSIZE(held); i++) {
        if (held[i].xid)
            post(&held[i]);
    }
    ExitProcess(0);
}
