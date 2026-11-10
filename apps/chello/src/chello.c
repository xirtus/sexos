// SPDX-License-Identifier: GPL-3.0-only
// An ordinary C program: nothing here knows it runs on SexOS. It checks the parts of
// the C library a portable program leans on and prints what it found. The last line
// is the score the program gate reads.
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/time.h>

static int passed, total;
static void check(const char *name, int ok) {
    total++;
    if (ok) passed++;
    printf("%s %s\n", ok ? "ok  " : "FAIL", name);
}

static int cmp(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }
static void bye(void) { fprintf(stderr, "[chello.atexit]\n"); }

int main(int argc, char **argv) {
    printf("Hello from C on SexOS.\n");
    printf("arguments:");
    for (int i = 1; i < argc; i++) printf(" [%s]", argv[i]);
    printf("\n");
    atexit(bye);

    char text[64];
    snprintf(text, sizeof text, "%d %5.2f %s %x", 42, 3.14159, "pi", 255);
    check("snprintf", strcmp(text, "42  3.14 pi ff") == 0);

    FILE *f = fopen("chello.txt", "w");
    check("fopen for writing", f != NULL);
    for (int i = 0; i < 1000; i++) fprintf(f, "line %d\n", i);
    check("fclose", fclose(f) == 0);

    f = fopen("chello.txt", "r");
    char line[64];
    int lines = 0, last = -1;
    while (fgets(line, sizeof line, f)) { lines++; last = atoi(line + 5); }
    check("fgets every line back", lines == 1000 && last == 999);
    fseek(f, -9, SEEK_END);
    check("fseek and ftell", ftell(f) == 8881 && fgets(line, sizeof line, f) && strcmp(line, "line 999\n") == 0);
    check("feof", fgetc(f) == EOF && feof(f));
    fclose(f);

    struct stat st;
    check("stat", stat("chello.txt", &st) == 0 && st.st_size == 8890 && S_ISREG(st.st_mode));

    f = fopen("chello.txt", "a");
    fputs("tail\n", f);
    fclose(f);
    int fd = open("chello.txt", O_RDONLY);
    char tail[5] = {0};
    check("open, lseek, read", fd >= 0 && lseek(fd, -5, SEEK_END) == 8890 && read(fd, tail, 4) == 4 && memcmp(tail, "tail", 4) == 0);
    close(fd);

    mkdir("chello-dir", 0755);
    check("rename", rename("chello.txt", "chello-dir/moved.txt") == 0 && access("chello.txt", F_OK) != 0);
    DIR *d = opendir("chello-dir");
    int found = 0;
    struct dirent *e;
    while (d && (e = readdir(d))) if (strcmp(e->d_name, "moved.txt") == 0) found = 1;
    if (d) closedir(d);
    check("opendir and readdir", found);
    check("remove", remove("chello-dir/moved.txt") == 0 && rmdir("chello-dir") == 0);
    check("errno for a missing file", fopen("no-such-file", "r") == NULL && errno == ENOENT);
    check("never above its folder", fopen("../../x", "r") == NULL);

    int nums[100];
    for (int i = 0; i < 100; i++) nums[i] = (i * 37) % 101;
    qsort(nums, 100, sizeof nums[0], cmp);
    int sorted = 1;
    for (int i = 1; i < 100; i++) sorted &= nums[i - 1] <= nums[i];
    check("qsort", sorted);

    char *big = malloc(1 << 20);
    memset(big, 7, 1 << 20);
    big = realloc(big, 2 << 20);
    check("malloc and realloc", big && big[(1 << 20) - 1] == 7);
    free(big);
    check("strtol", strtol("-0x1F", NULL, 16) == -31);
    check("time", time(NULL) > 1767225600);

    /* Sockets: what works without the network, then, in a run given it
       (`run +net chello net HOST PORT`), a TCP and a UDP echo. */
    struct in_addr a4; struct in6_addr a6; char back[INET6_ADDRSTRLEN];
    check("inet_pton and inet_ntop",
          inet_pton(AF_INET, "10.0.2.2", &a4) == 1 && strcmp(inet_ntop(AF_INET, &a4, back, sizeof back), "10.0.2.2") == 0 &&
          inet_pton(AF_INET6, "fd00:0:0::1", &a6) == 1 && strcmp(inet_ntop(AF_INET6, &a6, back, sizeof back), "fd00::1") == 0 &&
          htons(0x1234) == 0x3412);
    struct addrinfo hints = {0}, *res = NULL;
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
    int gai = getaddrinfo("10.0.2.2", "7", &hints, &res);
    check("getaddrinfo for an address", gai == 0 && res && res->ai_family == AF_INET &&
          ntohs(((struct sockaddr_in *)res->ai_addr)->sin_port) == 7);
    struct timeval tv;
    check("gettimeofday", gettimeofday(&tv, NULL) == 0 && tv.tv_sec > 1767225600);
    int net = argc >= 4 && strcmp(argv[1], "net") == 0;
    if (!net) {
        int s = socket(AF_INET, SOCK_STREAM, 0);
        check("no network without +net", s >= 0 && res && connect(s, res->ai_addr, res->ai_addrlen) == -1 && errno == EACCES);
        close(s);
    } else {
        struct addrinfo *peer = NULL;
        int ok = getaddrinfo(argv[2], argv[3], &hints, &peer) == 0;
        int s = socket(AF_INET, SOCK_STREAM, 0);
        ok = ok && s >= 0 && connect(s, peer->ai_addr, peer->ai_addrlen) == 0;
        const char *msg = "hello from C\n";
        char got[64] = {0};
        size_t have = 0;
        ok = ok && send(s, msg, strlen(msg), 0) == (ssize_t)strlen(msg);
        while (ok && have < strlen(msg)) {
            ssize_t n = recv(s, got + have, sizeof got - 1 - have, 0);
            if (n <= 0) { ok = 0; break; }
            have += n;
        }
        check("C TCP echo", ok && strcmp(got, msg) == 0);
        struct pollfd pf = { s, POLLOUT, 0 };
        check("poll", poll(&pf, 1, 1000) == 1 && (pf.revents & POLLOUT));
        close(s);
        int u = socket(AF_INET, SOCK_DGRAM, 0);
        struct timeval wait = { 5, 0 };
        setsockopt(u, SOL_SOCKET, SO_RCVTIMEO, &wait, sizeof wait);
        char dgram[32] = {0};
        struct sockaddr_in from; socklen_t fromlen = sizeof from;
        ssize_t n = -1;
        for (int i = 0; i < 5 && n < 0; i++) {
            sendto(u, "udp from C", 10, 0, peer->ai_addr, peer->ai_addrlen);
            n = recvfrom(u, dgram, sizeof dgram - 1, 0, (struct sockaddr *)&from, &fromlen);
        }
        check("C UDP echo", n == 10 && strcmp(dgram, "udp from C") == 0 && from.sin_family == AF_INET);
        close(u);
        freeaddrinfo(peer);
    }
    freeaddrinfo(res);

    printf("passed %d/%d\n", passed, total);
    fprintf(stderr, "[chello.%s] passed=%d/%d\n", passed == total ? "ok" : "fail", passed, total);
    return passed == total ? 0 : 1;
}
