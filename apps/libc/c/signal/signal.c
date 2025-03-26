#include <signal.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>

void test_term() {
  puts("test_term");
  if (fork() == 0) {
    kill(0, SIGTERM);
    puts("This should not be printed");
  }
  wait(0);
  puts("Done");
}

static void signal_handler(int signum) {
  static int count = 0;
  count++;
  printf("Received signal %d, count=%d\n", signum, count);
  if (count > 1) {
    return;
  }
  // This should be blocked and won't cause recursion
  kill(0, SIGTERM);
  printf("End, count=%d\n", count);
}

void test_sigaction() {
  puts("test_sigaction");
  struct sigaction sa = {0};
  sa.sa_handler = signal_handler;
  sa.sa_flags = 0;
  sigaction(SIGTERM, &sa, NULL);
  kill(0, SIGTERM);
  puts("Ok1");

  sa.sa_handler = (void (*)(int))1;
  sigaction(SIGTERM, &sa, NULL);
  kill(0, SIGTERM);
  puts("Ok2");

  sa.sa_handler = (void (*)(int))0;
  sigaction(SIGTERM, &sa, NULL);
}

void test_sigprocmask() {
  puts("test_sigprocmask");
  sigset_t set, set2;
  sigemptyset(&set);
  sigaddset(&set, SIGTERM);
  sigprocmask(SIG_BLOCK, &set, NULL);
  kill(0, SIGTERM);

  sigpending(&set2);
  if (sigismember(&set2, SIGTERM)) {
    puts("Ok1");
  }

  // Ignore SIGTERM for once
  struct sigaction sa = {0};
  sa.sa_handler = (void (*)(int))1;
  sa.sa_flags = 0;
  sigaction(SIGTERM, &sa, NULL);

  sigdelset(&set, SIGTERM);
  sigprocmask(SIG_SETMASK, &set, NULL);

  sigpending(&set2);
  if (!sigismember(&set2, SIGTERM)) {
    puts("Ok2");
  }

  sa.sa_handler = (void (*)(int))0;
  sigaction(SIGTERM, &sa, NULL);
}

void test_sigkill_stop() {
  puts("test_sigkill_stop");
  struct sigaction sa = {0};
  sa.sa_handler = signal_handler;
  sa.sa_flags = 0;
  if (sigaction(SIGKILL, &sa, NULL) == 0) {
    puts("Wrong SIGKILL");
  }
  if (sigaction(SIGSTOP, &sa, NULL) == 0) {
    puts("Wrong SIGSTOP");
  }
  puts("Done");
}

int main() {
  test_term();
  test_sigaction();
  test_sigprocmask();
  test_sigkill_stop();
  return 0;
}