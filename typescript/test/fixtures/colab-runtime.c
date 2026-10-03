/* A tiny native embedded-app stand-in. It proves verifier sensitivity, not Colab behavior. */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

static volatile sig_atomic_t stop = 0;
static void stopping(int signal_number) { (void)signal_number; stop = 1; }
static const char index_html[] = "<!doctype html><script src=\"./assets/app.js\"></script><link href=\"./assets/app.css\" rel=\"stylesheet\">tiny embedded app\n";
static const char js[] = "console.log('embedded fixture');\n";
static const char css[] = "body { color: blue; }\n";
static const char notices[] = "Tiny app attribution\n";

int main(int argc, char **argv) {
    if (argc == 2 && strcmp(argv[1], "--version") == 0) {
        puts("colab 0.1.0-alpha.1");
        return 0;
    }
    if (argc != 3 || strcmp(argv[1], "serve") || strcmp(argv[2], "--json")) return 2;
    const char *state = getenv("TMUX_TEAM_HOME");
    if (!state || !getenv("TMT_EXECUTABLE") || strcmp(getenv("PATH"), "" ) == 0) return 3;
    /* Relocation runs without ambient overrides or frontend tooling. */
    if (getenv("TMT_COLAB_APP_DIR") || getenv("GITHUB_TOKEN")) return 4;
    char directory[256];
    if (snprintf(directory, sizeof(directory), "%s/colab", state) >= (int)sizeof(directory)) return 5;
    if (mkdir(directory, 0700) && errno != EEXIST) return 6;
    struct sockaddr_un address;
    memset(&address, 0, sizeof(address));
    address.sun_family = AF_UNIX;
    if (snprintf(address.sun_path, sizeof(address.sun_path), "%s/door.sock", directory) >= (int)sizeof(address.sun_path)) return 7;
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = stopping;
    sigemptyset(&action.sa_mask);
    sigaction(SIGTERM, &action, NULL);
    signal(SIGPIPE, SIG_IGN);
    int listener = socket(AF_UNIX, SOCK_STREAM, 0);
    if (listener < 0 || bind(listener, (struct sockaddr *)&address, sizeof(address)) || listen(listener, 4)) return 8;
    chmod(address.sun_path, 0600);
#ifdef STARTUP_FAILURE
    fprintf(stderr, "COLAB_APP_UNAVAILABLE\n");
    return 9;
#endif
    printf("{\"socket\":\"%s\",\"state\":\"mounted\"}\n", address.sun_path);
    fflush(stdout);
    while (!stop) {
        int client = accept(listener, NULL, NULL);
        if (client < 0) { if (errno == EINTR) continue; return 10; }
        char request[8192] = {0};
        size_t used = 0;
        while (!strstr(request, "\r\n\r\n") && used < sizeof(request) - 1) {
            ssize_t length = read(client, request + used, sizeof(request) - 1 - used);
            if (length <= 0) break;
            used += (size_t)length;
        }
        char route[256] = {0};
        if (sscanf(request, "GET %255s", route) != 1) { close(client); continue; }
        const char *body = "NOT FOUND", *type = "text/plain";
        int status = 404;
        if (!strstr(request, "tmt-device-context:")) { status = 403; body = "DENIED"; }
        else if (!strcmp(route, "/") || !strcmp(route, "/index.html")) {
            status = 200; body = index_html; type = "text/html; charset=utf-8";
#ifdef PLACEHOLDER
            body = "<html>build the app</html>";
#endif
        } else if (!strcmp(route, "/assets/app.js")) {
            status = 200; body = js; type = "text/javascript; charset=utf-8";
#ifdef CORRUPT_ASSET
            body = "changed byte\n";
#endif
        } else if (!strcmp(route, "/assets/app.css")) {
            status = 200; body = css; type = "text/css; charset=utf-8";
        } else if (!strcmp(route, "/THIRD-PARTY-NOTICES.txt")) {
            status = 200; body = notices;
        }
        char header[512];
        int size = snprintf(header, sizeof(header), "HTTP/1.1 %d Response\r\nContent-Type: %s\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n", status, type, strlen(body));
        if (write(client, header, (size_t)size) >= 0) (void)write(client, body, strlen(body));
        close(client);
    }
    close(listener);
#ifndef LEAK_SOCKET
    unlink(address.sun_path);
#endif
    return 0;
}
