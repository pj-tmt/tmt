#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Linux-only E2E interposer, loaded into the owned native launcher. SQLite's
 * WAL close upgrades SHARED to EXCLUSIVE with F_WRLCK over SHARED_FIRST/size
 * (1073741826/510). Gate only after spawn: earlier storage transactions must
 * finish normally. Two FIFOs expose before-acquire and held-lock boundaries
 * without sleeping or changing SQLite's actual lock operation. */
static int used;

static int has_child(void) {
  char file[128];
  snprintf(file, sizeof file, "/proc/self/task/%ld/children", (long)getpid());
  FILE *stream = fopen(file, "r");
  long child = 0;
  int found = stream && fscanf(stream, "%ld", &child) == 1 && child > 0;
  if (stream) fclose(stream);
  return found;
}

static void marker(const char *name) {
  int fd = open(getenv(name), O_WRONLY | O_CREAT | O_TRUNC, 0600);
  if (fd < 0 || write(fd, "ready", 5) != 5 || close(fd)) _exit(91);
}

static void gate(const char *name) {
  int fd = open(getenv(name), O_RDONLY);
  if (fd < 0) _exit(92);
  char value;
  ssize_t count;
  do { count = read(fd, &value, 1); } while (count < 0 && errno == EINTR);
  if (count != 1 || close(fd)) _exit(92);
}

static int invoke(const char *symbol, int fd, int command, va_list args) {
  int (*original)(int, int, ...) = dlsym(RTLD_NEXT, symbol);
  if (!original) _exit(90);
  switch (command) {
    case F_GETFD: case F_GETFL: case F_GETOWN: case F_GETSIG:
    case F_GETLEASE: case F_GETPIPE_SZ:
      return original(fd, command);
    case F_SETFD: case F_SETFL: case F_SETOWN: case F_SETSIG:
    case F_SETLEASE: case F_SETPIPE_SZ: case F_DUPFD: case F_DUPFD_CLOEXEC:
      return original(fd, command, va_arg(args, int));
  }
  void *argument = va_arg(args, void *);
  const char *database = getenv("TMT_CLOSE_DATABASE");
  if (!used && database && command == F_SETLK) {
    struct flock *lock = argument;
    char descriptor[64], actual[4096];
    snprintf(descriptor, sizeof descriptor, "/proc/self/fd/%d", fd);
    ssize_t length = readlink(descriptor, actual, sizeof actual - 1);
    if (length >= 0) {
      actual[length] = 0;
      if (!strcmp(actual, database) && lock->l_type == F_WRLCK &&
          lock->l_start == 1073741826 && lock->l_len == 510 && has_child()) {
        used = 1;
        marker("TMT_CLOSE_BEFORE");
        gate("TMT_CLOSE_ACQUIRE");
        int result = original(fd, command, argument);
        if (result) _exit(93);
        marker("TMT_CLOSE_HELD");
        gate("TMT_CLOSE_RELEASE");
        return result;
      }
    }
  }
  return original(fd, command, argument);
}

int fcntl(int fd, int command, ...) {
  va_list args;
  va_start(args, command);
  int result = invoke("fcntl", fd, command, args);
  va_end(args);
  return result;
}

int fcntl64(int fd, int command, ...) {
  va_list args;
  va_start(args, command);
  int result = invoke("fcntl64", fd, command, args);
  va_end(args);
  return result;
}
