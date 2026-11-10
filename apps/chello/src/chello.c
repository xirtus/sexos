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

static int passed, total;
static void check(const char *name, int ok) {
    total++;
    if (ok) passed++;
    printf("%s %s\n", ok ? "ok  " : "FAIL", name);
}

static int cmp(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }
static int at_exit_ran;
static void bye(void) { at_exit_ran = 1; fprintf(stderr, "[chello.atexit]\n"); }

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

    printf("passed %d/%d\n", passed, total);
    fprintf(stderr, "[chello.%s] passed=%d/%d\n", passed == total ? "ok" : "fail", passed, total);
    return passed == total ? 0 : 1;
}
